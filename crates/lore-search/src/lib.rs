#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Search pipeline: FTS5 + vector → RRF → MMR → token budget.
//!
//! The entry point is [`search`].  The pipeline:
//!
//! 1. **FTS5 BM25** — keyword candidates via `SQLite` full-text search.
//! 2. **Vector KNN** — semantic candidates via cosine similarity.
//! 3. **RRF fusion** ([`rrf::merge`]) — merge the two ranked lists into one score.
//! 4. **Relevance threshold** — drop results below a fraction of the top score.
//! 5. **MMR diversity** ([`mmr`]) — greedily select a diverse result set.
//! 6. **Token budget** ([`budget`]) — stop once total tokens would be exceeded.
//! 7. **Resolve** ([`resolve`]) — attach doc titles and heading breadcrumbs.

mod budget;
mod mmr;
mod resolve;
mod rrf;

pub use lore_core::{ScoredNode, SearchConfig, SearchResult};

use std::collections::HashMap;

use lore_core::{Db, LoreError};
use tracing::instrument;

/// Executes the full search pipeline against `db`.
///
/// Both `query` (keyword) and `query_embedding` (semantic) are used; passing
/// an empty `query` disables FTS5 and uses only vector search.
///
/// # Errors
///
/// Returns [`LoreError`] if any database operation fails.
#[instrument(skip(db, query_embedding, config), fields(query = %query))]
pub async fn search(
    db: &Db,
    query: &str,
    query_embedding: &[f32],
    config: &SearchConfig,
) -> Result<Vec<SearchResult>, LoreError> {
    let limit = config.candidate_limit;

    // FTS5 and vector search run sequentially — the underlying connection is
    // single-threaded, so concurrent dispatch would not improve throughput.
    let fts_hits = db.fts_search(sanitize_fts_query(query), limit).await?;
    let vec_hits = db.vec_search(query_embedding.to_vec(), limit).await?;

    finish(db, vec![fts_hits, vec_hits], config).await
}

/// Keyword-only search: FTS5 BM25 without the semantic vector stage.
///
/// This skips query embedding entirely, so callers avoid loading the ~130 MB
/// ONNX model — turning a ~300 ms `lore search` into a single-digit-millisecond
/// lookup. MMR diversity still applies, using the chunk embeddings already
/// stored in the database.
///
/// # Errors
///
/// Returns [`LoreError`] if any database operation fails.
#[instrument(skip(db, config), fields(query = %query))]
pub async fn search_keyword(
    db: &Db,
    query: &str,
    config: &SearchConfig,
) -> Result<Vec<SearchResult>, LoreError> {
    let fts_hits = db.fts_search(sanitize_fts_query(query), config.candidate_limit).await?;
    finish(db, vec![fts_hits], config).await
}

/// Shared tail of the search pipeline: RRF-fuse the candidate lists, apply the
/// relevance threshold, diversify with MMR, enforce the token budget, and
/// resolve doc titles + heading breadcrumbs.
async fn finish(
    db: &Db,
    lists: Vec<Vec<ScoredNode>>,
    config: &SearchConfig,
) -> Result<Vec<SearchResult>, LoreError> {
    let merged = rrf::merge(lists);
    if merged.is_empty() {
        return Ok(vec![]);
    }

    // Min-max normalize the fused scores and drop the low tail. RRF scores are
    // too compressed (and sensitive to 1-vs-2 list membership) to threshold as
    // a raw fraction of the top; normalizing against the range fixes that.
    let top_score = merged[0].score;
    let bottom_score = merged.last().map_or(top_score, |n| n.score);
    let range = top_score - bottom_score;
    let merged: Vec<_> = if range <= f64::EPSILON {
        // All candidates scored equally — nothing to discriminate on.
        merged
    } else {
        merged
            .into_iter()
            .filter(|n| (n.score - bottom_score) / range >= config.relevance_threshold)
            .collect()
    };

    let node_ids: Vec<i64> = merged.iter().map(|n| n.node.id).collect();
    let embeddings: HashMap<i64, Vec<f32>> =
        db.get_embeddings_for_nodes(node_ids).await?.into_iter().collect();

    let selected = mmr::select(merged, &embeddings, config.mmr_lambda, config.candidate_limit);
    let selected = budget::apply(selected, config.token_budget);
    resolve::resolve(db, selected).await
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Converts a natural-language query into a safe FTS5 `MATCH` expression.
///
/// Each run of alphanumerics, hyphens, and apostrophes becomes one token, and
/// every token is wrapped in double quotes so FTS5 treats it as a literal
/// phrase.  This is essential for the library-docs domain: bare terms like
/// `async-std` or `sqlite-vec` are *not* valid FTS5 barewords (`-` parses as a
/// column filter, uppercase `AND`/`OR`/`NOT` as operators, `'` as a syntax
/// error), so an unquoted query errors out on exactly the identifiers users
/// search for most. Quoting sidesteps all FTS5 operator syntax.
///
/// Embedded double quotes are escaped by doubling (FTS5's own escaping rule).
/// Tokens shorter than two characters are dropped to avoid noise.  Returns an
/// empty string if nothing survives (the caller treats that as "skip FTS5").
fn sanitize_fts_query(query: &str) -> String {
    query
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
        .filter(|t| t.len() >= 2)
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_special_chars() {
        assert_eq!(sanitize_fts_query("hello, world!"), "\"hello\" \"world\"");
    }

    #[test]
    fn sanitize_drops_short_tokens() {
        assert_eq!(sanitize_fts_query("a b cd efg"), "\"cd\" \"efg\"");
    }

    #[test]
    fn sanitize_quotes_hyphenated_and_apostrophe_tokens() {
        // Each token is wrapped as a quoted FTS5 phrase so `-` / `'` never hit
        // FTS5 operator parsing.
        assert_eq!(sanitize_fts_query("don't use async-std"), "\"don't\" \"use\" \"async-std\"");
    }

    #[test]
    fn sanitize_quotes_fts5_operators_as_literals() {
        // Uppercase AND/OR/NOT and parens must not reach FTS5 as operators.
        assert_eq!(sanitize_fts_query("OR NOT working"), "\"OR\" \"NOT\" \"working\"");
    }

    #[test]
    fn sanitize_empty_returns_empty() {
        assert_eq!(sanitize_fts_query(""), "");
    }

    #[test]
    fn sanitize_all_specials_returns_empty() {
        assert_eq!(sanitize_fts_query("!@#$%^&*()"), "");
    }

    #[test]
    fn sanitize_cjk_characters_are_preserved() {
        // CJK chars are alphanumeric in Rust's char::is_alphanumeric(), so
        // they pass through the sanitizer (each token quoted).
        assert_eq!(sanitize_fts_query("日本語 検索"), "\"日本語\" \"検索\"");
    }

    #[test]
    fn sanitize_very_long_query_does_not_panic() {
        let long = "word ".repeat(500);
        let result = sanitize_fts_query(long.trim());
        assert!(!result.is_empty(), "long query should produce tokens");
    }

    #[test]
    fn sanitize_single_char_tokens_are_dropped() {
        // Tokens shorter than 2 chars are filtered out.
        assert_eq!(sanitize_fts_query("a b c hello"), "\"hello\"");
    }

    #[test]
    fn sanitize_mixed_unicode_and_ascii() {
        // Emoji are not alphanumeric — they act as separators.
        // "hello" and "世界" (3 chars, len=9 bytes) both survive; the emoji is stripped.
        let result = sanitize_fts_query("hello 🚀 世界");
        assert!(result.contains("hello"), "ascii word must survive");
        assert!(result.contains("世界"), "CJK word must survive");
        assert!(!result.contains('🚀'), "emoji must be stripped");
    }
}
