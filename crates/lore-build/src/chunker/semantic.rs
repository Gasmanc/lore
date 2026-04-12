//! Semantic refinement — splits oversized prose chunks at topic boundaries.
//!
//! For chunks flagged `needs_refinement = true`, [`SemanticRefiner`] embeds
//! each paragraph individually, computes cosine-similarity between consecutive
//! pairs, and splits where the similarity drops below `mean − 1.0 × std_dev`.
//! Resulting fragments below `min_tokens` are merged with an adjacent chunk.

use lore_core::NodeKind;

// Re-export so embedder tests can import `crate::chunker::semantic::cosine_similarity`.
pub use lore_core::cosine_similarity;

use crate::{
    embedder::Embedder,
    parser::ContentBlock,
    tokens::TokenCounter,
};

use super::{ChunkConfig, RawChunk};

// ── Public API ────────────────────────────────────────────────────────────────

/// Applies semantic splitting to oversized prose chunks.
///
/// Stateless beyond the shared [`ChunkConfig`]; the caller passes an
/// [`Embedder`] at call time so the same embedder instance can be shared
/// across the full indexing pipeline.
pub struct SemanticRefiner {
    config: ChunkConfig,
    counter: TokenCounter,
}

impl SemanticRefiner {
    /// Create a refiner with the given configuration.
    pub const fn new(config: ChunkConfig, counter: TokenCounter) -> Self {
        Self { config, counter }
    }

    /// Refine `chunk` by splitting at semantic topic boundaries.
    ///
    /// Returns `vec![chunk]` unchanged if:
    /// - `chunk.needs_refinement` is `false`
    /// - the chunk contains fewer than 3 prose paragraphs
    /// - no similarity valley is found
    ///
    /// # Errors
    ///
    /// Returns [`lore_core::LoreError::Embed`] if embedding fails.
    pub fn refine(
        &self,
        chunk: RawChunk,
        embedder: &Embedder,
    ) -> Result<Vec<RawChunk>, lore_core::LoreError> {
        if !chunk.needs_refinement {
            return Ok(vec![chunk]);
        }

        // Collect prose paragraphs (exclude code blocks).
        let paragraphs: Vec<&str> = chunk
            .blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Paragraph(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();

        if paragraphs.len() < 3 {
            return Ok(vec![chunk]);
        }

        // Embed each paragraph (without breadcrumb — these are fragments,
        // not final chunks).
        let embeddings = embedder.embed_batch(
            &paragraphs.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
        )?;

        // Compute cosine similarity between consecutive paragraph embeddings.
        let similarities: Vec<f32> = embeddings
            .windows(2)
            .map(|w| cosine_similarity(&w[0], &w[1]))
            .collect();

        // Find split positions using the valley detection heuristic.
        let split_positions = valley_positions(&similarities);
        if split_positions.is_empty() {
            return Ok(vec![chunk]);
        }

        // Build a list of segments (each segment is a contiguous slice of
        // ALL blocks, not just prose, that belong between two split points).
        let segments = split_chunk_at(&chunk, &paragraphs, &split_positions);

        // Merge tiny segments (token_count < min_tokens).
        let merged = merge_tiny_segments(segments, self.config.min_tokens, &self.counter);

        Ok(merged)
    }
}

// ── Valley detection ──────────────────────────────────────────────────────────

/// Return the indices `i` where `similarities[i] < mean - 1.0 * std_dev`.
///
/// A split at index `i` means: split between paragraph `i` and paragraph `i+1`.
fn valley_positions(similarities: &[f32]) -> Vec<usize> {
    if similarities.is_empty() {
        return vec![];
    }

    #[allow(clippy::cast_precision_loss)] // n never exceeds a few hundred
    let n = similarities.len() as f32;
    let mean: f32 = similarities.iter().sum::<f32>() / n;
    let variance: f32 =
        similarities.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / n;
    let std_dev = variance.sqrt();

    let threshold = std_dev.mul_add(-1.0, mean);
    similarities
        .iter()
        .enumerate()
        .filter_map(|(i, &s)| if s < threshold { Some(i) } else { None })
        .collect()
}

// ── Segment builder ───────────────────────────────────────────────────────────

/// Split a chunk's blocks at paragraph-level split positions.
///
/// `paragraphs` is the ordered slice of prose paragraph strings extracted from
/// `chunk.blocks`.  `split_positions[i]` means split AFTER paragraph `i`.
fn split_chunk_at(
    chunk: &RawChunk,
    paragraphs: &[&str],
    split_positions: &[usize],
) -> Vec<RawChunk> {
    let mut segments: Vec<RawChunk> = Vec::new();
    let mut current_blocks: Vec<ContentBlock> = Vec::new();
    let mut para_idx = 0usize;

    for block in &chunk.blocks {
        current_blocks.push(block.clone());
        if matches!(block, ContentBlock::Paragraph(_)) {
            if split_positions.contains(&para_idx) && !current_blocks.is_empty() {
                // Flush current segment.
                segments.push(make_segment(chunk, std::mem::take(&mut current_blocks)));
            }
            para_idx += 1;
        }
    }

    // Flush the final segment.
    if !current_blocks.is_empty() {
        segments.push(make_segment(chunk, current_blocks));
    }

    // If no split actually happened, return the original.
    if segments.is_empty() {
        let _ = paragraphs; // suppress unused warning
        return vec![chunk.clone()];
    }

    segments
}

/// Construct a [`RawChunk`] for a sub-segment of the original chunk.
fn make_segment(original: &RawChunk, blocks: Vec<ContentBlock>) -> RawChunk {
    // Mark needs_refinement = false — this segment is already refined.
    RawChunk {
        heading_path: original.heading_path.clone(),
        blocks,
        token_count: 0, // recalculated by merge_tiny_segments
        has_code: false,
        needs_refinement: false,
        doc_path: original.doc_path.clone(),
        doc_title: original.doc_title.clone(),
        kind: NodeKind::Chunk,
    }
}

// ── Tiny-segment merger ───────────────────────────────────────────────────────

/// Merge any segment with fewer than `min_tokens` into its neighbour.
///
/// Prefers merging with the *following* segment; falls back to the preceding
/// one for the last segment.
fn merge_tiny_segments(
    mut segments: Vec<RawChunk>,
    min_tokens: u32,
    counter: &TokenCounter,
) -> Vec<RawChunk> {
    // First pass: assign token counts.
    for seg in &mut segments {
        seg.token_count = counter.count(&seg.text());
    }

    // Merge loop: keep merging until stable.
    loop {
        let tiny = segments.iter().position(|s| s.token_count < min_tokens);
        let Some(idx) = tiny else { break };

        if idx + 1 < segments.len() {
            // Merge into the following segment.
            let next = segments.remove(idx + 1);
            let cur = &mut segments[idx];
            cur.blocks.extend(next.blocks);
            cur.token_count = counter.count(&cur.text());
        } else if idx > 0 {
            // Last segment — merge into the preceding one.
            let cur = segments.remove(idx);
            let prev = &mut segments[idx - 1];
            prev.blocks.extend(cur.blocks);
            prev.token_count = counter.count(&prev.text());
        } else {
            // Single-element list that is still tiny — nothing to merge.
            break;
        }
    }

    segments
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use super::*;
    use crate::chunker::ChunkConfig;

    fn refiner() -> SemanticRefiner {
        SemanticRefiner::new(
            ChunkConfig::default(),
            TokenCounter::new().expect("tokenizer must init"),
        )
    }

    /// Shared embedder — initialised at most once per test binary execution.
    static EMBEDDER: LazyLock<Embedder> = LazyLock::new(|| {
        let cache = dirs_next::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("lore")
            .join("models");
        Embedder::new(&cache).expect("embedder must init")
    });

    fn prose_chunk(paragraphs: &[&str]) -> RawChunk {
        let blocks = paragraphs
            .iter()
            .map(|p| ContentBlock::Paragraph((*p).to_owned()))
            .collect::<Vec<_>>();
        let text = paragraphs.join("\n\n");
        // Build a counter just for token counting in the test helper.
        let counter = TokenCounter::new().unwrap();
        let token_count = counter.count(&text);
        RawChunk {
            heading_path: vec!["Section".into()],
            blocks,
            token_count,
            has_code: false,
            needs_refinement: token_count > ChunkConfig::default().soft_max_tokens,
            doc_path: "test.md".into(),
            doc_title: None,
            kind: NodeKind::Chunk,
        }
    }

    // ── Unit tests (no embedder required) ────────────────────────────────────

    #[test]
    fn test_valley_no_split_uniform() {
        // All similarities equal → no valleys.
        let sims = vec![0.8, 0.8, 0.8, 0.8];
        assert!(valley_positions(&sims).is_empty());
    }

    #[test]
    fn test_valley_detects_dip() {
        // One obvious valley.
        let sims = vec![0.9, 0.9, 0.1, 0.9, 0.9];
        let positions = valley_positions(&sims);
        assert!(positions.contains(&2), "expected valley at index 2, got {positions:?}");
    }

    #[test]
    fn test_no_refinement_when_not_flagged() {
        let chunk = prose_chunk(&["Short paragraph."]);
        assert!(!chunk.needs_refinement);
        let result = refiner().refine(chunk.clone(), &EMBEDDER).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].blocks.len(), chunk.blocks.len());
    }

    #[test]
    fn test_no_split_too_few_paragraphs() {
        // With < 3 paragraphs, refinement is skipped even if flagged.
        let mut chunk = prose_chunk(&["Para one.", "Para two."]);
        chunk.needs_refinement = true;
        let result = refiner().refine(chunk, &EMBEDDER).unwrap();
        assert_eq!(result.len(), 1);
    }

    // ── Integration tests (require embedding model) ───────────────────────────

    #[test]
    fn test_no_split_single_topic() {
        // Five paragraphs all about Rust's borrow checker — should not split.
        let paragraphs = [
            "The borrow checker is the component of the Rust compiler that enforces memory safety at compile time.",
            "It ensures that references do not outlive the data they point to, preventing dangling pointers.",
            "Rust distinguishes between shared references and mutable references using the ownership model.",
            "At any given point, you may have either one mutable reference or any number of shared references.",
            "The borrow checker tracks these invariants statically, without any runtime overhead.",
        ];
        let mut chunk = prose_chunk(&paragraphs);
        chunk.needs_refinement = true;

        let result = refiner().refine(chunk, &EMBEDDER).unwrap();
        // Should remain as one chunk since all paragraphs are about the same topic.
        assert_eq!(
            result.len(),
            1,
            "single-topic text should not split, but got {} chunks",
            result.len()
        );
    }

    #[test]
    fn test_splits_multi_topic() {
        // First three paragraphs: Rust memory safety (each kept long enough so the
        // segment stays above min_tokens=50 after a split).
        // Last three paragraphs: French cuisine techniques.
        // The semantic shift between the two topics should be detected.
        let paragraphs = [
            "Rust's ownership model ensures memory safety without a garbage collector by requiring the programmer to reason about lifetimes and borrows at compile time rather than at runtime.",
            "The borrow checker enforces these rules at compile time, producing programs that are both fast and safe while eliminating entire classes of bugs such as use-after-free and data races.",
            "RAII (Resource Acquisition Is Initialization) is central to Rust's resource management strategy, ensuring destructors run deterministically when values go out of scope.",
            "Classic French cuisine relies on a repertoire of mother sauces — Béchamel, Velouté, Espagnole, Sauce Tomat, and Hollandaise — as the foundation for thousands of derived sauces.",
            "Techniques such as sautéing, braising, flambéing, and en papillote define the French culinary tradition and are taught in every classical cooking school around the world.",
            "The mirepoix — a mixture of diced onion, carrot, and celery cooked gently in butter — forms the aromatic base of countless French stocks, soups, stews, and braises.",
        ];

        let mut chunk = prose_chunk(&paragraphs);
        chunk.needs_refinement = true;

        let result = refiner().refine(chunk, &EMBEDDER).unwrap();
        // There should be at least two chunks after splitting.
        assert!(
            result.len() >= 2,
            "multi-topic text should split into at least 2 chunks, got {}",
            result.len()
        );
    }

    #[test]
    fn test_merges_tiny_fragment() {
        // Craft a chunk where one refinement segment would be a single short
        // paragraph (well below min_tokens=50).  After merging, no segment
        // should be below min_tokens.
        let refiner = SemanticRefiner::new(
            ChunkConfig { min_tokens: 20, soft_max_tokens: 10, hard_max_tokens: 100 },
            TokenCounter::new().unwrap(),
        );

        // Use clearly different topics with a deliberately tiny bridging paragraph.
        let paragraphs = [
            "Rust's ownership model ensures that memory is freed exactly once, preventing use-after-free bugs.",
            "The borrow checker tracks references at compile time to guarantee memory safety without a garbage collector.",
            "Notably.", // tiny "bridge" paragraph (~2 tokens)
            "French cuisine is renowned for its rich sauces, intricate techniques, and respect for fresh ingredients.",
            "The five mother sauces — Béchamel, Velouté, Espagnole, Hollandaise, and Tomato — underpin countless French dishes.",
        ];
        let mut chunk = prose_chunk(&paragraphs);
        chunk.needs_refinement = true;

        let result = refiner.refine(chunk, &EMBEDDER).unwrap();

        // Whatever split happened, no output chunk should be below min_tokens=20
        // (the "Notably." fragment must have been merged).
        for (i, seg) in result.iter().enumerate() {
            assert!(
                seg.token_count >= 20 || result.len() == 1,
                "segment {i} has only {} tokens (below min_tokens=20)",
                seg.token_count
            );
        }
    }
}
