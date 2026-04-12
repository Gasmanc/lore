use serde::{Deserialize, Serialize};

/// A documentation file recorded in the `docs` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Doc {
    /// Surrogate primary key.
    pub id: i64,
    /// Path of the file relative to the documentation root, e.g.
    /// `"docs/caching/overview.md"`.
    pub path: String,
    /// Document title extracted from frontmatter or the first `<h1>`.
    pub title: Option<String>,
}
