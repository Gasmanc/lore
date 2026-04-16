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

#![deny(clippy::all, clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![allow(clippy::module_name_repetitions, clippy::missing_errors_doc, clippy::must_use_candidate)]

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

    let merged = rrf::merge(&[fts_hits, vec_hits]);
    if merged.is_empty() {
        return Ok(vec![]);
    }

    let top_score = merged[0].score;
    let merged: Vec<_> =
        merged.into_iter().filter(|n| n.score >= top_score * config.relevance_threshold).collect();

    let node_ids: Vec<i64> = merged.iter().map(|n| n.node.id).collect();
    let embeddings: HashMap<i64, Vec<f32>> =
        db.get_embeddings_for_nodes(node_ids).await?.into_iter().collect();

    let selected = mmr::select(merged, &embeddings, config.mmr_lambda, limit);
    let selected = budget::apply(selected, config.token_budget);
    resolve::resolve(db, selected).await
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Strips FTS5-special characters from a natural-language query so that it
/// can be passed directly to `nodes_fts MATCH ?`.
///
/// Keeps alphanumeric characters, hyphens, and apostrophes.  Tokens shorter
/// than two characters are dropped to avoid noise.  Returns an empty string
/// if nothing survives (the caller treats that as "skip FTS5").
fn sanitize_fts_query(query: &str) -> String {
    let tokens: Vec<&str> = query
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
        .filter(|t| t.len() >= 2)
        .collect();
    tokens.join(" ")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_special_chars() {
        assert_eq!(sanitize_fts_query("hello, world!"), "hello world");
    }

    #[test]
    fn sanitize_drops_short_tokens() {
        assert_eq!(sanitize_fts_query("a b cd efg"), "cd efg");
    }

    #[test]
    fn sanitize_preserves_hyphen_and_apostrophe() {
        assert_eq!(sanitize_fts_query("don't use async-std"), "don't use async-std");
    }

    #[test]
    fn sanitize_empty_returns_empty() {
        assert_eq!(sanitize_fts_query(""), "");
    }

    #[test]
    fn sanitize_all_specials_returns_empty() {
        assert_eq!(sanitize_fts_query("!@#$%^&*()"), "");
    }
}
