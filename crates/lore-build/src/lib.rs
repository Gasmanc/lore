//! # lore-build
//!
//! Document parsing, chunking, embedding, and indexing pipeline for Lore.
//!
//! The pipeline stages are:
//!
//! 1. **Parsing** — [`parser`] converts raw file content into a
//!    [`parser::ParsedDoc`] heading tree.
//! 2. **Chunking** — [`chunker`] walks the tree and produces a flat
//!    [`chunker::ChunkTree`] of [`chunker::RawChunk`]s.
//! 3. **Embedding** — [`embedder`] encodes each chunk using
//!    `bge-small-en-v1.5` with contextual heading breadcrumbs.
//! 4. **Indexing** — (Phase 5) writes nodes, FTS5 entries, and vector
//!    embeddings into a [`lore_core::Db`].

#![warn(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    missing_docs,
    rust_2018_idioms
)]
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    clippy::must_use_candidate
)]

/// Package build orchestrator — coordinates the full pipeline.
pub mod builder;
/// Chunking pipeline: structural splitting and semantic refinement.
pub mod chunker;
/// File discovery — finds documentation files in a directory tree.
pub mod discovery;
/// Embedding pipeline using `fastembed` with `bge-small-en-v1.5`.
pub mod embedder;
/// File indexing pipeline: parse → chunk → embed → write to `Db`.
pub mod indexer;
/// Document parser trait and format-specific implementations.
pub mod parser;
/// Token counting with the `cl100k_base` BPE tokenizer.
pub mod tokens;

pub use builder::{BuildStats, PackageBuilder};
pub use chunker::{ChunkConfig, ChunkTree, RawChunk, SemanticRefiner, StructuralChunker};
pub use discovery::discover_files;
pub use embedder::{build_contextual_text, Embedder, EMBEDDING_DIMS};
pub use indexer::{FileStats, Indexer};
pub use parser::{
    AsciidocParser, ContentBlock, HeadingNode, HtmlParser, MarkdownParser, ParsedDoc,
    ParserRegistry, RstParser, detect_primary_heading_level,
};
pub use tokens::TokenCounter;
