//! Top-level package build orchestrator.
//!
//! [`PackageBuilder`] discovers documentation files in a source directory,
//! runs the full parse → chunk → embed → index pipeline on each one via
//! [`crate::indexer::Indexer`], rebuilds the FTS5 index, and writes package
//! metadata to the `meta` table.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lore_core::{Db, LoreError, Package};
use tracing::{info, instrument, warn};

use crate::{
    chunker::{ChunkConfig, SemanticRefiner, StructuralChunker},
    discovery::discover_files,
    embedder::Embedder,
    indexer::Indexer,
    parser::ParserRegistry,
    tokens::TokenCounter,
};

// ── Public types ──────────────────────────────────────────────────────────────

/// Statistics collected during a package build run.
#[derive(Debug, Clone, Default)]
pub struct BuildStats {
    /// Number of documentation files discovered and processed.
    pub files_processed: u32,
    /// Number of files that failed to parse or index (errors were logged).
    pub files_failed: u32,
    /// Total number of prose chunks inserted.
    pub chunk_count: u32,
    /// Total number of code-block chunks inserted.
    pub code_block_count: u32,
    /// Total tokens across all inserted chunks.
    pub total_tokens: u64,
    /// Wall-clock duration of the build.
    pub duration: Duration,
}

impl BuildStats {
    /// Returns a single-line human-readable summary.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} files, {} chunks, {} code blocks, {} tokens in {:.1}s",
            self.files_processed,
            self.chunk_count,
            self.code_block_count,
            self.total_tokens,
            self.duration.as_secs_f64(),
        )
    }
}

/// Builds a Lore package `.db` from a directory of documentation files.
///
/// # Usage
///
/// ```no_run
/// # use lore_build::builder::PackageBuilder;
/// # use lore_core::Package;
/// # use std::path::Path;
/// # async fn example() -> Result<(), lore_core::LoreError> {
/// let stats = PackageBuilder::new(Path::new("~/.cache/lore/models"))
///     .expect("builder init")
///     .build(
///         Path::new("./docs"),
///         Package { name: "mylib".into(), registry: "cargo".into(),
///                   version: "1.0.0".into(), description: None,
///                   source_url: None, git_sha: None },
///         Path::new("./mylib@1.0.0.db"),
///         false,
///     )
///     .await?;
/// println!("{}", stats.summary());
/// # Ok(())
/// # }
/// ```
pub struct PackageBuilder {
    embedder: Embedder,
    config: ChunkConfig,
}

impl PackageBuilder {
    /// Initialises the builder, loading the embedding model from `cache_dir`.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::Embed`] if the embedding model cannot be loaded.
    pub fn new(cache_dir: &Path) -> Result<Self, LoreError> {
        let embedder = Embedder::new(cache_dir)?;
        Ok(Self { embedder, config: ChunkConfig::default() })
    }

    /// Returns the [`Embedder`] used by this builder.
    ///
    /// Callers that need to embed query strings can reuse this instance rather
    /// than loading a second copy of the model.
    pub const fn embedder(&self) -> &Embedder {
        &self.embedder
    }

    /// Builds a package from `source_dir` and writes it to `output_path`.
    ///
    /// `meta` is written to the `meta` table after all documents are indexed.
    /// Set `exclude_examples` to `true` to skip `examples/`, `tests/`, and
    /// similar directories.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError`] if the database cannot be created or a fatal
    /// error occurs during indexing.
    #[instrument(skip(self, meta), fields(source = %source_dir.display(), output = %output_path.display()))]
    pub async fn build(
        &self,
        source_dir: &Path,
        meta: Package,
        output_path: &Path,
        exclude_examples: bool,
    ) -> Result<BuildStats, LoreError> {
        let start = Instant::now();

        // Open (or create) the output database.
        let db = Db::open(output_path).await?;

        // Write package metadata before indexing so the database is always
        // in a consistent state if the build is interrupted.
        self.write_meta(&db, &meta).await?;

        // Discover documentation files.
        let files = discover_files(source_dir, exclude_examples)?;
        info!(count = files.len(), "discovered documentation files");

        // Build the indexer once and reuse it for all files.
        let indexer = self.make_indexer(db.clone());

        let mut stats = BuildStats::default();

        for file_path in &files {
            match self.index_one(&indexer, file_path, source_dir, &mut stats).await {
                Ok(()) => stats.files_processed += 1,
                Err(e) => {
                    warn!(path = %file_path.display(), error = %e, "skipping file due to error");
                    stats.files_failed += 1;
                }
            }
        }

        // Rebuild FTS5 index after all nodes are inserted (bulk rebuild is
        // faster than incremental triggers for a one-shot build).
        db.rebuild_fts().await?;

        // Compact and tune the database.
        db.optimize().await?;

        stats.duration = start.elapsed();
        info!(summary = %stats.summary(), "build complete");
        Ok(stats)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    async fn write_meta(&self, db: &Db, meta: &Package) -> Result<(), LoreError> {
        db.set_meta("name".into(), meta.name.clone()).await?;
        db.set_meta("registry".into(), meta.registry.clone()).await?;
        db.set_meta("version".into(), meta.version.clone()).await?;
        if let Some(ref d) = meta.description {
            db.set_meta("description".into(), d.clone()).await?;
        }
        if let Some(ref u) = meta.source_url {
            db.set_meta("source_url".into(), u.clone()).await?;
        }
        if let Some(ref s) = meta.git_sha {
            db.set_meta("git_sha".into(), s.clone()).await?;
        }
        Ok(())
    }

    fn make_indexer(&self, db: Db) -> Indexer {
        let config = self.config.clone();
        Indexer::new(
            ParserRegistry::new(),
            StructuralChunker::new(config.clone(), TokenCounter::new().expect("tokenizer")),
            SemanticRefiner::new(config, TokenCounter::new().expect("tokenizer")),
            self.embedder.clone(),
            db,
        )
    }

    /// Reads a file, runs the indexer, and updates build stats.
    async fn index_one(
        &self,
        indexer: &Indexer,
        file_path: &PathBuf,
        source_dir: &Path,
        stats: &mut BuildStats,
    ) -> Result<(), LoreError> {
        let content = std::fs::read_to_string(file_path).map_err(LoreError::Io)?;

        // Store a relative path so the package is portable.
        let rel_path =
            file_path.strip_prefix(source_dir).unwrap_or(file_path).to_string_lossy().into_owned();

        if let Some(file_stats) = indexer.index_file(&rel_path, &content).await? {
            stats.chunk_count += file_stats.chunk_count;
            stats.code_block_count += file_stats.code_block_count;
            stats.total_tokens += file_stats.total_tokens;
        }

        Ok(())
    }
}
