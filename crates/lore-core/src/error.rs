use thiserror::Error;

/// All errors that can be produced by the `lore-core` crate.
#[derive(Debug, Error)]
pub enum LoreError {
    /// A rusqlite or SQLite-level error.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// The async database connection was closed before the call could complete.
    #[error("database connection closed unexpectedly")]
    ConnectionClosed,

    /// A schema validation or migration error.
    #[error("schema error: {0}")]
    Schema(String),

    /// A document parse error.
    #[error("parse error: {0}")]
    Parse(String),

    /// An embedding model error.
    #[error("embedding error: {0}")]
    Embed(String),

    /// A registry API or network error.
    #[error("registry error: {0}")]
    Registry(String),

    /// A requested resource (package, node, …) was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// An invalid or malformed configuration.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// An I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convert a `tokio_rusqlite` error (with an inner `rusqlite::Error`) into a
/// [`LoreError`].  The three variants are collapsed sensibly:
///
/// * `ConnectionClosed` → [`LoreError::ConnectionClosed`]
/// * `Close((_, e))` / `Error(e)` → [`LoreError::Database`]
impl From<tokio_rusqlite::Error<rusqlite::Error>> for LoreError {
    fn from(e: tokio_rusqlite::Error<rusqlite::Error>) -> Self {
        match e {
            tokio_rusqlite::Error::Close((_, db_err)) | tokio_rusqlite::Error::Error(db_err) => {
                Self::Database(db_err)
            }
            // `tokio_rusqlite::Error` is marked `non_exhaustive`; `ConnectionClosed`
            // and any future variants are all treated as connection-level failures.
            _ => Self::ConnectionClosed,
        }
    }
}
