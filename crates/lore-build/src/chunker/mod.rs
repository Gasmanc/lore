//! Chunking pipeline: structural splitting and semantic refinement.
//!
//! The pipeline operates in two stages:
//!
//! 1. **[`StructuralChunker`]** — walks the [`ParsedDoc`] heading tree and
//!    produces a flat [`ChunkTree`] of [`RawChunk`]s, one per heading section.
//!    Code blocks are always extracted as atomic [`NodeKind::CodeBlock`] chunks.
//!
//! 2. **[`SemanticRefiner`]** — for prose chunks that exceed
//!    [`ChunkConfig::soft_max_tokens`], further splits by detecting cosine-
//!    similarity valleys between consecutive paragraph embeddings.

pub mod semantic;
pub mod structural;

pub use semantic::SemanticRefiner;
pub use structural::StructuralChunker;

use lore_core::NodeKind;

use crate::parser::ContentBlock;

// ── Configuration ─────────────────────────────────────────────────────────────

/// Size thresholds that govern how chunks are formed and refined.
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    /// Prose chunks with fewer tokens than this are candidates for merging
    /// during semantic refinement.
    pub min_tokens: u32,
    /// Prose chunks above this size are marked for semantic refinement.
    pub soft_max_tokens: u32,
    /// Hard ceiling — semantic refinement is mandatory above this size.
    pub hard_max_tokens: u32,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self { min_tokens: 50, soft_max_tokens: 800, hard_max_tokens: 1200 }
    }
}

// ── RawChunk ──────────────────────────────────────────────────────────────────

/// An intermediate chunk produced by the structural chunker and consumed by
/// the semantic refiner and indexer.
#[derive(Debug, Clone)]
pub struct RawChunk {
    /// Ancestry headings from root down to (and including) this section.
    pub heading_path: Vec<String>,
    /// Content blocks belonging to this chunk.
    pub blocks: Vec<ContentBlock>,
    /// Estimated token count of the concatenated block text.
    pub token_count: u32,
    /// `true` when the chunk contains at least one code block.
    pub has_code: bool,
    /// `true` when `token_count > soft_max_tokens` and semantic refinement
    /// should be applied before indexing.
    pub needs_refinement: bool,
    /// File path of the source document (UTF-8 string for `docs` table).
    pub doc_path: String,
    /// Title of the source document, if known.
    pub doc_title: Option<String>,
    /// Whether this is a prose [`NodeKind::Chunk`] or a [`NodeKind::CodeBlock`].
    pub kind: NodeKind,
    /// Source heading levels parallel to [`heading_path`], preserving the
    /// original document structure (e.g. `[2, 3, 4]` for `H2 > H3 > H4`).
    pub heading_levels: Vec<u8>,
}

impl RawChunk {
    /// Returns the concatenated plain text of all content blocks, joined by
    /// double newlines.  Used for token counting and embedding.
    #[must_use]
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(ContentBlock::text)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

// ── ChunkTree ─────────────────────────────────────────────────────────────────

/// A heading path and its parallel source heading levels.
///
/// Recorded for headings that were folded into a parent chunk so the indexer
/// can still materialise them as structural heading nodes in the database.
#[derive(Debug, Clone)]
pub struct FoldedHeading {
    /// Full heading path from root down to this folded heading.
    pub heading_path: Vec<String>,
    /// Source heading levels parallel to `heading_path`.
    pub heading_levels: Vec<u8>,
}

/// A flat list of [`RawChunk`]s with parent-index back-links.
///
/// Each entry is `(chunk, parent_index)`.  The parent index is the position
/// of the parent chunk in this same list, or `None` for top-level chunks.
/// Insertion order guarantees parents always appear before their children.
#[derive(Debug, Default)]
pub struct ChunkTree {
    /// The flattened chunk list with parent back-links.
    pub nodes: Vec<(RawChunk, Option<usize>)>,
    /// Headings whose content was folded into a parent chunk.  The indexer
    /// must still create heading nodes for these so the structural hierarchy
    /// in the database faithfully reflects the source document.
    pub folded_headings: Vec<FoldedHeading>,
}

impl ChunkTree {
    /// Create an empty tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `chunk` with `parent` as its parent index and return the new
    /// chunk's index.
    pub fn push(&mut self, chunk: RawChunk, parent: Option<usize>) -> usize {
        let idx = self.nodes.len();
        self.nodes.push((chunk, parent));
        idx
    }

    /// Returns `true` if the tree contains no chunks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Iterate over all chunks.
    pub fn iter(&self) -> impl Iterator<Item = &(RawChunk, Option<usize>)> {
        self.nodes.iter()
    }

    /// Consume the tree, returning an iterator over all `(chunk, parent_index)` pairs.
    pub fn consume(self) -> impl Iterator<Item = (RawChunk, Option<usize>)> {
        self.nodes.into_iter()
    }
}
