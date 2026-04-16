//! Token-budget enforcement: truncate results once the cumulative token count
//! would exceed the configured ceiling.

use lore_core::ScoredNode;

/// Returns the longest prefix of `nodes` whose cumulative token count stays
/// within `budget`.
///
/// If the very first node already exceeds `budget` it is still included so
/// that the caller always receives at least one result (never an empty set
/// due to budget alone).
#[must_use]
pub fn apply(nodes: Vec<ScoredNode>, budget: u32) -> Vec<ScoredNode> {
    let mut total: u32 = 0;
    let mut out = Vec::with_capacity(nodes.len());

    for node in nodes {
        let tokens = node.node.token_count;
        let would_exceed = total.saturating_add(tokens) > budget;
        if would_exceed && !out.is_empty() {
            break;
        }
        total = total.saturating_add(tokens);
        out.push(node);
    }

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lore_core::{Node, NodeKind, ScoredNode};

    fn node_with_tokens(id: i64, token_count: u32) -> ScoredNode {
        ScoredNode {
            node: Node {
                id,
                parent_id: None,
                path: id.to_string(),
                doc_id: 1,
                kind: NodeKind::Chunk,
                level: None,
                title: None,
                content: None,
                token_count,
                lang: None,
            },
            score: 1.0,
        }
    }

    #[test]
    fn budget_limits_results() {
        let nodes =
            vec![node_with_tokens(1, 100), node_with_tokens(2, 100), node_with_tokens(3, 100)];
        let result = apply(nodes, 250);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn budget_includes_at_least_one_even_if_oversized() {
        let nodes = vec![node_with_tokens(1, 5000), node_with_tokens(2, 100)];
        let result = apply(nodes, 100);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].node.id, 1);
    }

    #[test]
    fn budget_allows_all_when_under_limit() {
        let nodes = vec![node_with_tokens(1, 50), node_with_tokens(2, 50), node_with_tokens(3, 50)];
        let result = apply(nodes, 200);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn empty_input_returns_empty() {
        let result = apply(vec![], 1000);
        assert!(result.is_empty());
    }
}
