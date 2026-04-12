//! Maximal Marginal Relevance (MMR) for diverse result selection.
//!
//! MMR greedily builds a result set that balances relevance against redundancy.
//! At each step the candidate with the highest
//! `λ · rel(c) − (1−λ) · max_{s∈S} sim(c,s)` score is selected, where
//! `rel(c)` is the normalised RRF relevance and `sim` is cosine similarity
//! between chunk embeddings.

use std::collections::HashMap;

use lore_core::{ScoredNode, cosine_similarity};

/// Selects up to `limit` diverse nodes from `candidates` using MMR.
///
/// `embeddings` maps node id → embedding vector.  Nodes without an entry are
/// treated as having zero similarity to everything and are still eligible for
/// selection.
///
/// `lambda = 1.0` → pure relevance ranking (no diversity penalty).
/// `lambda = 0.0` → pure diversity (relevance ignored).
#[must_use]
pub fn select<'e>(
    candidates: Vec<ScoredNode>,
    embeddings: &'e HashMap<i64, Vec<f32>>,
    lambda:     f64,
    limit:      usize,
) -> Vec<ScoredNode> {
    let n = limit.min(candidates.len());
    if n == 0 {
        return vec![];
    }

    // Normalise relevance scores to [0, 1].
    let max_score = candidates
        .first()
        .map_or(1.0, |c| c.score)
        .max(f64::EPSILON);

    let mut remaining = candidates;
    let mut selected: Vec<ScoredNode> = Vec::with_capacity(n);
    // Borrow slices from `embeddings` — avoids cloning on every iteration.
    let mut sel_embs: Vec<&'e [f32]>  = Vec::with_capacity(n);

    while selected.len() < n && !remaining.is_empty() {
        let best_idx = remaining
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let rel     = c.score / max_score;
                let max_sim = max_sim_to_selected(c.node.id, embeddings, &sel_embs);
                let mmr     = lambda.mul_add(rel, -(1.0 - lambda) * max_sim);
                (i, mmr)
            })
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map_or(0, |(i, _)| i);

        let chosen = remaining.swap_remove(best_idx);
        if let Some(emb) = embeddings.get(&chosen.node.id) {
            sel_embs.push(emb.as_slice());
        }
        selected.push(chosen);
    }

    selected
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn max_sim_to_selected(
    node_id:    i64,
    embeddings: &HashMap<i64, Vec<f32>>,
    sel_embs:   &[&[f32]],
) -> f64 {
    if sel_embs.is_empty() {
        return 0.0;
    }
    let Some(emb) = embeddings.get(&node_id) else {
        return 0.0;
    };
    sel_embs
        .iter()
        .map(|s| f64::from(cosine_similarity(emb, s)))
        .fold(f64::NEG_INFINITY, f64::max)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lore_core::{Node, NodeKind, ScoredNode};

    fn fake_node(id: i64) -> Node {
        Node {
            id,
            parent_id:   None,
            path:        id.to_string(),
            doc_id:      1,
            kind:        NodeKind::Chunk,
            level:       None,
            title:       None,
            content:     Some(format!("content {id}")),
            token_count: 20,
            lang:        None,
        }
    }

    fn scored(id: i64, score: f64) -> ScoredNode {
        ScoredNode { node: fake_node(id), score }
    }

    fn unit_vec(v: &[f32]) -> Vec<f32> {
        let norm: f32 = v.iter().map(|&x| x * x).sum::<f32>().sqrt();
        v.iter().map(|&x| x / norm).collect()
    }

    #[test]
    fn pure_relevance_preserves_order() {
        let candidates = vec![scored(1, 3.0), scored(2, 2.0), scored(3, 1.0)];
        let result = select(candidates, &HashMap::new(), 1.0, 3);
        let ids: Vec<i64> = result.iter().map(|n| n.node.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn limit_respected() {
        let candidates = vec![scored(1, 3.0), scored(2, 2.0), scored(3, 1.0)];
        let result = select(candidates, &HashMap::new(), 1.0, 2);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn diversity_deprioritises_redundant_node() {
        // Node 1 and 2 have identical embeddings; node 3 is orthogonal.
        // With high diversity weight (lambda=0.1), whichever of 1/2 is chosen
        // first, the second pick should be node 3 rather than the near-duplicate.
        let emb_a = unit_vec(&[1.0_f32, 0.0, 0.0]);
        let emb_b = unit_vec(&[1.0_f32, 0.0, 0.0]); // identical to a
        let emb_c = unit_vec(&[0.0_f32, 1.0, 0.0]); // orthogonal

        let mut embeddings = HashMap::new();
        embeddings.insert(1i64, emb_a);
        embeddings.insert(2i64, emb_b);
        embeddings.insert(3i64, emb_c);

        let candidates = vec![scored(1, 1.0), scored(2, 1.0), scored(3, 1.0)];
        let result = select(candidates, &embeddings, 0.1, 2);

        let ids: std::collections::HashSet<i64> = result.iter().map(|n| n.node.id).collect();
        assert!(ids.contains(&3), "diverse node must be selected");
        // The identical-embedding pair should not both appear in the top 2.
        assert!(
            !(ids.contains(&1) && ids.contains(&2)),
            "both near-duplicate nodes must not both be selected"
        );
    }
}
