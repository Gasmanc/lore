//! Private query-building and row-mapping utilities for the `db` module.
//!
//! All items are `pub(super)` — visible within the `db` module (and its
//! submodules) but not outside it.

use crate::node::{Node, NodeKind};

use super::NODE_COLUMNS;

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Extracts the ancestor node ids from a path string, excluding the node
/// itself.
///
/// For path `"1/4/9/23"` this returns `[1, 4, 9]`.
pub(super) fn ancestor_ids_from_path(path: &str, self_id: i64) -> Vec<i64> {
    path.split('/').filter_map(|s| s.parse::<i64>().ok()).filter(|&id| id != self_id).collect()
}

/// Builds a comma-separated placeholder string `?1, ?2, …, ?N` for use in
/// `WHERE id IN (…)` queries.
pub(super) fn placeholders_for(ids: &[i64]) -> String {
    (1..=ids.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ")
}

// ---------------------------------------------------------------------------
// Node fetching
// ---------------------------------------------------------------------------

/// Fetches a batch of nodes by `ids` in a single `WHERE id IN (…)` query,
/// returning them in the same order as the input slice.
pub(super) fn fetch_nodes_by_ids(
    db: &rusqlite::Connection,
    ids: &[i64],
) -> rusqlite::Result<Vec<Node>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let sql = format!("SELECT {NODE_COLUMNS} FROM nodes WHERE id IN ({})", placeholders_for(ids),);
    let mut stmt = db.prepare(&sql)?;
    let mut nodes: Vec<Node> = stmt
        .query_map(rusqlite::params_from_iter(ids.iter()), node_from_row)?
        .collect::<rusqlite::Result<_>>()?;

    // Re-order to match the caller's requested id order (rusqlite does not
    // guarantee a particular row order for IN queries).
    let order: std::collections::HashMap<i64, usize> =
        ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    nodes.sort_unstable_by_key(|n| order.get(&n.id).copied().unwrap_or(usize::MAX));
    Ok(nodes)
}

/// Fetches a single [`Node`] by `id` from the provided synchronous connection.
pub(super) fn node_from_db(db: &rusqlite::Connection, id: i64) -> rusqlite::Result<Node> {
    db.query_row(
        &format!("SELECT {NODE_COLUMNS} FROM nodes WHERE id = ?1"),
        rusqlite::params![id],
        node_from_row,
    )
}

// ---------------------------------------------------------------------------
// Row deserialization
// ---------------------------------------------------------------------------

/// Constructs a [`Node`] from a `rusqlite::Row` with the columns defined by
/// [`NODE_COLUMNS`]: `id, parent_id, path, doc_id, kind, level, title,
/// content, token_count, lang`.
pub(super) fn node_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    let kind_str: String = row.get(4)?;
    let kind = NodeKind::try_from(kind_str.as_str()).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::fmt::Error),
        )
    })?;
    Ok(Node {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        path: row.get(2)?,
        doc_id: row.get(3)?,
        kind,
        level: row.get(5)?,
        title: row.get(6)?,
        content: row.get(7)?,
        token_count: row.get::<_, u32>(8)?,
        lang: row.get(9)?,
    })
}

// ---------------------------------------------------------------------------
// Embedding serialization
// ---------------------------------------------------------------------------

/// Serialises a slice of `f32` values to a little-endian byte vector
/// compatible with `sqlite-vec`'s BLOB encoding.
pub(super) fn f32_slice_to_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for &v in values {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

/// Deserialises a little-endian byte slice produced by [`f32_slice_to_bytes`]
/// back into a `Vec<f32>`.
pub(super) fn bytes_to_f32_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}
