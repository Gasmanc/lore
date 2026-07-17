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
    /// Generate docs from a crate's `rustdoc --output-format json`.
    ///
    /// The build creates a throwaway cargo project pinned to `version` (with the
    /// given `features`) and runs `cargo +nightly rustdoc`, so the indexed docs
    /// are the exact locked-version API — reproducible on any machine with a
    /// nightly toolchain.
    Rustdoc {
        /// Crate name on crates.io (e.g. `axum`).
        #[serde(rename = "crate")]
        crate_name: String,
        /// Exact version to document (e.g. `0.8.9`).
        version: String,
        /// Cargo features to enable while documenting.
        #[serde(default)]
        features: Vec<String>,
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
    let spec: PackageSpec =
        serde_yaml::from_str(&yaml).map_err(|e| LoreError::Schema(e.to_string()))?;
    spec.validate()?;
    Ok(spec)
}

impl PackageSpec {
    /// Validates the fields that get turned into filesystem paths or shell/git
    /// arguments downstream.
    ///
    /// This is the single home for spec-field safety: a `subdir` must stay
    /// inside the cloned repo, and a git/website `url` must use an expected
    /// scheme (so it cannot be misparsed as a `git` option — the classic
    /// argument-injection vector).
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::InvalidConfig`] on the first unsafe field.
    pub fn validate(&self) -> Result<(), LoreError> {
        match &self.source {
            SourceSpec::Git { url, subdir, .. } => {
                validate_source_url(url)?;
                if let Some(sub) = subdir {
                    validate_subdir(sub)?;
                }
            }
            SourceSpec::Website { url, .. } => validate_source_url(url)?,
            SourceSpec::Local { .. } => {}
            SourceSpec::Rustdoc { crate_name, version, features } => {
                validate_crate_token(crate_name, "source.crate")?;
                validate_crate_token(version, "source.version")?;
                for f in features {
                    validate_crate_token(f, "source.features")?;
                }
            }
        }
        Ok(())
    }
}

/// Rejects a crate name / version / feature that could inject cargo arguments
/// or shell metacharacters when the build assembles a throwaway project.
fn validate_crate_token(token: &str, field: &str) -> Result<(), LoreError> {
    let ok = !token.is_empty()
        && token.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'));
    if ok {
        Ok(())
    } else {
        Err(LoreError::InvalidConfig(format!(
            "{field} '{token}' must be non-empty and match [A-Za-z0-9._+-]"
        )))
    }
}

/// Rejects a `subdir` that could escape the cloned repository root.
fn validate_subdir(subdir: &str) -> Result<(), LoreError> {
    let p = Path::new(subdir);
    if p.is_absolute()
        || subdir.contains("..")
        || p.components().any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(LoreError::InvalidConfig(format!(
            "source.subdir '{subdir}' must be a relative path inside the repository"
        )));
    }
    Ok(())
}

/// Requires a source URL to use an expected scheme, so it can never be
/// misinterpreted as a command-line option by downstream git tooling.
fn validate_source_url(url: &str) -> Result<(), LoreError> {
    let ok = url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("git://")
        || url.starts_with("git@");
    if ok {
        Ok(())
    } else {
        Err(LoreError::InvalidConfig(format!(
            "source url '{url}' must start with https://, http://, git://, or git@"
        )))
    }
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
        assert!(
            matches!(spec.source, SourceSpec::Git { ref branch, .. } if branch.as_deref() == Some("v15.0.0"))
        );
        assert!(spec.build.exclude_examples);
    }

    #[test]
    fn parse_website_spec() {
        let spec: PackageSpec = serde_yaml::from_str(WEBSITE_YAML).unwrap();
        assert_eq!(spec.name, "tokio");
        assert!(matches!(spec.source, SourceSpec::Website { max_pages: Some(200), .. }));
    }

    #[test]
    fn parse_and_validate_rustdoc_spec() {
        let yaml = r#"
name: sqlx
registry: cargo
version: "0.8.6"
source:
  type: rustdoc
  crate: sqlx
  version: "0.8.6"
  features: [runtime-tokio, tls-rustls, sqlite]
"#;
        let spec: PackageSpec = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(
            spec.source,
            SourceSpec::Rustdoc { ref crate_name, ref version, .. }
                if crate_name == "sqlx" && version == "0.8.6"
        ));
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn rejects_rustdoc_arg_injection() {
        let yaml = r#"
name: x
registry: cargo
version: "1"
source:
  type: rustdoc
  crate: "--evil flag"
  version: "1.0.0"
"#;
        let spec: PackageSpec = serde_yaml::from_str(yaml).unwrap();
        assert!(spec.validate().is_err());
    }

    #[test]
    fn valid_specs_pass_validation() {
        for yaml in [GIT_YAML, WEBSITE_YAML] {
            let spec: PackageSpec = serde_yaml::from_str(yaml).unwrap();
            assert!(spec.validate().is_ok());
        }
    }

    #[test]
    fn rejects_subdir_traversal() {
        let yaml = r#"
name: x
registry: cargo
version: "1"
source:
  type: git
  url: "https://github.com/x/y"
  subdir: "../../../etc"
"#;
        let spec: PackageSpec = serde_yaml::from_str(yaml).unwrap();
        assert!(spec.validate().is_err());
    }

    #[test]
    fn rejects_option_like_url() {
        let yaml = r#"
name: x
registry: cargo
version: "1"
source:
  type: git
  url: "--upload-pack=touch /tmp/pwned"
"#;
        let spec: PackageSpec = serde_yaml::from_str(yaml).unwrap();
        assert!(spec.validate().is_err());
    }
}
