//! Resolve [`ScoredNode`]s into full [`SearchResult`]s.
//!
//! Fetches doc titles and heading paths in two batched DB calls rather than
//! per-node round-trips.

use std::collections::HashMap;

use lore_core::{Db, LoreError, ScoredNode, SearchResult};

/// Resolves `nodes` into [`SearchResult`]s.
///
/// Issues at most two database calls regardless of how many nodes are resolved:
/// one for all doc titles and one for all heading paths.
pub async fn resolve(db: &Db, nodes: Vec<ScoredNode>) -> Result<Vec<SearchResult>, LoreError> {
    if nodes.is_empty() {
        return Ok(vec![]);
    }

    let node_ids: Vec<i64> = nodes.iter().map(|n| n.node.id).collect();
    let doc_ids: Vec<i64> = {
        let mut seen = std::collections::HashSet::new();
        nodes.iter().filter_map(|n| seen.insert(n.node.doc_id).then_some(n.node.doc_id)).collect()
    };

    let docs = db.get_docs_by_ids(doc_ids).await?;
    let doc_map: HashMap<i64, lore_core::Doc> = docs.into_iter().map(|d| (d.id, d)).collect();

    let heading_paths = db.get_heading_paths_for_nodes(node_ids).await?;
    let path_map: HashMap<i64, Vec<String>> = heading_paths.into_iter().collect();

    let mut results = Vec::with_capacity(nodes.len());
    for scored in nodes {
        let doc = doc_map.get(&scored.node.doc_id);
        let doc_title = doc
            .and_then(|d| d.title.clone())
            .or_else(|| doc.map(|d| d.path.clone()))
            .unwrap_or_default();
        let heading_path = path_map.get(&scored.node.id).cloned().unwrap_or_default();

        let doc_path = doc.map(|d| d.path.clone()).unwrap_or_default();
        results.push(SearchResult {
            node: scored.node,
            doc_path,
            doc_title,
            heading_path,
            score: scored.score,
        });
    }
    Ok(results)
}
