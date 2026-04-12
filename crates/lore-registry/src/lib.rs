//! Registry API client and package download pipeline for Lore.
//!
//! [`RegistryClient`] fetches the package index from a remote registry,
//! searches it, and streams package `.db` files to disk with an optional
//! [`indicatif::ProgressBar`].
//!
//! # Registry protocol
//!
//! The registry exposes two endpoints under `base_url`:
//! - `GET /index.json` — returns a JSON array of [`RegistryEntry`].
//! - `GET /packages/{display_key}.db` — returns the raw `SQLite` database.
//!
//! The `display_key` is `"{registry}-{name}@{version}"`, matching
//! [`lore_core::Package::display_key`].

#![warn(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    missing_docs,
    rust_2018_idioms
)]
#![allow(
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    clippy::must_use_candidate
)]

use std::path::Path;

use futures::StreamExt as _;
use indicatif::{ProgressBar, ProgressStyle};
use lore_core::{LoreError, PackageMetadata};
use serde::{Deserialize, Serialize};
use tracing::instrument;

// ── Public types ──────────────────────────────────────────────────────���───────

/// An entry in the registry index, extending [`PackageMetadata`] with the
/// canonical download URL for the package database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Core package metadata.
    #[serde(flatten)]
    pub metadata: PackageMetadata,
    /// Direct URL to the `.db` file.
    pub download_url: String,
}

// ── Client ────────────────────────────────────────────────────────────────────

/// HTTP client for the Lore package registry.
pub struct RegistryClient {
    base_url: String,
    http: reqwest::Client,
}

impl RegistryClient {
    /// The default public registry URL.
    pub const DEFAULT_URL: &'static str = "https://registry.lore.dev";

    /// Create a new client pointed at `base_url`.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::Registry`] if the HTTP client cannot be built.
    pub fn new(base_url: &str) -> Result<Self, LoreError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("lore/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| LoreError::Registry(e.to_string()))?;
        Ok(Self { base_url: base_url.trim_end_matches('/').to_owned(), http })
    }

    /// Fetch the full registry index.
    #[instrument(skip(self), fields(base_url = %self.base_url))]
    pub async fn list(&self) -> Result<Vec<RegistryEntry>, LoreError> {
        let url = format!("{}/index.json", self.base_url);
        let entries: Vec<RegistryEntry> = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| LoreError::Registry(e.to_string()))?
            .error_for_status()
            .map_err(|e| LoreError::Registry(e.to_string()))?
            .json()
            .await
            .map_err(|e| LoreError::Registry(format!("malformed index: {e}")))?;
        Ok(entries)
    }

    /// Search the registry for packages whose name contains `query`.
    ///
    /// The match is case-insensitive.
    pub async fn search(&self, query: &str) -> Result<Vec<RegistryEntry>, LoreError> {
        let lower = query.to_lowercase();
        let all = self.list().await?;
        Ok(all
            .into_iter()
            .filter(|e| e.metadata.package.name.to_lowercase().contains(&lower))
            .collect())
    }

    /// Download a package database to `target_path`.
    ///
    /// If `progress` is `Some`, it is updated as bytes are received and
    /// finished when the download completes.
    ///
    /// The parent directory of `target_path` must already exist.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::Registry`] on HTTP errors or
    /// [`LoreError::Io`] on write failures.
    #[instrument(skip(self, progress), fields(url = %entry.download_url))]
    pub async fn download(
        &self,
        entry: &RegistryEntry,
        target_path: &Path,
        progress: Option<&ProgressBar>,
    ) -> Result<(), LoreError> {
        let resp = self
            .http
            .get(&entry.download_url)
            .send()
            .await
            .map_err(|e| LoreError::Registry(e.to_string()))?
            .error_for_status()
            .map_err(|e| LoreError::Registry(e.to_string()))?;

        if let (Some(pb), Some(total)) = (progress, resp.content_length()) {
            pb.set_length(total);
            pb.set_style(download_style());
        }

        let tmp_path = target_path.with_extension("db.tmp");
        let file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(LoreError::Io)?;
        let mut file = tokio::io::BufWriter::new(file);

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| LoreError::Registry(e.to_string()))?;
            tokio::io::AsyncWriteExt::write_all(&mut file, &bytes)
                .await
                .map_err(LoreError::Io)?;
            if let Some(pb) = progress {
                #[allow(clippy::cast_possible_truncation)]
                pb.inc(bytes.len() as u64);
            }
        }

        tokio::io::AsyncWriteExt::flush(&mut file).await.map_err(LoreError::Io)?;

        // Atomic rename so the target is never half-written.
        tokio::fs::rename(&tmp_path, target_path)
            .await
            .map_err(LoreError::Io)?;

        if let Some(pb) = progress {
            pb.finish_and_clear();
        }
        Ok(())
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

fn download_style() -> ProgressStyle {
    ProgressStyle::default_bar()
        .template("{spinner:.cyan} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .expect("valid template")
        .progress_chars("=>-")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_construction_succeeds() {
        assert!(RegistryClient::new(RegistryClient::DEFAULT_URL).is_ok());
    }

    #[test]
    fn client_strips_trailing_slash() {
        let c = RegistryClient::new("https://example.com/").unwrap();
        assert_eq!(c.base_url, "https://example.com");
    }
}
