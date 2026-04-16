//! CRUD operations on `docs`, `nodes`, `node_embeddings`, and `meta` tables.

use rusqlite::params;

use crate::{
    doc::Doc,
    error::LoreError,
    node::{NewNode, Node, NodeKind},
    package::Package,
};

use super::{
    Db, NODE_COLUMNS,
    helpers::{
        ancestor_ids_from_path, bytes_to_f32_vec, f32_slice_to_bytes, fetch_nodes_by_ids,
        node_from_db, node_from_row, placeholders_for,
    },
};

impl Db {
    // -----------------------------------------------------------------------
    // Meta table helpers
    // -----------------------------------------------------------------------

    /// Inserts or replaces a key-value pair in the `meta` table.
    pub async fn set_meta(&self, key: String, value: String) -> Result<(), LoreError> {
        self.conn
            .call(move |db| {
                db.execute(
                    "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                    params![key, value],
                )?;
                Ok(())
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns the value stored under `key`, or `None` if the key is absent.
    pub async fn get_meta(&self, key: String) -> Result<Option<String>, LoreError> {
        self.conn
            .call(move |db| {
                let mut stmt = db.prepare_cached("SELECT value FROM meta WHERE key = ?1")?;
                let mut rows = stmt.query(params![key])?;
                rows.next()?.map(|row| row.get::<_, String>(0)).transpose()
            })
            .await
            .map_err(LoreError::from)
    }

    // -----------------------------------------------------------------------
    // docs table
    // -----------------------------------------------------------------------

    /// Inserts a documentation file record and returns the new `id`.
    ///
    /// If a record with the same `path` already exists the existing `id` is
    /// returned (the title is not updated).
    pub async fn insert_doc(&self, path: String, title: Option<String>) -> Result<i64, LoreError> {
        self.conn
            .call(move |db| {
                // INSERT OR IGNORE returns last_insert_rowid() = 0 on conflict,
                // so a follow-up SELECT is required in both the insert and
                // the already-exists cases.
                db.execute(
                    "INSERT OR IGNORE INTO docs (path, title) VALUES (?1, ?2)",
                    params![path, title],
                )?;
                let mut stmt = db.prepare_cached("SELECT id FROM docs WHERE path = ?1")?;
                stmt.query_row(params![path], |row| row.get(0))
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns the [`Doc`] with the given `id`.
    pub async fn get_doc(&self, id: i64) -> Result<Doc, LoreError> {
        self.conn
            .call(move |db| {
                db.query_row("SELECT id, path, title FROM docs WHERE id = ?1", params![id], |row| {
                    Ok(Doc { id: row.get(0)?, path: row.get(1)?, title: row.get(2)? })
                })
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns the [`Doc`] with the given `path`, or `None` if not found.
    pub async fn get_doc_by_path(&self, path: String) -> Result<Option<Doc>, LoreError> {
        self.conn
            .call(move |db| {
                let mut stmt =
                    db.prepare_cached("SELECT id, path, title FROM docs WHERE path = ?1")?;
                let mut rows = stmt.query(params![path])?;
                rows.next()?
                    .map(|row| Ok(Doc { id: row.get(0)?, path: row.get(1)?, title: row.get(2)? }))
                    .transpose()
            })
            .await
            .map_err(LoreError::from)
    }

    // -----------------------------------------------------------------------
    // nodes table
    // -----------------------------------------------------------------------

    /// Inserts a new node and returns its assigned `id`.
    ///
    /// The `path` column is computed from the parent's path after the row is
    /// inserted so that it encodes the new node's own `id`.
    pub async fn insert_node(&self, new_node: NewNode) -> Result<i64, LoreError> {
        self.conn
            .call(move |db| {
                let parent_path: Option<String> = match new_node.parent_id {
                    Some(pid) => Some(db.query_row(
                        "SELECT path FROM nodes WHERE id = ?1",
                        params![pid],
                        |row| row.get(0),
                    )?),
                    None => None,
                };

                db.execute(
                    "INSERT INTO nodes
                        (parent_id, path, doc_id, kind, level, title, content,
                         token_count, lang)
                     VALUES (?1, '', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        new_node.parent_id,
                        new_node.doc_id,
                        new_node.kind.as_str(),
                        new_node.level,
                        new_node.title,
                        new_node.content,
                        new_node.token_count,
                        new_node.lang,
                    ],
                )?;
                let id = db.last_insert_rowid();

                // Path is built after insert because it must encode the node's
                // own id, which is only known after last_insert_rowid().
                let path = parent_path.map_or_else(|| id.to_string(), |pp| format!("{pp}/{id}"));
                db.execute("UPDATE nodes SET path = ?1 WHERE id = ?2", params![path, id])?;

                Ok(id)
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns the [`Node`] with the given `id`.
    pub async fn get_node(&self, id: i64) -> Result<Node, LoreError> {
        self.conn.call(move |db| node_from_db(db, id)).await.map_err(LoreError::from)
    }

    /// Returns all direct children of `parent_id`, ordered by `id`.
    pub async fn get_children(&self, parent_id: i64) -> Result<Vec<Node>, LoreError> {
        self.conn
            .call(move |db| {
                let mut stmt = db.prepare(&format!(
                    "SELECT {NODE_COLUMNS} FROM nodes WHERE parent_id = ?1 ORDER BY id"
                ))?;
                stmt.query_map(params![parent_id], node_from_row)?.collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns all ancestor nodes of `node_id` ordered from the root down to
    /// the immediate parent.
    ///
    /// Uses the path string to derive ancestor ids: for path `"1/4/9/23"` the
    /// ancestors are nodes `1`, `4`, and `9`.
    pub async fn get_ancestors(&self, node_id: i64) -> Result<Vec<Node>, LoreError> {
        self.conn
            .call(move |db| {
                let path: String = db.query_row(
                    "SELECT path FROM nodes WHERE id = ?1",
                    params![node_id],
                    |row| row.get(0),
                )?;
                let ancestor_ids = ancestor_ids_from_path(&path, node_id);
                fetch_nodes_by_ids(db, &ancestor_ids)
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns the titles of all heading ancestors of `node_id`, ordered from
    /// root to nearest ancestor.
    ///
    /// Non-heading nodes in the ancestor chain are skipped — only nodes with
    /// `kind = 'heading'` contribute to the breadcrumb.
    pub async fn get_heading_path(&self, node_id: i64) -> Result<Vec<String>, LoreError> {
        self.conn
            .call(move |db| {
                let path: String = db.query_row(
                    "SELECT path FROM nodes WHERE id = ?1",
                    params![node_id],
                    |row| row.get(0),
                )?;
                let ancestor_ids = ancestor_ids_from_path(&path, node_id);
                if ancestor_ids.is_empty() {
                    return Ok(vec![]);
                }

                let placeholders = placeholders_for(&ancestor_ids);
                let sql = format!(
                    "SELECT id, title FROM nodes WHERE id IN ({placeholders}) AND kind = '{}' ORDER BY id",
                    NodeKind::Heading.as_str(),
                );
                let mut stmt = db.prepare(&sql)?;
                let pairs: Vec<(i64, Option<String>)> =
                    stmt.query_map(rusqlite::params_from_iter(&ancestor_ids), |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?;

                // Re-order to match ancestor_ids (root-first) and drop None titles.
                let id_to_title: std::collections::HashMap<i64, Option<String>> =
                    pairs.into_iter().collect();
                let titles = ancestor_ids
                    .iter()
                    .filter_map(|id| id_to_title.get(id)?.as_deref().map(str::to_owned))
                    .collect();
                Ok(titles)
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns all nodes belonging to `doc_id`, ordered by insertion order.
    pub async fn get_nodes_for_doc(&self, doc_id: i64) -> Result<Vec<Node>, LoreError> {
        self.conn
            .call(move |db| {
                let mut stmt = db.prepare(&format!(
                    "SELECT {NODE_COLUMNS} FROM nodes WHERE doc_id = ?1 ORDER BY id"
                ))?;
                stmt.query_map(params![doc_id], node_from_row)?.collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(LoreError::from)
    }

    // -----------------------------------------------------------------------
    // node_embeddings table
    // -----------------------------------------------------------------------

    /// Stores a 384-dimensional embedding for a node.
    ///
    /// The `node_id` is used as the `rowid` in the `node_embeddings` table,
    /// so it must already exist in `nodes`.
    pub async fn insert_embedding(
        &self,
        node_id: i64,
        embedding: Vec<f32>,
    ) -> Result<(), LoreError> {
        self.conn
            .call(move |db| {
                let blob = f32_slice_to_bytes(&embedding);
                db.execute(
                    "INSERT OR REPLACE INTO node_embeddings(rowid, embedding)
                     VALUES (?1, ?2)",
                    params![node_id, blob],
                )?;
                Ok(())
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns the embedding stored for `node_id`, or `None` if absent.
    pub async fn get_embedding(&self, node_id: i64) -> Result<Option<Vec<f32>>, LoreError> {
        self.conn
            .call(move |db| {
                let mut stmt =
                    db.prepare_cached("SELECT embedding FROM node_embeddings WHERE rowid = ?1")?;
                let mut rows = stmt.query(params![node_id])?;
                rows.next()?
                    .map(|row| -> rusqlite::Result<Vec<f32>> {
                        let blob: Vec<u8> = row.get(0)?;
                        Ok(bytes_to_f32_vec(&blob))
                    })
                    .transpose()
            })
            .await
            .map_err(LoreError::from)
    }

    // -----------------------------------------------------------------------
    // Package metadata helpers
    // -----------------------------------------------------------------------

    /// Reads all package-level metadata keys from the `meta` table and
    /// assembles a [`Package`].
    ///
    /// Returns an error if any of the required keys (`name`, `registry`,
    /// `version`) are missing.
    pub async fn get_package_meta(&self) -> Result<Package, LoreError> {
        self.conn
            .call(|db| {
                let get = |key: &str| -> rusqlite::Result<Option<String>> {
                    let mut stmt = db.prepare_cached("SELECT value FROM meta WHERE key = ?1")?;
                    let mut rows = stmt.query(params![key])?;
                    rows.next()?.map(|r| r.get(0)).transpose()
                };
                let required =
                    |opt: Option<String>| opt.ok_or(rusqlite::Error::QueryReturnedNoRows);

                Ok(Package {
                    name: required(get("name")?)?,
                    registry: required(get("registry")?)?,
                    version: required(get("version")?)?,
                    description: get("description")?,
                    source_url: get("source_url")?,
                    git_sha: get("git_sha")?,
                })
            })
            .await
            .map_err(LoreError::from)
    }

    // -----------------------------------------------------------------------
    // FTS5 maintenance
    // -----------------------------------------------------------------------

    /// Rebuilds the FTS5 index from the current contents of the `nodes` table.
    ///
    /// Should be called once after a bulk insert phase completes.
    pub async fn rebuild_fts(&self) -> Result<(), LoreError> {
        self.conn
            .call(|db| db.execute_batch("INSERT INTO nodes_fts(nodes_fts) VALUES('rebuild');"))
            .await
            .map_err(LoreError::from)
    }

    /// Deletes all nodes and their embeddings for a given `doc_id`.
    ///
    /// This is used before re-indexing a document so that a rebuild does not
    /// duplicate nodes.  The doc record itself is kept (caller may delete it
    /// separately with [`Db::delete_doc`]).
    pub async fn delete_nodes_for_doc(&self, doc_id: i64) -> Result<(), LoreError> {
        self.conn
            .call(move |db| {
                // Delete embeddings for all nodes belonging to this doc.
                db.execute(
                    "DELETE FROM node_embeddings WHERE rowid IN (SELECT id FROM nodes WHERE doc_id = ?1)",
                    params![doc_id],
                )?;
                // Delete the nodes themselves (triggers update FTS).
                db.execute("DELETE FROM nodes WHERE doc_id = ?1", params![doc_id])?;
                Ok(())
            })
            .await
            .map_err(LoreError::from)
    }

    // -----------------------------------------------------------------------
    // Savepoints
    // -----------------------------------------------------------------------

    /// Creates a named `SAVEPOINT`, allowing a logical unit of work to be
    /// rolled back atomically without affecting any outer transaction.
    ///
    /// Pair with [`Db::release_savepoint`] on success or
    /// [`Db::rollback_savepoint`] on failure.
    pub async fn begin_savepoint(&self, name: String) -> Result<(), LoreError> {
        self.conn
            .call(move |db| db.execute_batch(&format!("SAVEPOINT \"{name}\"")))
            .await
            .map_err(LoreError::from)
    }

    /// Commits all work done since [`Db::begin_savepoint`] was called with
    /// `name`, making the changes permanent (or visible to any outer
    /// transaction).
    pub async fn release_savepoint(&self, name: String) -> Result<(), LoreError> {
        self.conn
            .call(move |db| db.execute_batch(&format!("RELEASE \"{name}\"")))
            .await
            .map_err(LoreError::from)
    }

    /// Rolls back all work done since [`Db::begin_savepoint`] was called with
    /// `name`, then releases the savepoint so it can be reused.
    pub async fn rollback_savepoint(&self, name: String) -> Result<(), LoreError> {
        self.conn
            .call(move |db| {
                db.execute_batch(&format!("ROLLBACK TO SAVEPOINT \"{name}\"; RELEASE \"{name}\""))
            })
            .await
            .map_err(LoreError::from)
    }

    // -----------------------------------------------------------------------
    // Maintenance
    // -----------------------------------------------------------------------

    /// Runs `PRAGMA optimize` and `VACUUM` to compact and tune the database
    /// after a build has finished.
    pub async fn optimize(&self) -> Result<(), LoreError> {
        self.conn
            .call(|db| db.execute_batch("PRAGMA optimize; VACUUM;"))
            .await
            .map_err(LoreError::from)
    }
}
