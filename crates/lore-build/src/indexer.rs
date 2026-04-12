//! File indexing pipeline: parse → chunk → embed → write to [`Db`].
//!
//! [`Indexer`] is the central coordinator for a single documentation file.
//! It owns shared references to the pipeline components and exposes a single
//! [`Indexer::index_file`] method.  All database I/O is async; parsing and
//! chunking are synchronous.  Batch embedding runs in a `spawn_blocking` task
//! so the Tokio reactor is never stalled by neural-network inference.

use std::collections::HashMap;
use std::path::Path;

use lore_core::{Db, LoreError, NewNode, NodeKind};
use tracing::{debug, instrument, warn};

use crate::{
    chunker::{ChunkTree, RawChunk, SemanticRefiner, StructuralChunker},
    embedder::{Embedder, build_contextual_text},
    parser::{ParserRegistry, detect_primary_heading_level},
};

// ── Public types ──────────────────────────────────────────────────────────────

/// Per-file statistics returned by [`Indexer::index_file`].
#[derive(Debug, Clone, Default)]
pub struct FileStats {
    /// Number of prose [`NodeKind::Chunk`] nodes inserted.
    pub chunk_count: u32,
    /// Number of [`NodeKind::CodeBlock`] nodes inserted.
    pub code_block_count: u32,
    /// Total token count across all inserted chunks.
    pub total_tokens: u64,
}

impl FileStats {
    fn accumulate(&mut self, chunk: &RawChunk) {
        use lore_core::NodeKind;
        match chunk.kind {
            NodeKind::Chunk => {
                self.chunk_count += 1;
                self.total_tokens += u64::from(chunk.token_count);
            }
            NodeKind::CodeBlock => {
                self.code_block_count += 1;
                self.total_tokens += u64::from(chunk.token_count);
            }
            NodeKind::Heading => {}
        }
    }
}

/// Coordinates the parse → chunk → embed → write pipeline for a single file.
///
/// Construct once per build run and reuse across all files — each component
/// (tokeniser, model, …) is initialised exactly once.
pub struct Indexer {
    parsers:  ParserRegistry,
    chunker:  StructuralChunker,
    refiner:  SemanticRefiner,
    embedder: Embedder,
    db:       Db,
}

impl Indexer {
    /// Creates an `Indexer` using the given components.
    ///
    /// In production use [`crate::builder::PackageBuilder`] which constructs
    /// and wires everything together.  In tests you can pass individual
    /// instances directly.
    #[must_use]
    pub const fn new(
        parsers:  ParserRegistry,
        chunker:  StructuralChunker,
        refiner:  SemanticRefiner,
        embedder: Embedder,
        db:       Db,
    ) -> Self {
        Self { parsers, chunker, refiner, embedder, db }
    }

    /// Returns a reference to the database used by this indexer.
    pub const fn db(&self) -> &Db {
        &self.db
    }

    /// Parses, chunks, embeds, and inserts all nodes from `content` into the
    /// database.
    ///
    /// `path` is stored as-is on the [`lore_core::Doc`] record and on every
    /// [`lore_core::Node`] — it should be a relative path from the package
    /// root so that the resulting `.db` is portable.
    ///
    /// Returns per-file counts for the caller to accumulate into build stats.
    /// Returns `None` if no chunks were produced (e.g. empty file).
    ///
    /// # Errors
    ///
    /// Returns [`LoreError`] if parsing, embedding, or any database operation
    /// fails.
    #[instrument(skip(self, content), fields(path = %path.as_ref().display()))]
    pub async fn index_file(
        &self,
        path:    impl AsRef<Path>,
        content: &str,
    ) -> Result<Option<FileStats>, LoreError> {
        let path = path.as_ref();
        let path_str = path.to_string_lossy().into_owned();

        let doc = self.parsers.parse(path, content).map_err(|e| {
            warn!(path = %path.display(), error = %e, "parse failed");
            e
        })?;

        let primary_level = detect_primary_heading_level(&doc.root);
        let tree = self.chunker.chunk(&doc, &path_str, primary_level);
        let refined = self.refine_tree(tree)?;

        // Ensure the doc record exists and purge any previously indexed nodes
        // so rebuilds are idempotent.  This must happen *before* the empty
        // check: a file that previously produced chunks but now yields none
        // (e.g. edited to empty) must still have its stale nodes removed.
        let doc_id = self.db.insert_doc(path_str, doc.title).await?;
        self.db.delete_nodes_for_doc(doc_id).await?;

        if refined.is_empty() {
            debug!(path = %path.display(), "no chunks produced; skipping");
            return Ok(None);
        }

        // Pass 1: insert all heading and content nodes, collect texts to embed.
        // heading_path → node_id cache: each unique heading inserted once per doc.
        let mut heading_cache: HashMap<Vec<String>, i64> = HashMap::new();
        let mut file_stats = FileStats::default();
        let mut embed_queue: Vec<(i64, String)> = Vec::new();

        for (chunk, _) in refined.iter() {
            let parent_id = self
                .ensure_heading_chain(&chunk.heading_path, &chunk.heading_levels, doc_id, &mut heading_cache)
                .await?;

            let node_id = self.insert_chunk(chunk, doc_id, parent_id).await?;
            file_stats.accumulate(chunk);

            if let Some(content_text) = chunk.text_for_embedding() {
                let ctx_text = build_contextual_text(&chunk.heading_path, &content_text);
                embed_queue.push((node_id, ctx_text));
            }
        }

        // Materialise folded headings as structural nodes so the DB hierarchy
        // faithfully reflects the source document even though their content was
        // merged into a parent chunk.  `ensure_heading_chain` is idempotent via
        // `heading_cache`, so headings already created by chunk processing above
        // are not duplicated.
        for fh in &refined.folded_headings {
            self.ensure_heading_chain(fh.heading_path(), fh.heading_levels(), doc_id, &mut heading_cache)
                .await?;
        }

        // Pass 2: batch-embed all texts in one blocking call, then persist.
        // Grouping into a single spawn_blocking avoids reactor stalls and uses
        // ONNX batch inference, which is faster than N individual calls.
        if !embed_queue.is_empty() {
            let texts: Vec<String> = embed_queue.iter().map(|(_, t)| t.clone()).collect();
            let embedder = self.embedder.clone();
            let embeddings = tokio::task::spawn_blocking(move || embedder.embed_batch(&texts))
                .await
                .map_err(|e| LoreError::Embed(e.to_string()))??;

            for ((node_id, _), embedding) in embed_queue.iter().zip(embeddings) {
                self.db.insert_embedding(*node_id, embedding).await?;
            }
        }

        Ok(Some(file_stats))
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Runs semantic refinement over every chunk in the tree that is flagged
    /// `needs_refinement`.  Returns a flat `Vec` preserving the order and
    /// parent relationships.
    fn refine_tree(&self, tree: ChunkTree) -> Result<ChunkTree, LoreError> {
        let mut out = ChunkTree::new();
        // Propagate folded headings — refinement does not affect them.
        out.folded_headings = tree.folded_headings;
        for (chunk, parent_idx) in tree.nodes {
            if chunk.needs_refinement {
                let sub_chunks = self.refiner.refine(chunk, &self.embedder)?;
                for sub in sub_chunks {
                    out.push(sub, parent_idx);
                }
            } else {
                out.push(chunk, parent_idx);
            }
        }
        Ok(out)
    }

    /// Walks `heading_path` and inserts any heading nodes that do not yet
    /// exist for this document, returning the `id` of the deepest heading.
    ///
    /// `heading_levels` carries the original heading levels from the source
    /// document (parallel to `heading_path`).  When a heading is first inserted,
    /// its level is taken from this slice so that the stored level faithfully
    /// reflects the source structure (e.g. an `H3` is stored as level 3, not
    /// depth 2).
    ///
    /// `heading_cache` maps the full heading path (as a `Vec<String>`) to the
    /// already-inserted node id, so duplicate headings across chunks are not
    /// re-inserted.
    async fn ensure_heading_chain(
        &self,
        heading_path:   &[String],
        heading_levels: &[u8],
        doc_id:         i64,
        cache:          &mut HashMap<Vec<String>, i64>,
    ) -> Result<Option<i64>, LoreError> {
        debug_assert_eq!(
            heading_path.len(),
            heading_levels.len(),
            "heading_path and heading_levels must have the same length"
        );

        if heading_path.is_empty() {
            return Ok(None);
        }

        let mut parent_id: Option<i64> = None;

        for depth in 1..=heading_path.len() {
            let prefix = heading_path[..depth].to_vec();

            if let Some(&cached_id) = cache.get(&prefix) {
                parent_id = Some(cached_id);
                continue;
            }

            // Use the source heading level when available, falling back to
            // depth (which is correct for headings created before heading_levels
            // was populated).
            #[allow(clippy::cast_possible_truncation)]
            let level = heading_levels
                .get(depth - 1)
                .copied()
                .unwrap_or(depth as u8);
            let title = heading_path[depth - 1].clone();

            let node_id = self
                .db
                .insert_node(NewNode {
                    parent_id,
                    doc_id,
                    kind:        NodeKind::Heading,
                    level:       Some(level),
                    title:       Some(title),
                    content:     None,
                    token_count: 0,
                    lang:        None,
                })
                .await?;

            cache.insert(prefix, node_id);
            parent_id = Some(node_id);
        }

        Ok(parent_id)
    }

    /// Inserts a single [`RawChunk`] as a `Chunk` or `CodeBlock` node.
    async fn insert_chunk(
        &self,
        chunk:     &RawChunk,
        doc_id:    i64,
        parent_id: Option<i64>,
    ) -> Result<i64, LoreError> {
        let (content, lang) = chunk.content_and_lang();
        self.db
            .insert_node(NewNode {
                parent_id,
                doc_id,
                kind:        chunk.kind,
                level:       None,
                title:       None,
                content:     Some(content),
                token_count: chunk.token_count,
                lang,
            })
            .await
    }
}

// ── RawChunk extensions ───────────────────────────────────────────────────────

/// Private helper methods on [`RawChunk`] used only by the indexer.
trait RawChunkExt {
    /// Returns the text content suitable for embedding (prose or code text),
    /// or `None` if the chunk is empty.
    fn text_for_embedding(&self) -> Option<String>;

    /// Returns the primary text content and the optional language tag (for
    /// code blocks).
    fn content_and_lang(&self) -> (String, Option<String>);
}

impl RawChunkExt for RawChunk {
    fn text_for_embedding(&self) -> Option<String> {
        let t = self.text();
        if t.trim().is_empty() { None } else { Some(t) }
    }

    fn content_and_lang(&self) -> (String, Option<String>) {
        use crate::parser::ContentBlock;
        // Code chunks: single Code block → extract text and lang.
        if self.kind == NodeKind::CodeBlock {
            if let Some(ContentBlock::Code { lang, content }) = self.blocks.first() {
                return (content.clone(), lang.clone());
            }
        }
        // Prose chunks: join all block text.
        (self.text(), None)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use super::*;
    use crate::{
        chunker::ChunkConfig,
        embedder::Embedder,
        tokens::TokenCounter,
    };

    static EMBEDDER: LazyLock<Embedder> = LazyLock::new(|| {
        let cache = dirs_next::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("lore")
            .join("models");
        Embedder::new(&cache).expect("embedder must init")
    });

    fn make_indexer(db: Db) -> Indexer {
        let config = ChunkConfig::default();
        Indexer::new(
            ParserRegistry::new(),
            StructuralChunker::new(config.clone(), TokenCounter::new().expect("tokenizer")),
            SemanticRefiner::new(config, TokenCounter::new().expect("tokenizer")),
            EMBEDDER.clone(),
            db,
        )
    }

    #[tokio::test]
    async fn test_index_simple_markdown() {
        let db = Db::open_in_memory().await.expect("db open");
        let indexer = make_indexer(db.clone());

        let md = "# My Library\n\nIntroduction paragraph.\n\n## Installation\n\nRun `cargo add mylib`.\n\n## Usage\n\nImport and call `run()`.\n";
        indexer
            .index_file(std::path::Path::new("test.md"), md)
            .await
            .expect("index should succeed");

        let doc = db.get_doc(1).await.expect("doc 1 should exist");
        assert_eq!(doc.path, "test.md");
    }

    #[tokio::test]
    async fn test_index_code_block_gets_embedding() {
        let db = Db::open_in_memory().await.expect("db open");
        let indexer = make_indexer(db.clone());

        let md = "## Usage\n\nHere is an example:\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n";
        indexer
            .index_file(Path::new("code.md"), md)
            .await
            .expect("index should succeed");

        // Verify the code-block node exists and has an embedding.
        let code_node = {
            // Walk nodes to find the CodeBlock.
            let mut found = None;
            for id in 1i64..=20 {
                if let Ok(n) = db.get_node(id).await {
                    if n.kind == NodeKind::CodeBlock {
                        found = Some(n);
                        break;
                    }
                }
            }
            found.expect("CodeBlock node must exist")
        };
        let emb = db.get_embedding(code_node.id).await.expect("embedding query");
        assert!(emb.is_some(), "code block must have an embedding");
        assert_eq!(emb.unwrap().len(), crate::embedder::EMBEDDING_DIMS);
    }

    #[tokio::test]
    async fn test_index_empty_file_does_not_error() {
        let db = Db::open_in_memory().await.expect("db open");
        let indexer = make_indexer(db.clone());
        // A file with no real content should not error.
        indexer
            .index_file(Path::new("empty.md"), "")
            .await
            .expect("empty file must not error");
    }

    #[tokio::test]
    async fn test_folded_headings_create_structural_nodes() {
        // H2 "Guide" with prose, then H3 "Setup" with prose.  detect_primary
        // will return 2, so H3 is folded into H2's chunk.  The H3 heading
        // node must still exist in the database.
        let db = Db::open_in_memory().await.expect("db open");
        let indexer = make_indexer(db.clone());

        let md = "## Guide\n\nIntro paragraph.\n\n### Setup\n\nInstall instructions.\n";
        indexer
            .index_file(Path::new("fold.md"), md)
            .await
            .expect("index should succeed");

        // Collect all heading nodes for the doc.
        let nodes = db.get_nodes_for_doc(1).await.expect("get nodes");
        let headings: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Heading)
            .collect();

        // Both H2 "Guide" and H3 "Setup" must exist as heading nodes.
        assert_eq!(headings.len(), 2, "expected both H2 and folded H3 heading nodes");
        assert_eq!(headings[0].title.as_deref(), Some("Guide"));
        assert_eq!(headings[0].level, Some(2));
        assert_eq!(headings[1].title.as_deref(), Some("Setup"));
        assert_eq!(headings[1].level, Some(3));

        // H3 "Setup" must be a child of H2 "Guide".
        assert_eq!(headings[1].parent_id, Some(headings[0].id));
    }

    #[tokio::test]
    async fn test_rebuild_purges_stale_nodes() {
        let db = Db::open_in_memory().await.expect("db open");
        let indexer = make_indexer(db.clone());

        // First build: file with content.
        let md = "## Section\n\nContent here.\n";
        indexer
            .index_file(Path::new("doc.md"), md)
            .await
            .expect("first index");

        let nodes_before = db.get_nodes_for_doc(1).await.expect("get nodes");
        assert!(!nodes_before.is_empty(), "should have nodes after first build");

        // Second build: same path, now empty.
        indexer
            .index_file(Path::new("doc.md"), "")
            .await
            .expect("second index (empty)");

        let nodes_after = db.get_nodes_for_doc(1).await.expect("get nodes");
        assert!(nodes_after.is_empty(), "stale nodes should be purged on empty rebuild");
    }
}
