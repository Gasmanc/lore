use serde::{Deserialize, Serialize};

use crate::error::LoreError;

/// Validates that a package key (or any individual component of one) is safe to
/// use as a filesystem path segment.
///
/// This is the single authoritative home for the "a package key can never
/// escape the packages directory" invariant.  Every code path that turns an
/// externally-supplied key — a CLI argument, an MCP tool parameter, or a
/// registry index entry — into a `<packages_dir>/<key>.db` path MUST call this
/// first.
///
/// A key is rejected if it is empty, contains a path separator (`/` or `\`), a
/// `..` sequence, a NUL byte, or any character outside the conservative set
/// `[A-Za-z0-9._@+-]`.  That set is a superset of every legitimate
/// `{registry}-{name}@{version}` key while excluding everything usable for
/// traversal.
///
/// # Errors
///
/// Returns [`LoreError::InvalidConfig`] describing the first violation found.
pub fn validate_package_key(key: &str) -> Result<(), LoreError> {
    if key.is_empty() {
        return Err(LoreError::InvalidConfig("package key is empty".into()));
    }
    if key.contains("..") {
        return Err(LoreError::InvalidConfig(format!(
            "package key '{key}' contains a '..' sequence"
        )));
    }
    if let Some(bad) = key
        .chars()
        .find(|&c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '+' | '-')))
    {
        return Err(LoreError::InvalidConfig(format!(
            "package key '{key}' contains an illegal character {bad:?}"
        )));
    }
    Ok(())
}

/// Metadata about an indexed documentation package, stored in the `meta` table
/// and also used when communicating with the registry API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Package name, e.g. `"next"`.
    pub name: String,
    /// Registry this package belongs to, e.g. `"npm"`, `"pypi"`, `"cargo"`.
    pub registry: String,
    /// Semantic version string, e.g. `"15.0.0"`.
    pub version: String,
    /// Human-readable description, if available.
    pub description: Option<String>,
    /// URL of the upstream source (git repository, website, …).
    pub source_url: Option<String>,
    /// Git commit SHA at which the documentation was indexed.
    pub git_sha: Option<String>,
}

impl Package {
    /// Returns a canonical display key in the form `"{registry}-{name}@{version}"`.
    ///
    /// This key is used as the file stem when storing the package on disk, and
    /// as the enum value exposed through the MCP `get_docs` tool.
    #[must_use]
    pub fn display_key(&self) -> String {
        format!("{}-{}@{}", self.registry, self.name, self.version)
    }

    /// Validates that this package's identity is safe to materialise on disk.
    ///
    /// Checks the `registry`, `name`, and `version` components individually —
    /// and the assembled [`display_key`](Self::display_key) — against
    /// [`validate_package_key`].  Called at the registry-download boundary so a
    /// malicious index entry can never write outside the packages directory.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::InvalidConfig`] if any component is unsafe.
    pub fn validate(&self) -> Result<(), LoreError> {
        validate_package_key(&self.registry)?;
        validate_package_key(&self.name)?;
        validate_package_key(&self.version)?;
        validate_package_key(&self.display_key())
    }
}

/// Richer metadata returned by the registry search API, extending [`Package`]
/// with build-time statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMetadata {
    /// Core package identity and provenance.
    #[serde(flatten)]
    pub package: Package,
    /// Size of the `.db` file in bytes.
    pub size_bytes: Option<u64>,
    /// Total number of indexed chunks and code blocks.
    pub chunk_count: Option<u32>,
    /// ISO 8601 date-time string recorded when the package was built.
    pub build_date: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_keys() {
        for key in [
            "npm-next@15.0.0",
            "cargo-tokio@1.44.2",
            "pypi-pandas@2.2.3",
            "local-my_lib@0.1.0",
            "swift-swiftui@6.0",
        ] {
            assert!(validate_package_key(key).is_ok(), "{key} should be valid");
        }
    }

    #[test]
    fn rejects_path_traversal() {
        for key in ["../ESCAPED@1.0", "..", "a/../../b", "foo/bar", "foo\\bar", ""] {
            assert!(validate_package_key(key).is_err(), "{key:?} must be rejected");
        }
    }

    #[test]
    fn rejects_nul_and_control() {
        assert!(validate_package_key("foo\0bar").is_err());
        assert!(validate_package_key("foo\nbar").is_err());
    }

    #[test]
    fn package_validate_catches_component_traversal() {
        let malicious = Package {
            name: "evil".into(),
            registry: "../../../etc/cron.d/x".into(),
            version: "1".into(),
            description: None,
            source_url: None,
            git_sha: None,
        };
        assert!(malicious.validate().is_err());
    }
}
