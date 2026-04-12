use serde::{Deserialize, Serialize};

use crate::node::Node;

/// Tunable parameters that control the search pipeline.
///
/// All fields have conservative defaults suitable for general use.
/// Pass a customised instance to [`crate::db::Db`] search helpers when the
/// caller needs different behaviour (e.g. a tighter token budget for an API
/// look-up vs. a looser one for broad conceptual research).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Maximum number of candidate results to retrieve from each of the FTS5
    /// and vector search stages before fusion.  Defaults to `20`.
    pub candidate_limit: usize,

    /// Fraction of the top BM25 score below which candidates are discarded.
    /// A value of `0.5` means only results within half a BM25 score of the
    /// best match are kept.  Defaults to `0.5`.
    pub relevance_threshold: f64,

    /// Maximum total token count across all returned results.  The pipeline
    /// stops adding results once this budget would be exceeded.  Defaults to
    /// `2000`.
    pub token_budget: u32,

    /// Maximal Marginal Relevance λ parameter.  Controls the trade-off between
    /// relevance and diversity: `1.0` = pure relevance ranking, `0.0` = pure
    /// diversity.  Defaults to `0.7`.
    pub mmr_lambda: f64,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            candidate_limit: 20,
            relevance_threshold: 0.5,
            token_budget: 2000,
            mmr_lambda: 0.7,
        }
    }
}

/// A [`Node`] paired with a relevance score produced by the search pipeline.
#[derive(Debug, Clone)]
pub struct ScoredNode {
    /// The node data.
    pub node: Node,
    /// Relevance score in an arbitrary positive scale (higher = more relevant).
    pub score: f64,
}

/// A fully resolved search result ready to return to a caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matched node.
    pub node: Node,
    /// Path of the source document relative to the documentation root,
    /// e.g. `"docs/caching/overview.md"`.
    pub doc_path: String,
    /// Title of the document this node belongs to.
    pub doc_title: String,
    /// Ordered list of heading titles from the document root to this node's
    /// nearest heading ancestor, e.g. `["Next.js Docs", "Caching",
    /// "cacheLife()"]`.
    pub heading_path: Vec<String>,
    /// Relevance score.
    pub score: f64,
}
