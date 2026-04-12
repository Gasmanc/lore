//! Embedding pipeline using `fastembed` with the `bge-small-en-v1.5` model.
//!
//! The [`Embedder`] wraps `fastembed::TextEmbedding` and provides single-text
//! and batch embedding.  [`build_contextual_text`] prepends the heading
//! breadcrumb path before embedding each chunk, following the Anthropic
//! contextual-retrieval pattern for ~35% better retrieval recall.

use std::path::Path;
use std::sync::Arc;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use lore_core::LoreError;

/// Number of dimensions produced by `bge-small-en-v1.5`.
pub const EMBEDDING_DIMS: usize = 384;

/// Batch size used when embedding multiple texts.
const BATCH_SIZE: usize = 32;

// ── Embedder ──────────────────────────────────────────────────────────────────

/// Wraps a `fastembed` text-embedding model.
///
/// Initialisation downloads the model on first use (~130 MB, cached in
/// `cache_dir`).  All subsequent calls use the local cache.
///
/// `Embedder` is cheaply cloneable — all clones share the same underlying
/// model instance via [`Arc`].
#[derive(Clone)]
pub struct Embedder {
    model: Arc<TextEmbedding>,
}

impl Embedder {
    /// Create an [`Embedder`] backed by `bge-small-en-v1.5`.
    ///
    /// If the model is not yet cached under `cache_dir`, it is downloaded
    /// automatically.  A progress message is printed to `stderr`.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::Embed`] if model initialisation fails.
    pub fn new(cache_dir: &Path) -> Result<Self, LoreError> {
        let needs_download = !model_is_cached(cache_dir);
        if needs_download {
            eprintln!("Downloading embedding model bge-small-en-v1.5 (~130 MB, one-time setup)…");
        }

        let opts = InitOptions::new(EmbeddingModel::BGESmallENV15)
            .with_cache_dir(cache_dir.to_path_buf())
            .with_show_download_progress(needs_download);

        let model = TextEmbedding::try_new(opts)
            .map_err(|e| LoreError::Embed(format!("model init failed: {e}")))?;

        Ok(Self { model: Arc::new(model) })
    }

    /// Embed a single text string and return the 384-dimensional vector.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::Embed`] if the embedding call fails.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, LoreError> {
        let mut results = self
            .model
            .embed(vec![text], None)
            .map_err(|e| LoreError::Embed(format!("embedding failed: {e}")))?;

        results
            .pop()
            .ok_or_else(|| LoreError::Embed("embedder returned empty results".into()))
    }

    /// Embed a batch of texts.  Processes up to [`BATCH_SIZE`] texts per call.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::Embed`] if any batch fails.
    pub fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, LoreError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let mut all = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(BATCH_SIZE) {
            let batch: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let results = self
                .model
                .embed(batch, None)
                .map_err(|e| LoreError::Embed(format!("batch embedding failed: {e}")))?;
            all.extend(results);
        }
        Ok(all)
    }

    /// The number of dimensions this model produces (always 384 for `bge-small-en-v1.5`).
    #[must_use]
    pub const fn dimensions() -> usize {
        EMBEDDING_DIMS
    }
}

// ── Contextual text builder ───────────────────────────────────────────────────

/// Build the contextual text to embed for a chunk.
///
/// Prepends the heading breadcrumb path to the content, separated by
/// `" > "` and a blank line.  This gives the embedding model the structural
/// context of where the chunk lives in the document, significantly improving
/// retrieval recall (Anthropic contextual-retrieval finding).
///
/// If `heading_path` is empty the raw `content` is returned unchanged.
///
/// # Examples
///
/// ```
/// use lore_build::build_contextual_text;
///
/// let text = build_contextual_text(
///     &["Next.js".into(), "Caching".into(), "cacheLife()".into()],
///     "Controls TTL.",
/// );
/// assert_eq!(text, "Next.js > Caching > cacheLife()\n\nControls TTL.");
/// ```
#[must_use]
pub fn build_contextual_text(heading_path: &[String], content: &str) -> String {
    if heading_path.is_empty() {
        return content.to_owned();
    }
    format!("{}\n\n{}", heading_path.join(" > "), content)
}

// ── Cache probe ───────────────────────────────────────────────────────────────

/// Returns `true` if the `bge-small-en-v1.5` model appears to be cached.
///
/// `fastembed` stores models in `<cache_dir>/<model-name>/`.  We check for
/// the presence of the directory rather than specific files so this stays
/// robust across fastembed minor versions.
fn model_is_cached(cache_dir: &Path) -> bool {
    // fastembed uses the model's string ID as the subdirectory name.
    // BGESmallENV15 resolves to "fast-bge-small-en-v1.5".
    let candidates = [
        "fast-bge-small-en-v1.5",
        "bge-small-en-v1.5",
        "BAAI/bge-small-en-v1.5",
    ];
    candidates.iter().any(|name| cache_dir.join(name).exists())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use super::*;

    /// Shared embedder — initialised at most once per test binary execution.
    /// Prevents file-system races when tests run on multiple threads.
    static EMBEDDER: LazyLock<Embedder> = LazyLock::new(|| {
        let cache = dirs_next::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("lore")
            .join("models");
        Embedder::new(&cache).expect("embedder must initialise")
    });

    // ── Contextual text (no model required) ──────────────────────────────────

    #[test]
    fn test_contextual_text_format() {
        let path = vec!["Next.js".into(), "Caching".into(), "cacheLife()".into()];
        let result = build_contextual_text(&path, "Controls TTL.");
        assert_eq!(result, "Next.js > Caching > cacheLife()\n\nControls TTL.");
    }

    #[test]
    fn test_contextual_text_empty_path() {
        let result = build_contextual_text(&[], "Just the content.");
        assert_eq!(result, "Just the content.");
    }

    #[test]
    fn test_contextual_text_single_heading() {
        let result = build_contextual_text(&["Overview".into()], "Intro paragraph.");
        assert_eq!(result, "Overview\n\nIntro paragraph.");
    }

    // ── Embedding tests (require model download on first run) ─────────────────

    #[test]
    fn test_embed_returns_384_dims() {
        let vec = EMBEDDER.embed("Hello, world!").expect("embed must succeed");
        assert_eq!(vec.len(), EMBEDDING_DIMS);
    }

    #[test]
    fn test_embed_batch_returns_correct_count() {
        let texts = vec!["first".into(), "second".into(), "third".into()];
        let results = EMBEDDER.embed_batch(&texts).expect("batch must succeed");
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|v| v.len() == EMBEDDING_DIMS));
    }

    #[test]
    fn test_similar_texts_higher_similarity() {
        use crate::chunker::semantic::cosine_similarity;

        let dog = EMBEDDER.embed("dog").unwrap();
        let puppy = EMBEDDER.embed("puppy").unwrap();
        let quantum = EMBEDDER.embed("quantum mechanics wave function").unwrap();

        let sim_dog_puppy = cosine_similarity(&dog, &puppy);
        let sim_dog_quantum = cosine_similarity(&dog, &quantum);

        assert!(
            sim_dog_puppy > sim_dog_quantum,
            "dog/puppy ({sim_dog_puppy:.3}) should be more similar than dog/quantum ({sim_dog_quantum:.3})"
        );
    }

    #[test]
    fn test_dimensions_constant() {
        assert_eq!(Embedder::dimensions(), 384);
    }
}
