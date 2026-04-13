//! Database connection management and schema migrations.
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
//!
//! # Module layout
//!
//! | Submodule     | Contents                                             |
//! |---------------|------------------------------------------------------|
//! | [`crud`]      | CRUD helpers: docs, nodes, embeddings, meta, FTS     |
//! | [`search`]    | FTS5 / vector KNN search and batch resolve helpers   |
//! | [`helpers`]   | Private row-mapping and query-building utilities     |

mod crud;
mod helpers;
mod search;

use std::path::Path;
use std::sync::OnceLock;

use rusqlite::params;
use tracing::{debug, instrument};

use crate::error::LoreError;

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
    // Cosine distance is appropriate for unit-normalised BGE embeddings.
    "CREATE VIRTUAL TABLE IF NOT EXISTS node_embeddings USING vec0(
        embedding FLOAT[384] distance_metric=cosine
    );",
];

// ---------------------------------------------------------------------------
// Node column list
// ---------------------------------------------------------------------------

/// The canonical SELECT column list for the `nodes` table used by
/// [`helpers::node_from_row`].  All query sites reference this constant so the
/// column order can never silently diverge.
pub(super) const NODE_COLUMNS: &str =
    "id, parent_id, path, doc_id, kind, level, title, content, token_count, lang";

/// Same as [`NODE_COLUMNS`] but prefixed with `n.` for use in JOIN queries
/// where multiple tables have columns with the same name (`title`, `content`).
pub(super) const NODE_COLUMNS_ALIASED: &str =
    "n.id, n.parent_id, n.path, n.doc_id, n.kind, n.level, n.title, n.content, n.token_count, n.lang";

// ---------------------------------------------------------------------------
// Db
// ---------------------------------------------------------------------------

/// Async wrapper around a single `SQLite` database file used by Lore.
///
/// Clone this handle freely — all clones share the same underlying connection
/// thread.
#[derive(Clone, Debug)]
pub struct Db {
    pub(super) conn: tokio_rusqlite::Connection,
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
}
