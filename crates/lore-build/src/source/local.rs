//! [`LocalSource`] — a directory already present on disk.

use std::path::PathBuf;

use lore_core::LoreError;

use super::{PreparedSource, Source};

/// A documentation source that is already on the local filesystem.
///
/// `prepare()` is a no-op: the directory is returned as-is.
pub struct LocalSource {
    /// Path to the directory containing documentation files.
    pub dir: PathBuf,
}

impl LocalSource {
    /// Create a `LocalSource` pointing at `dir`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Source for LocalSource {
    async fn prepare(&self) -> Result<PreparedSource, LoreError> {
        Ok(PreparedSource::from_dir(self.dir.clone()))
    }
}
