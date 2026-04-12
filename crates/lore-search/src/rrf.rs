//! Reciprocal Rank Fusion: merge multiple ranked lists into one.
//!
//! RRF assigns each candidate a score of `∑ 1 / (K + rank)` across all input
//! lists (1-based rank within each list), then sorts descending.

use std::collections::HashMap;

use lore_core::{Node, ScoredNode};

/// Standard RRF smoothing constant.  Higher values down-weight the importance
/// of top-rank positions; 60 is the widely-used default.
const K: f64 = 60.0;

/// Merges multiple ranked lists into a single list using RRF.
///
/// Empty input lists are ignored.  If the same node appears in multiple lists
/// its RRF score is the sum of per-list contributions.  The output is sorted
/// by descending RRF score.
#[must_use]
pub fn merge(lists: &[Vec<ScoredNode>]) -> Vec<ScoredNode> {
    let mut scores: HashMap<i64, f64> = HashMap::new();
    let mut nodes: HashMap<i64, Node>  = HashMap::new();

    for list in lists {
        for (rank, scored) in list.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let rrf = 1.0 / (K + (rank + 1) as f64);
            *scores.entry(scored.node.id).or_insert(0.0) += rrf;
            nodes.entry(scored.node.id).or_insert_with(|| scored.node.clone());
        }
    }

    let mut result: Vec<ScoredNode> = scores
        .into_iter()
        .filter_map(|(id, score)| nodes.remove(&id).map(|node| ScoredNode { node, score }))
        .collect();

    result.sort_unstable_by(|a, b| {
        b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
    });
    result
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lore_core::{Node, NodeKind};

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
            token_count: 10,
            lang:        None,
        }
    }

    fn scored(id: i64, score: f64) -> ScoredNode {
        ScoredNode { node: fake_node(id), score }
    }

    #[test]
    fn merge_single_list_preserves_order() {
        let list = vec![scored(1, 3.0), scored(2, 2.0), scored(3, 1.0)];
        let result = merge(&[list]);
        let ids: Vec<i64> = result.iter().map(|n| n.node.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn merge_two_lists_boosts_overlap() {
        // Node 1 appears at rank 0 in both lists → higher RRF score than
        // node 2 which appears at rank 1 in list A and not in list B.
        let list_a = vec![scored(1, 1.0), scored(2, 0.5)];
        let list_b = vec![scored(3, 1.0), scored(1, 0.8)];
        let result = merge(&[list_a, list_b]);
        // Node 1 appears twice; it should beat node 3 which appears once at rank 0.
        let top = result[0].node.id;
        assert_eq!(top, 1, "node present in both lists should rank highest");
    }

    #[test]
    fn merge_empty_lists_returns_empty() {
        let result = merge(&[vec![], vec![]]);
        assert!(result.is_empty());
    }

    #[test]
    fn merge_no_lists_returns_empty() {
        let result = merge(&[]);
        assert!(result.is_empty());
    }
}
