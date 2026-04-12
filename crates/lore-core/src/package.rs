use serde::{Deserialize, Serialize};

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
