//! Structural chunker — splits a [`ParsedDoc`] heading tree into [`RawChunk`]s
//! at heading boundaries, keeping code blocks atomic.

use lore_core::NodeKind;

use crate::{
    parser::{ContentBlock, HeadingNode, ParsedDoc},
    tokens::TokenCounter,
};

use super::{ChunkConfig, ChunkTree, RawChunk};

// ── Public API ────────────────────────────────────────────────────────────────

/// Splits a parsed document into a [`ChunkTree`] by walking the heading tree.
///
/// The chunker respects three invariants:
///
/// - **Code block atomicity**: every code block becomes its own
///   [`NodeKind::CodeBlock`] chunk, regardless of size.
/// - **Soft-max marking**: prose chunks exceeding [`ChunkConfig::soft_max_tokens`]
///   are flagged `needs_refinement = true` for the [`super::SemanticRefiner`].
/// - **Parent tracking**: each chunk records its parent's index in the
///   [`ChunkTree`], enabling correct `SQLite` path enumeration in Phase 5.
pub struct StructuralChunker {
    config: ChunkConfig,
    counter: TokenCounter,
}

impl StructuralChunker {
    /// Create a new chunker with the given configuration and token counter.
    #[must_use]
    pub const fn new(config: ChunkConfig, counter: TokenCounter) -> Self {
        Self { config, counter }
    }

    /// Walk `doc` and produce a [`ChunkTree`].
    ///
    /// `doc_path` is stored verbatim on every chunk (for the `docs` table
    /// foreign key).  `primary_level` controls which heading level triggers a
    /// new chunk boundary — typically returned by
    /// [`crate::detect_primary_heading_level`].  Headings deeper than
    /// `primary_level` have their content folded into the nearest ancestor
    /// chunk rather than forming separate chunks.
    #[must_use]
    pub fn chunk(&self, doc: &ParsedDoc, doc_path: &str, primary_level: u8) -> ChunkTree {
        let mut tree = ChunkTree::new();
        walk(
            &doc.root,
            None,
            &[],
            &[],
            doc_path,
            doc.title.as_deref(),
            primary_level,
            &mut tree,
            &self.counter,
            &self.config,
        );
        tree
    }
}

// ── Recursive walk ────────────────────────────────────────────────────────────

/// Recursively walk a heading subtree and push [`RawChunk`]s onto `tree`.
///
/// Returns the index of the prose chunk created for `node` (if any), so that
/// child headings can record it as their parent.
///
/// `primary_level` controls chunk granularity: headings deeper (higher
/// `level` value) than `primary_level` have their blocks folded into the
/// nearest ancestor chunk rather than forming separate chunks.
#[allow(clippy::too_many_arguments)] // all parameters carry distinct state
fn walk(
    node: &HeadingNode,
    parent_chunk_idx: Option<usize>,
    heading_path: &[String],
    heading_levels: &[u8],
    doc_path: &str,
    doc_title: Option<&str>,
    primary_level: u8,
    tree: &mut ChunkTree,
    counter: &TokenCounter,
    config: &ChunkConfig,
) -> Option<usize> {
    // Build the path for this heading level.
    let mut path = heading_path.to_vec();
    let mut levels = heading_levels.to_vec();
    if node.level > 0 {
        path.push(node.title.clone());
        levels.push(node.level);
    }

    // Headings deeper than primary_level fold their content into the parent
    // chunk instead of creating a new boundary.  The root node (level 0) and
    // headings at or above primary_level always emit their own chunks.
    // If there is no parent chunk to fold into, a new chunk is created anyway.
    let can_fold = node.level > primary_level && parent_chunk_idx.is_some();

    // Partition blocks: code goes to atomic chunks, prose accumulates.
    let (code_blocks, prose_blocks): (Vec<ContentBlock>, Vec<ContentBlock>) =
        node.blocks.iter().cloned().partition(|b| matches!(b, ContentBlock::Code { .. }));

    // ── Prose chunk ────────────────────────────────────────────────────────
    let prose_idx: Option<usize> = if can_fold {
        // Fold: append blocks to the parent chunk instead of creating a new one.
        let parent_idx = parent_chunk_idx.expect("checked by can_fold");
        if !prose_blocks.is_empty() {
            let (parent_chunk, _) = &mut tree.nodes[parent_idx];
            parent_chunk.blocks.extend(prose_blocks);
            // Re-count tokens after merge.
            let text: String = parent_chunk
                .blocks
                .iter()
                .map(ContentBlock::text)
                .collect::<Vec<_>>()
                .join("\n\n");
            parent_chunk.token_count = counter.count(&text);
            parent_chunk.needs_refinement = parent_chunk.token_count > config.soft_max_tokens;
        }
        // Return parent as the effective chunk for children.
        parent_chunk_idx
    } else if prose_blocks.is_empty() {
        // No prose content at this level — pass the parent down unchanged so
        // child chunks are still properly linked.
        parent_chunk_idx
    } else {
        let text: String =
            prose_blocks.iter().map(ContentBlock::text).collect::<Vec<_>>().join("\n\n");
        let token_count = counter.count(&text);
        let needs_refinement = token_count > config.soft_max_tokens;

        let chunk = RawChunk {
            heading_path: path.clone(),
            heading_levels: levels.clone(),
            blocks: prose_blocks,
            token_count,
            has_code: false,
            needs_refinement,
            doc_path: doc_path.to_owned(),
            doc_title: doc_title.map(str::to_owned),
            kind: NodeKind::Chunk,
        };
        Some(tree.push(chunk, parent_chunk_idx))
    };

    // ── Code chunks (always atomic, always siblings of the prose chunk) ────
    for block in code_blocks {
        let token_count = counter.count(block.text());
        let chunk = RawChunk {
            heading_path: path.clone(),
            heading_levels: levels.clone(),
            blocks: vec![block],
            token_count,
            has_code: true,
            needs_refinement: false, // code blocks are never semantically refined
            doc_path: doc_path.to_owned(),
            doc_title: doc_title.map(str::to_owned),
            kind: NodeKind::CodeBlock,
        };
        // Code chunks are siblings of the prose chunk, so they share the same
        // parent as the prose chunk (not the prose chunk itself).
        tree.push(chunk, parent_chunk_idx);
    }

    // ── Recurse into children ──────────────────────────────────────────────
    // Children use the prose chunk of this heading as their parent so they
    // nest correctly in the path enumeration.
    for child in &node.children {
        walk(child, prose_idx, &path, &levels, doc_path, doc_title, primary_level, tree, counter, config);
    }

    prose_idx
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::parser::{ContentBlock, HeadingNode, ParsedDoc};

    use super::*;

    fn counter() -> TokenCounter {
        TokenCounter::new().expect("cl100k_base must init")
    }

    fn chunker() -> StructuralChunker {
        StructuralChunker::new(ChunkConfig::default(), counter())
    }

    fn para(s: &str) -> ContentBlock {
        ContentBlock::Paragraph(s.into())
    }

    fn code(lang: &str, s: &str) -> ContentBlock {
        ContentBlock::Code { lang: Some(lang.into()), content: s.into() }
    }

    fn heading(level: u8, title: &str, blocks: Vec<ContentBlock>, children: Vec<HeadingNode>) -> HeadingNode {
        HeadingNode { level, title: title.into(), blocks, children }
    }

    fn doc_with_root(children: Vec<HeadingNode>) -> ParsedDoc {
        ParsedDoc {
            title: Some("Test Doc".into()),
            root: HeadingNode { children, ..HeadingNode::root() },
        }
    }

    #[test]
    fn test_chunk_flat_document() {
        // 3 H2 sections, each with a paragraph → 3 prose chunks.
        let doc = doc_with_root(vec![
            heading(2, "Alpha", vec![para("Alpha content.")], vec![]),
            heading(2, "Beta", vec![para("Beta content.")], vec![]),
            heading(2, "Gamma", vec![para("Gamma content.")], vec![]),
        ]);
        let tree = chunker().chunk(&doc, "test.md", 2);
        let prose_chunks: Vec<_> =
            tree.nodes.iter().filter(|(c, _)| c.kind == NodeKind::Chunk).collect();
        assert_eq!(prose_chunks.len(), 3);
        // All are top-level (no parent).
        assert!(prose_chunks.iter().all(|(_, p)| p.is_none()));
    }

    #[test]
    fn test_chunk_code_block_atomic() {
        // One H2 with both prose and a code block → prose chunk + code chunk.
        let big_code = "fn ".repeat(200); // ~200 tokens
        let doc = doc_with_root(vec![heading(
            2,
            "Usage",
            vec![para("Here is an example:"), code("rust", &big_code)],
            vec![],
        )]);
        let tree = chunker().chunk(&doc, "test.md", 2);
        assert_eq!(tree.nodes.len(), 2);

        let prose = tree.nodes.iter().find(|(c, _)| c.kind == NodeKind::Chunk);
        let code_chunk = tree.nodes.iter().find(|(c, _)| c.kind == NodeKind::CodeBlock);
        assert!(prose.is_some(), "expected prose chunk");
        assert!(code_chunk.is_some(), "expected code chunk");

        // Code chunk must not be split — single block.
        assert_eq!(code_chunk.unwrap().0.blocks.len(), 1);
        // Code chunk never needs refinement.
        assert!(!code_chunk.unwrap().0.needs_refinement);
    }

    #[test]
    fn test_chunk_api_reference() {
        // H2 heading with no content of its own, but 10 H3 children each with
        // a prose paragraph → 10 prose chunks.
        let children: Vec<HeadingNode> = (0..10)
            .map(|i| heading(3, &format!("func_{i}"), vec![para("Description.")], vec![]))
            .collect();
        let doc = doc_with_root(vec![heading(2, "API", vec![], children)]);
        let tree = chunker().chunk(&doc, "api.md", 2);
        assert_eq!(tree.nodes.iter().filter(|(c, _)| c.kind == NodeKind::Chunk).count(), 10);
    }

    #[test]
    fn test_chunk_marks_large_for_refinement() {
        // A paragraph with > 800 tokens should be flagged.
        // Approximate: 4 chars ≈ 1 token, so 4 000 chars ≈ 1000 tokens.
        let big_para = para(&"word ".repeat(900));
        let doc = doc_with_root(vec![heading(2, "Large Section", vec![big_para], vec![])]);
        let tree = chunker().chunk(&doc, "large.md", 2);
        let chunk = &tree.nodes[0].0;
        assert!(
            chunk.needs_refinement,
            "expected needs_refinement=true for ~900-word paragraph, token_count={}",
            chunk.token_count
        );
    }

    #[test]
    fn test_chunk_heading_path_propagates() {
        // H2 > H3 > H4 — the H4 chunk's heading_path should include all ancestors.
        let h4 = heading(4, "Deep", vec![para("Deep content.")], vec![]);
        let h3 = heading(3, "Middle", vec![], vec![h4]);
        let h2 = heading(2, "Top", vec![], vec![h3]);
        let doc = doc_with_root(vec![h2]);
        let tree = chunker().chunk(&doc, "nested.md", 2);
        let deep = &tree.nodes[0].0;
        assert_eq!(deep.heading_path, vec!["Top", "Middle", "Deep"]);
    }

    #[test]
    fn test_chunk_code_never_refined() {
        let large_code = code("rust", &"fn f() {} ".repeat(500));
        let doc = doc_with_root(vec![heading(2, "Code", vec![large_code], vec![])]);
        let tree = chunker().chunk(&doc, "code.md", 2);
        let code_chunk = tree.nodes.iter().find(|(c, _)| c.kind == NodeKind::CodeBlock).unwrap();
        assert!(!code_chunk.0.needs_refinement);
    }
}
