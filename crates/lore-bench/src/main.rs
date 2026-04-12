//! Retrieval quality benchmark for the Lore search pipeline.
//!
//! Builds a 20-document synthetic corpus, runs 20 natural-language queries
//! through the full FTS5 + vector → RRF → MMR pipeline, and reports
//! MRR@10, Hit@1, Hit@3, and Hit@10.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p lore-bench --release
//! ```
//!
//! The embedding model (~130 MB) is downloaded on first run and cached.
//! Subsequent runs reuse the cache and complete in under 30 seconds.

#![warn(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    missing_docs,
    rust_2018_idioms
)]
#![allow(clippy::missing_errors_doc)]

mod corpus;
mod mrr;

use lore_build::PackageBuilder;
use lore_core::{Db, LoreError, Package, SearchConfig};
use lore_search::search;
use tempfile::tempdir;

const MRR_CUTOFF: usize = 10;

#[tokio::main]
async fn main() -> Result<(), LoreError> {
    tracing_subscriber::fmt()
        .with_env_filter("warn")
        .init();

    let cache_dir = lore_mcp::model_cache_dir();

    println!("Lore Retrieval Quality Benchmark");
    println!("=================================");
    println!("Corpus : {} documents", corpus::DOCS.len());
    println!("Queries: {}", corpus::QUERIES.len());
    println!("Cutoff : MRR@{MRR_CUTOFF}");
    println!();

    // ── Build corpus ──────────────────────────────────────────────────────────

    println!("Building corpus (may download embedding model on first run)…");

    let work_dir = tempdir().map_err(LoreError::Io)?;
    let src_path = work_dir.path().join("src");
    let db_path  = work_dir.path().join("bench.db");
    std::fs::create_dir(&src_path).map_err(LoreError::Io)?;

    for (filename, content) in corpus::DOCS {
        std::fs::write(src_path.join(filename), content).map_err(LoreError::Io)?;
    }

    let package = Package {
        name:        "bench-corpus".to_owned(),
        registry:    "bench".to_owned(),
        version:     "1.0.0".to_owned(),
        description: None,
        source_url:  None,
        git_sha:     None,
    };

    // PackageBuilder::new loads the embedding model; we reuse its Embedder for
    // query embedding so the model is only loaded once.
    let builder =
        tokio::task::spawn_blocking(move || PackageBuilder::new(&cache_dir))
            .await
            .map_err(|e| LoreError::InvalidConfig(e.to_string()))??;

    let stats = builder.build(&src_path, package, &db_path, false).await?;
    println!("  {}\n", stats.summary());

    // ── Prepare search resources ──────────────────────────────────────────────

    let db = Db::open(&db_path).await?;

    // Reuse the embedder from the builder — no second model load required.
    let embedder = builder.embedder().clone();

    let config = SearchConfig {
        candidate_limit:     20,
        // Permissive threshold: keeps weak matches visible so rank can be measured.
        relevance_threshold: 0.05,
        // No token budget — we rank by position, not total tokens.
        token_budget:        u32::MAX,
        mmr_lambda:          0.7,
    };

    // ── Embed all queries in one batch ────────────────────────────────────────

    let query_strings: Vec<String> = corpus::QUERIES
        .iter()
        .map(|(q, _)| (*q).to_owned())
        .collect();

    let embeddings = tokio::task::spawn_blocking({
        let embedder = embedder.clone();
        move || embedder.embed_batch(&query_strings)
    })
    .await
    .map_err(|e| LoreError::InvalidConfig(e.to_string()))??;

    // ── Run queries ───────────────────────────────────────────────────────────

    println!("{:<52} {:>4}  {:>6}", "Query", "Rank", "  RR");
    println!("{}", "─".repeat(66));

    let mut reciprocal_ranks = Vec::with_capacity(corpus::QUERIES.len());

    for ((query, expected_doc), embedding) in corpus::QUERIES.iter().zip(&embeddings) {
        let results = search(&db, query, embedding, &config).await?;
        let doc_paths: Vec<&str> = results.iter().map(|r| r.doc_path.as_str()).collect();
        let rr = mrr::reciprocal_rank(&doc_paths, expected_doc, MRR_CUTOFF);
        reciprocal_ranks.push(rr);

        let rank_display = rank_label(rr);
        let q = truncate(query, 50);
        println!("{q:<52} {rank_display:>4}  {rr:>6.4}");
    }

    // ── Report ────────────────────────────────────────────────────────────────

    println!("{}", "─".repeat(66));

    let mrr_score = mrr::compute(&reciprocal_ranks);
    let n = reciprocal_ranks.len();
    let count_at = |min_rr: f64| reciprocal_ranks.iter().filter(|&&r| r >= min_rr).count();
    let hit1  = count_at(1.0);
    let hit3  = count_at(1.0 / 3.0);
    #[allow(clippy::cast_precision_loss)]
    let hit10 = count_at(1.0 / MRR_CUTOFF as f64);

    println!();
    println!("MRR@{MRR_CUTOFF:<2} : {mrr_score:.4}");
    println!("Hit@1  : {hit1:>2}/{n}  ({:.1}%)", pct(hit1, n));
    println!("Hit@3  : {hit3:>2}/{n}  ({:.1}%)", pct(hit3, n));
    println!("Hit@10 : {hit10:>2}/{n}  ({:.1}%)", pct(hit10, n));

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Format a reciprocal rank as a 1-indexed rank string, or "—" if zero.
fn rank_label(rr: f64) -> String {
    if rr == 0.0 {
        "—".to_owned()
    } else {
        // rr is always 1/k for an integer k in [1, cutoff], so the cast is safe.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rank = (1.0_f64 / rr).round() as usize;
        rank.to_string()
    }
}

/// Truncate `s` to at most `max_chars` Unicode scalar values.
fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Percentage helper.
#[allow(clippy::cast_precision_loss)]
fn pct(n: usize, total: usize) -> f64 {
    if total == 0 { 0.0 } else { 100.0 * n as f64 / total as f64 }
}
