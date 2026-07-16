//! [`LlmsTxtSource`] — ingest a site's `llms.txt` / `llms-full.txt` file.
//!
//! The [llms.txt standard](https://llmstxt.org) publishes a curated,
//! LLM-friendly Markdown digest of a documentation site at a well-known path.
//! `llms-full.txt` is the expanded variant containing the full docs inline.
//! Both are exactly the clean Markdown Lore's pipeline wants, so ingesting one
//! is near-zero-effort compared to crawling and de-boilerplating HTML.
//!
//! Given a base site URL this source tries, in order:
//! 1. the URL itself, if it already points at a `.txt` file;
//! 2. `<base>/llms-full.txt`;
//! 3. `<base>/llms.txt`.
//!
//! The first that returns HTTP 200 is written into a temporary directory as a
//! single Markdown file for the standard parse → chunk → embed → index flow.

use std::time::Duration;

use lore_core::LoreError;
use reqwest::Url;
use tracing::{debug, info};

use super::{PreparedSource, Source};

/// Maximum bytes to accept for an `llms.txt` document (16 MiB). Guards against a
/// hostile or misconfigured endpoint streaming an unbounded body.
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// A documentation source backed by a site's `llms.txt` / `llms-full.txt`.
pub struct LlmsTxtSource {
    /// Base site URL or a direct link to an `llms(.full).txt` file.
    pub url: String,
}

impl LlmsTxtSource {
    /// Create a source for `url` (a site root or a direct `.txt` link).
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }

    /// Build the ordered list of candidate URLs to try.
    fn candidates(&self) -> Result<Vec<Url>, LoreError> {
        let base = Url::parse(&self.url)
            .map_err(|e| LoreError::InvalidConfig(format!("invalid url '{}': {e}", self.url)))?;

        let path = base.path();
        if std::path::Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("txt")) {
            return Ok(vec![base]);
        }
        // Join against the base so a path prefix (e.g. `example.com/docs/`) is
        // respected. `join` needs a trailing slash to treat the base as a dir.
        let dir = if path.ends_with('/') {
            base
        } else {
            let with_slash = format!("{base}/");
            Url::parse(&with_slash).unwrap_or(base)
        };
        let full = dir
            .join("llms-full.txt")
            .map_err(|e| LoreError::InvalidConfig(format!("cannot form llms-full.txt url: {e}")))?;
        let short = dir
            .join("llms.txt")
            .map_err(|e| LoreError::InvalidConfig(format!("cannot form llms.txt url: {e}")))?;
        Ok(vec![full, short])
    }
}

impl Source for LlmsTxtSource {
    async fn prepare(&self) -> Result<PreparedSource, LoreError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("lore/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| LoreError::Registry(e.to_string()))?;

        let candidates = self.candidates()?;
        let mut last_status = None;
        for url in &candidates {
            debug!(%url, "trying llms.txt candidate");
            let resp = match http.get(url.clone()).send().await {
                Ok(r) => r,
                Err(e) => {
                    debug!(%url, error = %e, "request failed");
                    continue;
                }
            };
            if !resp.status().is_success() {
                last_status = Some(resp.status());
                continue;
            }
            let body = read_capped(resp).await?;
            if body.trim().is_empty() {
                continue;
            }

            let temp = tempfile::tempdir().map_err(LoreError::Io)?;
            let file = temp.path().join("llms.md");
            tokio::fs::write(&file, body.as_bytes()).await.map_err(LoreError::Io)?;
            info!(%url, bytes = body.len(), "ingested llms.txt");
            return Ok(PreparedSource::from_temp(temp, None));
        }

        Err(LoreError::Registry(format!(
            "no llms.txt or llms-full.txt found at {} (last status: {})",
            self.url,
            last_status.map_or_else(|| "none".to_owned(), |s| s.to_string())
        )))
    }
}

/// Read a response body, refusing anything larger than [`MAX_BYTES`].
async fn read_capped(mut resp: reqwest::Response) -> Result<String, LoreError> {
    let over_cap = || {
        LoreError::Registry(format!("llms.txt exceeds {} MiB size cap", MAX_BYTES / (1024 * 1024)))
    };
    if let Some(len) = resp.content_length() {
        if len > MAX_BYTES as u64 {
            return Err(over_cap());
        }
    }
    let mut buf: Vec<u8> = Vec::new();
    // `Response::chunk` streams the body without pulling in `futures`.
    while let Some(bytes) = resp.chunk().await.map_err(|e| LoreError::Registry(e.to_string()))? {
        if buf.len().saturating_add(bytes.len()) > MAX_BYTES {
            return Err(over_cap());
        }
        buf.extend_from_slice(&bytes);
    }
    String::from_utf8(buf).map_err(|e| LoreError::Parse(format!("llms.txt is not valid UTF-8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_txt_url_is_single_candidate() {
        let s = LlmsTxtSource::new("https://example.com/docs/llms-full.txt");
        let c = s.candidates().unwrap();
        assert_eq!(c.len(), 1);
        assert!(c[0].as_str().ends_with("llms-full.txt"));
    }

    #[test]
    fn base_url_expands_to_full_then_short() {
        let s = LlmsTxtSource::new("https://example.com");
        let c = s.candidates().unwrap();
        assert_eq!(c.len(), 2);
        assert!(c[0].as_str().ends_with("/llms-full.txt"));
        assert!(c[1].as_str().ends_with("/llms.txt"));
    }

    #[test]
    fn path_prefix_is_preserved() {
        let s = LlmsTxtSource::new("https://example.com/project/docs");
        let c = s.candidates().unwrap();
        assert!(c[0].as_str().ends_with("/project/docs/llms-full.txt"), "{}", c[0]);
    }

    #[test]
    fn invalid_url_errors() {
        assert!(LlmsTxtSource::new("not a url").candidates().is_err());
    }
}
