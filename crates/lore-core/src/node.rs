use serde::{Deserialize, Serialize};

use crate::error::LoreError;

/// Discriminates the three kinds of node stored in the `nodes` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A heading node (`<h1>`–`<h6>`).  Heading nodes form the structural
    /// skeleton of a document and are never returned as search results on
    /// their own; they exist solely to carry breadcrumb context.
    Heading,
    /// A prose chunk – one or more paragraphs that belong under a heading.
    Chunk,
    /// A code block.  Code blocks are always atomic: they are never split
    /// across chunk boundaries regardless of their token count.
    CodeBlock,
}

impl NodeKind {
    /// Returns the lowercase string representation stored in the database.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Heading => "heading",
            Self::Chunk => "chunk",
            Self::CodeBlock => "code_block",
        }
    }
}

impl TryFrom<&str> for NodeKind {
    type Error = LoreError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "heading" => Ok(Self::Heading),
            "chunk" => Ok(Self::Chunk),
            "code_block" => Ok(Self::CodeBlock),
            other => Err(LoreError::Schema(format!("unknown node kind: {other:?}"))),
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A fully hydrated row from the `nodes` table, including its computed `path`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Surrogate primary key.
    pub id: i64,
    /// Parent node id, or `None` for root-level nodes.
    pub parent_id: Option<i64>,
    /// Slash-separated path encoding the full ancestry, e.g. `"1/4/9/23"`.
    /// Enables fast subtree queries via `LIKE 'prefix/%'`.
    pub path: String,
    /// Foreign key to the `docs` table.
    pub doc_id: i64,
    /// Discriminant: heading, prose chunk, or code block.
    pub kind: NodeKind,
    /// Heading level (1–6).  `None` for chunks and code blocks.
    pub level: Option<u8>,
    /// Heading text, or `None` for non-heading nodes.
    pub title: Option<String>,
    /// Textual content, or `None` for heading-only nodes.
    pub content: Option<String>,
    /// Approximate token count for this node's content.
    pub token_count: u32,
    /// Programming language for code blocks (e.g. `"rust"`), or `None`.
    pub lang: Option<String>,
}

/// The subset of [`Node`] fields supplied by callers when inserting a new node.
/// The `id` and `path` fields are assigned by [`crate::db::Db::insert_node`].
#[derive(Debug, Clone)]
pub struct NewNode {
    /// Parent node id, or `None` for root-level nodes.
    pub parent_id: Option<i64>,
    /// Foreign key to the `docs` table.
    pub doc_id: i64,
    /// Discriminant: heading, prose chunk, or code block.
    pub kind: NodeKind,
    /// Heading level (1–6).  `None` for chunks and code blocks.
    pub level: Option<u8>,
    /// Heading text, or `None` for non-heading nodes.
    pub title: Option<String>,
    /// Textual content, or `None` for heading-only nodes.
    pub content: Option<String>,
    /// Approximate token count for this node's content.
    pub token_count: u32,
    /// Programming language for code blocks (e.g. `"rust"`), or `None`.
    pub lang: Option<String>,
}
