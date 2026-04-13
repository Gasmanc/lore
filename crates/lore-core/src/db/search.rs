//! FTS5 full-text search, vector KNN search, and batch resolve helpers.
//!
//! These methods are called by `lore-search` during query execution.

use rusqlite::params;

use crate::{
    doc::Doc,
    error::LoreError,
    node::NodeKind,
    search::ScoredNode,
};

use super::{
    Db, NODE_COLUMNS_ALIASED,
    helpers::{
        ancestor_ids_from_path, bytes_to_f32_vec, f32_slice_to_bytes, fetch_nodes_by_ids,
        node_from_row, placeholders_for,
    },
};

impl Db {
    // -----------------------------------------------------------------------
    // Batch helpers for resolve
    // -----------------------------------------------------------------------

    /// Returns all [`Doc`]s whose ids are in `ids`, in the same order.
    pub async fn get_docs_by_ids(&self, ids: Vec<i64>) -> Result<Vec<Doc>, LoreError> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        self.conn
            .call(move |db| {
                let sql = format!(
                    "SELECT id, path, title FROM docs WHERE id IN ({})",
                    placeholders_for(&ids)
                );
                let mut stmt = db.prepare(&sql)?;
                let order: std::collections::HashMap<i64, usize> =
                    ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
                let mut docs: Vec<Doc> = stmt
                    .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                        Ok(Doc { id: row.get(0)?, path: row.get(1)?, title: row.get(2)? })
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                docs.sort_unstable_by_key(|d| order.get(&d.id).copied().unwrap_or(usize::MAX));
                Ok(docs)
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns heading breadcrumb paths for multiple nodes in one connection
    /// call.  The returned `Vec` is in the same order as `node_ids`.
    ///
    /// Each entry is `(node_id, heading_path)`.
    pub async fn get_heading_paths_for_nodes(
        &self,
        node_ids: Vec<i64>,
    ) -> Result<Vec<(i64, Vec<String>)>, LoreError> {
        if node_ids.is_empty() {
            return Ok(vec![]);
        }
        self.conn
            .call(move |db| {
                // Fetch path strings for all nodes in one query.
                let sql = format!(
                    "SELECT id, path FROM nodes WHERE id IN ({})",
                    placeholders_for(&node_ids)
                );
                let mut stmt = db.prepare(&sql)?;
                let id_to_path: std::collections::HashMap<i64, String> = stmt
                    .query_map(rusqlite::params_from_iter(node_ids.iter()), |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?;

                // For each node, resolve heading path from its ancestor ids.
                let mut result = Vec::with_capacity(node_ids.len());
                for &node_id in &node_ids {
                    let path_str = id_to_path.get(&node_id).map_or("", String::as_str);
                    let ancestor_ids = ancestor_ids_from_path(path_str, node_id);
                    let heading_path = if ancestor_ids.is_empty() {
                        vec![]
                    } else {
                        let ph = placeholders_for(&ancestor_ids);
                        let hsql = format!(
                            "SELECT id, title FROM nodes WHERE id IN ({ph}) AND kind = '{}' ORDER BY id",
                            NodeKind::Heading.as_str(),
                        );
                        let mut hstmt = db.prepare(&hsql)?;
                        let pairs: Vec<(i64, Option<String>)> = hstmt
                            .query_map(rusqlite::params_from_iter(&ancestor_ids), |row| {
                                Ok((row.get(0)?, row.get(1)?))
                            })?
                            .collect::<rusqlite::Result<_>>()?;
                        let id_to_title: std::collections::HashMap<i64, Option<String>> =
                            pairs.into_iter().collect();
                        ancestor_ids
                            .iter()
                            .filter_map(|id| id_to_title.get(id)?.as_deref().map(str::to_owned))
                            .collect()
                    };
                    result.push((node_id, heading_path));
                }
                Ok(result)
            })
            .await
            .map_err(LoreError::from)
    }

    // -----------------------------------------------------------------------
    // Full-text search
    // -----------------------------------------------------------------------

    /// Executes a BM25 full-text search and returns up to `limit` nodes with
    /// their BM25 relevance scores (higher = more relevant).
    ///
    /// `query` must be a valid FTS5 query string.  An empty query returns an
    /// empty result set.
    pub async fn fts_search(
        &self,
        query: String,
        limit: usize,
    ) -> Result<Vec<ScoredNode>, LoreError> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        self.conn
            .call(move |db| {
                let sql = format!(
                    "SELECT {NODE_COLUMNS_ALIASED}, -bm25(nodes_fts) AS score
                     FROM nodes_fts
                     JOIN nodes n ON n.id = nodes_fts.rowid
                     WHERE nodes_fts MATCH ?1
                     ORDER BY bm25(nodes_fts)
                     LIMIT ?2"
                );
                let mut stmt = db.prepare_cached(&sql)?;
                #[allow(clippy::cast_possible_wrap)]
                let limit_i64 = limit as i64;
                stmt.query_map(params![query, limit_i64], |row| {
                    let node = node_from_row(row)?;
                    let score: f64 = row.get(10)?;
                    Ok(ScoredNode { node, score })
                })?
                .collect::<rusqlite::Result<_>>()
            })
            .await
            .map_err(LoreError::from)
    }

    // -----------------------------------------------------------------------
    // Vector KNN search
    // -----------------------------------------------------------------------

    /// Executes a KNN vector search and returns up to `limit` nodes ordered
    /// by cosine similarity to `query_embedding` (higher = more similar).
    pub async fn vec_search(
        &self,
        query_embedding: Vec<f32>,
        limit: usize,
    ) -> Result<Vec<ScoredNode>, LoreError> {
        self.conn
            .call(move |db| {
                let blob = f32_slice_to_bytes(&query_embedding);
                let mut stmt = db.prepare_cached(
                    "SELECT ne.rowid, ne.distance
                     FROM node_embeddings ne
                     WHERE ne.embedding MATCH ?1 AND k = ?2
                     ORDER BY ne.distance",
                )?;
                #[allow(clippy::cast_possible_wrap)]
                let limit_i64 = limit as i64;
                let pairs: Vec<(i64, f64)> = stmt
                    .query_map(params![blob, limit_i64], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?;

                if pairs.is_empty() {
                    return Ok(vec![]);
                }

                // Resolve node ids → Node structs.
                let id_order: std::collections::HashMap<i64, usize> =
                    pairs.iter().enumerate().map(|(i, &(id, _))| (id, i)).collect();
                let ids: Vec<i64> = pairs.iter().map(|&(id, _)| id).collect();
                let mut nodes = fetch_nodes_by_ids(db, &ids)?;
                nodes.sort_unstable_by_key(|n| id_order.get(&n.id).copied().unwrap_or(usize::MAX));

                // Convert cosine distance → similarity: 1.0 - distance (range [-1, 1]).
                Ok(nodes
                    .into_iter()
                    .zip(pairs.iter().map(|&(_, d)| d))
                    .map(|(node, distance)| ScoredNode { node, score: 1.0 - distance })
                    .collect())
            })
            .await
            .map_err(LoreError::from)
    }

    /// Fetches embeddings for a batch of node ids in one connection call.
    ///
    /// Returns `(node_id, embedding)` pairs for nodes that have embeddings;
    /// nodes without embeddings are silently omitted.
    pub async fn get_embeddings_for_nodes(
        &self,
        node_ids: Vec<i64>,
    ) -> Result<Vec<(i64, Vec<f32>)>, LoreError> {
        if node_ids.is_empty() {
            return Ok(vec![]);
        }
        self.conn
            .call(move |db| {
                let mut stmt =
                    db.prepare_cached("SELECT embedding FROM node_embeddings WHERE rowid = ?1")?;
                let mut result = Vec::with_capacity(node_ids.len());
                for id in &node_ids {
                    let mut rows = stmt.query(params![id])?;
                    if let Some(row) = rows.next()? {
                        let blob: Vec<u8> = row.get(0)?;
                        result.push((*id, bytes_to_f32_vec(&blob)));
                    }
                }
                Ok(result)
            })
            .await
            .map_err(LoreError::from)
    }
}
