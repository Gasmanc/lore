//! Database connection management and CRUD operations.
//!
//! [`Db`] wraps a `tokio_rusqlite::Connection` and provides an async interface
//! over a single `SQLite` `.db` file.  All operations run on a dedicated
//! background thread (managed by `tokio_rusqlite`), keeping the async runtime
//! free.
//!
//! # Extension loading
//!
//! `sqlite-vec` is registered as an auto-extension via
//! [`rusqlite::ffi::sqlite3_auto_extension`] exactly once per process.  Every
//! subsequent connection opened by rusqlite — including those opened through
//! `tokio_rusqlite` — will therefore automatically have the `vec0` virtual
//! table module available.

use std::path::Path;
use std::sync::OnceLock;

use rusqlite::params;
use tracing::{debug, instrument};

use crate::{
    doc::Doc,
    error::LoreError,
    node::{NewNode, Node, NodeKind},
    package::Package,
};

// ---------------------------------------------------------------------------
// sqlite-vec auto-extension bootstrap
// ---------------------------------------------------------------------------

static VEC_EXTENSION_INIT: OnceLock<()> = OnceLock::new();

/// Registers the `sqlite-vec` extension as a global `SQLite` auto-extension.
///
/// This function is idempotent: the registration happens at most once per
/// process regardless of how many connections are opened.
///
/// # Safety
///
/// `sqlite3_auto_extension` is an FFI call into `SQLite`'s C library.  The
/// function pointer transmutation is the documented pattern for registering
/// `SQLite` extensions from Rust (mirrors the approach used in the `sqlite-vec`
/// crate's own test suite).
fn ensure_vec_extension_registered() {
    VEC_EXTENSION_INIT.get_or_init(|| {
        // SAFETY: `sqlite3_auto_extension` expects a pointer to a function
        // with the `SQLite` extension entry-point signature
        // `fn(*mut sqlite3, *mut *mut c_char, *const sqlite3_api_routines) -> c_int`.
        // `sqlite_vec::sqlite3_vec_init` has exactly that signature at the ABI
        // level (it is a standard `SQLite` extension), so the transmute is sound.
        unsafe {
            type EntryPoint = unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::ffi::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::ffi::c_int;

            rusqlite::ffi::sqlite3_auto_extension(Some(
                std::mem::transmute::<*const (), EntryPoint>(
                    sqlite_vec::sqlite3_vec_init as *const (),
                ),
            ));
        }
        debug!("sqlite-vec auto-extension registered");
    });
}

// ---------------------------------------------------------------------------
// Schema migrations
// ---------------------------------------------------------------------------

/// Each element is the SQL that implements one incremental migration.
/// Migrations are applied in order; the current version is stored in the
/// `meta` table under the key `"schema_version"`.
const MIGRATIONS: &[&str] = &[
    // Migration 1 — core tables and indexes.
    "CREATE TABLE IF NOT EXISTS docs (
        id    INTEGER PRIMARY KEY AUTOINCREMENT,
        path  TEXT NOT NULL,
        title TEXT,
        UNIQUE(path)
    );

    CREATE TABLE IF NOT EXISTS nodes (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        parent_id   INTEGER REFERENCES nodes(id),
        path        TEXT NOT NULL DEFAULT '',
        doc_id      INTEGER NOT NULL REFERENCES docs(id),
        kind        TEXT NOT NULL CHECK(kind IN ('heading','chunk','code_block')),
        level       INTEGER CHECK(level IS NULL OR (level >= 1 AND level <= 6)),
        title       TEXT,
        content     TEXT,
        token_count INTEGER NOT NULL DEFAULT 0,
        lang        TEXT
    );

    CREATE INDEX IF NOT EXISTS nodes_path   ON nodes(path);
    CREATE INDEX IF NOT EXISTS nodes_doc    ON nodes(doc_id);
    CREATE INDEX IF NOT EXISTS nodes_parent ON nodes(parent_id);",
    // Migration 2 — FTS5 virtual table.
    "CREATE VIRTUAL TABLE IF NOT EXISTS nodes_fts USING fts5(
        title,
        content,
        content='nodes',
        content_rowid='id',
        tokenize='porter unicode61'
    );",
    // Migration 3 — FTS5 content-table sync triggers.
    "CREATE TRIGGER IF NOT EXISTS nodes_ai AFTER INSERT ON nodes BEGIN
        INSERT INTO nodes_fts(rowid, title, content)
        VALUES (new.id, new.title, new.content);
    END;

    CREATE TRIGGER IF NOT EXISTS nodes_ad AFTER DELETE ON nodes BEGIN
        INSERT INTO nodes_fts(nodes_fts, rowid, title, content)
        VALUES ('delete', old.id, old.title, old.content);
    END;

    CREATE TRIGGER IF NOT EXISTS nodes_au AFTER UPDATE ON nodes BEGIN
        INSERT INTO nodes_fts(nodes_fts, rowid, title, content)
        VALUES ('delete', old.id, old.title, old.content);
        INSERT INTO nodes_fts(rowid, title, content)
        VALUES (new.id, new.title, new.content);
    END;",
    // Migration 4 — sqlite-vec embedding table (384-dim = bge-small-en-v1.5).
    "CREATE VIRTUAL TABLE IF NOT EXISTS node_embeddings USING vec0(
        embedding FLOAT[384]
    );",
];

// ---------------------------------------------------------------------------
// Node column list
// ---------------------------------------------------------------------------

/// The canonical SELECT column list for the `nodes` table used by
/// [`node_from_row`].  Both query sites reference this constant so the column
/// order can never silently diverge.
const NODE_COLUMNS: &str =
    "id, parent_id, path, doc_id, kind, level, title, content, token_count, lang";

// ---------------------------------------------------------------------------
// Db
// ---------------------------------------------------------------------------

/// Async wrapper around a single `SQLite` database file used by Lore.
///
/// Clone this handle freely — all clones share the same underlying connection
/// thread.
#[derive(Clone, Debug)]
pub struct Db {
    conn: tokio_rusqlite::Connection,
}

impl Db {
    /// Opens (or creates) the database at `path`, registers the `sqlite-vec`
    /// extension, sets recommended PRAGMAs, and runs any pending migrations.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, LoreError> {
        ensure_vec_extension_registered();
        let conn = tokio_rusqlite::Connection::open(path.as_ref())
            .await
            .map_err(LoreError::Database)?;
        Self::init(conn).await
    }

    /// Opens an in-memory database.  Useful for tests.
    pub async fn open_in_memory() -> Result<Self, LoreError> {
        ensure_vec_extension_registered();
        let conn = tokio_rusqlite::Connection::open_in_memory()
            .await
            .map_err(LoreError::Database)?;
        Self::init(conn).await
    }

    /// Applies PRAGMAs and runs migrations on a freshly opened connection.
    async fn init(conn: tokio_rusqlite::Connection) -> Result<Self, LoreError> {
        let db = Self { conn };
        db.configure().await?;
        db.run_migrations().await?;
        Ok(db)
    }

    /// Applies connection-level PRAGMAs that improve performance and safety.
    async fn configure(&self) -> Result<(), LoreError> {
        self.conn
            .call(|db| {
                db.execute_batch(
                    "PRAGMA journal_mode = WAL;
                     PRAGMA synchronous  = NORMAL;
                     PRAGMA foreign_keys = ON;
                     PRAGMA temp_store   = MEMORY;",
                )
            })
            .await
            .map_err(LoreError::from)
    }

    /// Applies all pending schema migrations in order.
    async fn run_migrations(&self) -> Result<(), LoreError> {
        self.conn
            .call(|db| {
                // Ensure the meta table exists before we read the version from it.
                db.execute_batch(
                    "CREATE TABLE IF NOT EXISTS meta (
                        key   TEXT PRIMARY KEY,
                        value TEXT NOT NULL
                    );",
                )?;

                let version: u32 = db
                    .query_row(
                        "SELECT COALESCE(
                            (SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'schema_version'),
                            0
                        )",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);

                for (idx, migration_sql) in MIGRATIONS.iter().enumerate() {
                    // `MIGRATIONS` will never have 2^32 entries; the cast is safe.
                    #[allow(clippy::cast_possible_truncation)]
                    let migration_version = (idx + 1) as u32;
                    if migration_version <= version {
                        continue;
                    }

                    db.execute_batch(migration_sql)?;
                    db.execute(
                        "INSERT OR REPLACE INTO meta (key, value)
                         VALUES ('schema_version', ?1)",
                        params![migration_version],
                    )?;
                }

                Ok(())
            })
            .await
            .map_err(LoreError::from)
    }

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
                let mut stmt =
                    db.prepare_cached("SELECT value FROM meta WHERE key = ?1")?;
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
    pub async fn insert_doc(
        &self,
        path: String,
        title: Option<String>,
    ) -> Result<i64, LoreError> {
        self.conn
            .call(move |db| {
                // INSERT OR IGNORE returns last_insert_rowid() = 0 on conflict,
                // so a follow-up SELECT is required in both the insert and
                // the already-exists cases.
                db.execute(
                    "INSERT OR IGNORE INTO docs (path, title) VALUES (?1, ?2)",
                    params![path, title],
                )?;
                let mut stmt =
                    db.prepare_cached("SELECT id FROM docs WHERE path = ?1")?;
                stmt.query_row(params![path], |row| row.get(0))
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns the [`Doc`] with the given `id`.
    pub async fn get_doc(&self, id: i64) -> Result<Doc, LoreError> {
        self.conn
            .call(move |db| {
                db.query_row(
                    "SELECT id, path, title FROM docs WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok(Doc {
                            id: row.get(0)?,
                            path: row.get(1)?,
                            title: row.get(2)?,
                        })
                    },
                )
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
                let path = parent_path
                    .map_or_else(|| id.to_string(), |pp| format!("{pp}/{id}"));
                db.execute(
                    "UPDATE nodes SET path = ?1 WHERE id = ?2",
                    params![path, id],
                )?;

                Ok(id)
            })
            .await
            .map_err(LoreError::from)
    }

    /// Returns the [`Node`] with the given `id`.
    pub async fn get_node(&self, id: i64) -> Result<Node, LoreError> {
        self.conn
            .call(move |db| node_from_db(db, id))
            .await
            .map_err(LoreError::from)
    }

    /// Returns all direct children of `parent_id`, ordered by `id`.
    pub async fn get_children(&self, parent_id: i64) -> Result<Vec<Node>, LoreError> {
        self.conn
            .call(move |db| {
                let mut stmt = db.prepare(
                    &format!("SELECT {NODE_COLUMNS} FROM nodes WHERE parent_id = ?1 ORDER BY id"),
                )?;
                stmt.query_map(params![parent_id], node_from_row)?
                    .collect::<Result<Vec<_>, _>>()
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
                let mut stmt = db.prepare_cached(
                    "SELECT embedding FROM node_embeddings WHERE rowid = ?1",
                )?;
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
                    let mut stmt =
                        db.prepare_cached("SELECT value FROM meta WHERE key = ?1")?;
                    let mut rows = stmt.query(params![key])?;
                    rows.next()?.map(|r| r.get(0)).transpose()
                };
                let required = |opt: Option<String>| {
                    opt.ok_or(rusqlite::Error::QueryReturnedNoRows)
                };

                Ok(Package {
                    name:        required(get("name")?)?,
                    registry:    required(get("registry")?)?,
                    version:     required(get("version")?)?,
                    description: get("description")?,
                    source_url:  get("source_url")?,
                    git_sha:     get("git_sha")?,
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

    /// Runs `PRAGMA optimize` and `VACUUM` to compact and tune the database
    /// after a build has finished.
    pub async fn optimize(&self) -> Result<(), LoreError> {
        self.conn
            .call(|db| db.execute_batch("PRAGMA optimize; VACUUM;"))
            .await
            .map_err(LoreError::from)
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Extracts the ancestor node ids from a path string, excluding the node
/// itself.
///
/// For path `"1/4/9/23"` this returns `[1, 4, 9]`.
fn ancestor_ids_from_path(path: &str, self_id: i64) -> Vec<i64> {
    path.split('/')
        .filter_map(|s| s.parse::<i64>().ok())
        .filter(|&id| id != self_id)
        .collect()
}

/// Builds a comma-separated placeholder string `?1, ?2, …, ?N` for use in
/// `WHERE id IN (…)` queries.
fn placeholders_for(ids: &[i64]) -> String {
    (1..=ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Fetches a batch of nodes by `ids` in a single `WHERE id IN (…)` query,
/// returning them in the same order as the input slice.
fn fetch_nodes_by_ids(
    db: &rusqlite::Connection,
    ids: &[i64],
) -> rusqlite::Result<Vec<Node>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let sql = format!(
        "SELECT {NODE_COLUMNS} FROM nodes WHERE id IN ({})",
        placeholders_for(ids),
    );
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

/// Serialises a slice of `f32` values to a little-endian byte vector
/// compatible with `sqlite-vec`'s BLOB encoding.
fn f32_slice_to_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for &v in values {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

/// Deserialises a little-endian byte slice produced by [`f32_slice_to_bytes`]
/// back into a `Vec<f32>`.
fn bytes_to_f32_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Constructs a [`Node`] from a `rusqlite::Row` with the columns defined by
/// [`NODE_COLUMNS`]: `id, parent_id, path, doc_id, kind, level, title,
/// content, token_count, lang`.
fn node_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    let kind_str: String = row.get(4)?;
    let kind = NodeKind::try_from(kind_str.as_str()).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::fmt::Error),
        )
    })?;
    Ok(Node {
        id:          row.get(0)?,
        parent_id:   row.get(1)?,
        path:        row.get(2)?,
        doc_id:      row.get(3)?,
        kind,
        level:       row.get(5)?,
        title:       row.get(6)?,
        content:     row.get(7)?,
        token_count: row.get::<_, u32>(8)?,
        lang:        row.get(9)?,
    })
}

/// Fetches a single [`Node`] by `id` from the provided synchronous connection.
fn node_from_db(db: &rusqlite::Connection, id: i64) -> rusqlite::Result<Node> {
    db.query_row(
        &format!("SELECT {NODE_COLUMNS} FROM nodes WHERE id = ?1"),
        params![id],
        node_from_row,
    )
}
