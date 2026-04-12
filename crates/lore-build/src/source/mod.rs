//! Pluggable build sources for the Lore documentation pipeline.
//!
//! A [`Source`] knows how to materialise documentation content into a local
//! directory.  The caller then hands that directory to [`PackageBuilder`] to
//! run the standard parse → chunk → embed → index pipeline.
//!
//! Three sources ship out of the box:
//! - [`LocalSource`] — a directory already on disk (no-op materialisation).
//! - [`GitSource`] — clone or update a git repository.
//! - [`WebsiteSource`] — crawl a website and convert pages to Markdown.

mod git;
mod local;
mod website;

pub use git::GitSource;
pub use local::LocalSource;
pub use website::WebsiteSource;

use std::path::PathBuf;

use lore_core::LoreError;

// ── PreparedSource ────────────────────────────────────────────────────────────

/// The result of calling [`Source::prepare`].
///
/// Holds the directory that contains the materialised documentation and an
/// optional git commit SHA (for provenance).  If the directory was created
/// by the source, `_temp` keeps the [`tempfile::TempDir`] alive; it is
/// dropped — and the directory deleted — when this struct is dropped.
pub struct PreparedSource {
    /// Directory containing documentation files ready for indexing.
    pub dir: PathBuf,
    /// Git commit SHA at which the documentation was captured, if applicable.
    pub git_sha: Option<String>,
    // Keeps a temp directory alive for the lifetime of this value.
    _temp: Option<tempfile::TempDir>,
}

impl PreparedSource {
    /// Create a `PreparedSource` backed by a pre-existing directory.
    pub const fn from_dir(dir: PathBuf) -> Self {
        Self { dir, git_sha: None, _temp: None }
    }

    /// Create a `PreparedSource` that owns a temporary directory.
    pub fn from_temp(temp: tempfile::TempDir, git_sha: Option<String>) -> Self {
        let dir = temp.path().to_path_buf();
        Self { dir, git_sha, _temp: Some(temp) }
    }
}

// ── Source trait ──────────────────────────────────────────────────────────────

/// A source that can materialise documentation content into a local directory.
pub trait Source {
    /// Prepare the source — fetch, clone, or crawl — and return the
    /// directory of documentation files ready for indexing.
    fn prepare(
        &self,
    ) -> impl std::future::Future<Output = Result<PreparedSource, LoreError>> + Send;
}
