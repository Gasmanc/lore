//! YAML package specifications — describe how to fetch and build a package.
//!
//! A [`PackageSpec`] is loaded from a `.yaml` file in the `packages/` tree
//! and passed to [`build_from_spec`] to produce a `.db` file.

use std::path::Path;

use lore_core::{LoreError, Package};
use serde::Deserialize;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Full specification for a Lore documentation package.
#[derive(Debug, Deserialize)]
pub struct PackageSpec {
    /// Package name (e.g. `"next"`).
    pub name: String,
    /// Registry identifier (e.g. `"npm"`, `"cargo"`, `"pypi"`).
    pub registry: String,
    /// Version string (e.g. `"15.0.0"`).
    pub version: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// URL of the canonical upstream source.
    pub source_url: Option<String>,
    /// Where to fetch the documentation content.
    pub source: SourceSpec,
    /// Optional per-package build overrides.
    #[serde(default)]
    pub build: BuildOptions,
}

impl From<&PackageSpec> for Package {
    /// Convert to a [`Package`] value (without git SHA, which is set later).
    fn from(s: &PackageSpec) -> Self {
        Self {
            name: s.name.clone(),
            registry: s.registry.clone(),
            version: s.version.clone(),
            description: s.description.clone(),
            source_url: s.source_url.clone(),
            git_sha: None,
        }
    }
}

/// How to obtain the documentation content.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SourceSpec {
    /// Clone a git repository.
    Git {
        /// Remote URL.
        url: String,
        /// Branch, tag, or commit SHA to check out.
        branch: Option<String>,
        /// Optional subdirectory within the repository to index.
        subdir: Option<String>,
    },
    /// Crawl a website.
    Website {
        /// Starting URL for the crawl.
        url: String,
        /// Maximum number of pages to fetch (defaults to 500).
        max_pages: Option<usize>,
    },
    /// Use a directory already on the local filesystem.
    Local {
        /// Absolute or relative path to the documentation directory.
        dir: String,
    },
}

/// Build-time options that can be overridden per package.
#[derive(Debug, Default, Deserialize)]
pub struct BuildOptions {
    /// Skip `examples/`, `tests/`, and similar directories.
    #[serde(default)]
    pub exclude_examples: bool,
}

// ── Loading ───────────────────────────────────────────────────────────────────

/// Load a [`PackageSpec`] from a YAML file at `path`.
///
/// # Errors
///
/// Returns [`LoreError::Io`] if the file cannot be read or
/// [`LoreError::Schema`] if the YAML is malformed.
pub fn load_spec(path: &Path) -> Result<PackageSpec, LoreError> {
    let yaml = std::fs::read_to_string(path).map_err(LoreError::Io)?;
    serde_yaml::from_str(&yaml).map_err(|e| LoreError::Schema(e.to_string()))
}

/// Walk `specs_dir` and load all `*.yaml` files as package specs.
///
/// Errors encountered for individual files are logged as warnings; the
/// remaining specs are returned.
pub fn load_all_specs(specs_dir: &Path) -> Result<Vec<PackageSpec>, LoreError> {
    let mut specs = Vec::new();
    let rd = match std::fs::read_dir(specs_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(specs),
        Err(e) => return Err(LoreError::Io(e)),
    };
    for entry in rd {
        let entry = entry.map_err(LoreError::Io)?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        match load_spec(&path) {
            Ok(spec) => specs.push(spec),
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "skipping invalid spec"),
        }
    }
    Ok(specs)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const GIT_YAML: &str = r#"
name: next
registry: npm
version: "15.0.0"
description: "The React Framework for the Web"
source_url: "https://github.com/vercel/next.js"
source:
  type: git
  url: "https://github.com/vercel/next.js"
  branch: v15.0.0
  subdir: docs
build:
  exclude_examples: true
"#;

    const WEBSITE_YAML: &str = r#"
name: tokio
registry: cargo
version: "1"
source:
  type: website
  url: "https://tokio.rs"
  max_pages: 200
"#;

    #[test]
    fn parse_git_spec() {
        let spec: PackageSpec = serde_yaml::from_str(GIT_YAML).unwrap();
        assert_eq!(spec.name, "next");
        assert!(matches!(spec.source, SourceSpec::Git { ref branch, .. } if branch.as_deref() == Some("v15.0.0")));
        assert!(spec.build.exclude_examples);
    }

    #[test]
    fn parse_website_spec() {
        let spec: PackageSpec = serde_yaml::from_str(WEBSITE_YAML).unwrap();
        assert_eq!(spec.name, "tokio");
        assert!(matches!(spec.source, SourceSpec::Website { max_pages: Some(200), .. }));
    }
}
