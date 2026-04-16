//! MCP server for Lore — exposes documentation search and retrieval to AI
//! coding assistants over the Model Context Protocol.
//!
//! The server exposes four tools:
//! - `search_docs` — semantic + keyword search across a loaded package DB.
//! - `list_packages` — enumerate locally installed package databases.
//! - `get_manifest` — return metadata for a specific package.
//! - `get_node` — retrieve the full content of a specific node by id.
//!
//! # Entry point
//!
//! Call [`serve_stdio`] from `main`; it blocks until the client disconnects.

#![deny(clippy::all, clippy::pedantic, clippy::nursery, missing_docs, rust_2018_idioms)]
#![allow(clippy::module_name_repetitions, clippy::missing_errors_doc, clippy::must_use_candidate)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lore_build::Embedder;
use lore_core::{Db, LoreError, SearchConfig, SearchResult};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;

// ── Tool parameter types ───────────────────────────────────────────────────────

/// Parameters for the `search_docs` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Package key in `"{registry}-{name}@{version}"` format (e.g.
    /// `"npm-next@15.0.0"`).
    pub package: String,
    /// Natural-language or keyword query string.
    pub query: String,
    /// Maximum tokens to return across all results. Defaults to `2000`.
    #[serde(default)]
    pub token_budget: Option<u32>,
}

/// Parameters for the `get_manifest` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetManifestParams {
    /// Package key in `"{registry}-{name}@{version}"` format.
    pub package: String,
}

/// Parameters for the `get_node` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetNodeParams {
    /// Package key in `"{registry}-{name}@{version}"` format.
    pub package: String,
    /// Numeric node id as returned by `search_docs`.
    pub node_id: i64,
}

// ── Server ─────────────────────────────────────────────────────────────────────

/// MCP server that exposes Lore documentation to AI coding assistants.
#[derive(Clone)]
pub struct LoreServer {
    packages_dir: PathBuf,
    embedder: Arc<Embedder>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
#[allow(missing_docs)] // #[tool] macro generates undocumented wrapper fns
impl LoreServer {
    /// Construct a new [`LoreServer`].
    fn new_inner(packages_dir: PathBuf, embedder: Embedder) -> Self {
        Self { packages_dir, embedder: Arc::new(embedder), tool_router: Self::tool_router() }
    }

    // ── Tools ──────────────────────────────────────────────────────────────────

    /// Search indexed documentation using hybrid semantic + keyword retrieval.
    #[tool(
        description = "Search indexed documentation for a package using hybrid semantic + keyword retrieval. Returns ranked excerpts with heading paths and relevance scores."
    )]
    async fn search_docs(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<String, rmcp::Error> {
        let db = self.open_db(&p.package).await?;
        let embedder = self.embedder.clone();
        let query = p.query.clone();
        let embedding = tokio::task::spawn_blocking(move || embedder.embed(&query))
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let config = SearchConfig {
            token_budget: p.token_budget.unwrap_or(2000),
            ..SearchConfig::default()
        };

        let results = lore_search::search(&db, &p.query, &embedding, &config)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        Ok(format_search_results(&results))
    }

    /// List all locally installed documentation packages.
    #[tool(description = "List all locally installed documentation packages available for search.")]
    async fn list_packages(&self) -> Result<String, rmcp::Error> {
        let packages = scan_packages(&self.packages_dir)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        if packages.is_empty() {
            return Ok("No packages installed. Use `lore add <package>` to install one.".into());
        }

        let lines: Vec<String> = packages
            .iter()
            .map(|(key, meta)| {
                let desc = meta.description.as_deref().unwrap_or("");
                if desc.is_empty() { format!("- {key}") } else { format!("- {key}: {desc}") }
            })
            .collect();
        Ok(lines.join("\n"))
    }

    /// Return the compressed API surface manifest for an installed package.
    ///
    /// The manifest is a `~500 token` index of the package's public API,
    /// suitable for pasting into `CLAUDE.md` as a fingerpost.
    #[tool(
        description = "Return the compressed API surface manifest for an installed package (~500 tokens). Contains heading paths and API signatures suitable for pasting into CLAUDE.md."
    )]
    async fn get_manifest(
        &self,
        Parameters(p): Parameters<GetManifestParams>,
    ) -> Result<String, rmcp::Error> {
        let db = self.open_db(&p.package).await?;
        let manifest = db
            .get_meta("manifest".to_owned())
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        match manifest {
            Some(m) if !m.is_empty() => Ok(m),
            _ => Err(rmcp::Error::invalid_params(
                format!(
                    "package '{pkg}' has no manifest — rebuild with `lore build`",
                    pkg = p.package
                ),
                None,
            )),
        }
    }

    /// Retrieve the full content of a specific node by its numeric id.
    #[tool(
        description = "Retrieve the full content of a specific documentation node by its numeric id (as returned by search_docs)."
    )]
    async fn get_node(
        &self,
        Parameters(p): Parameters<GetNodeParams>,
    ) -> Result<String, rmcp::Error> {
        let db = self.open_db(&p.package).await?;
        let node = db
            .get_node(p.node_id)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        Ok(node.content.unwrap_or_default())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LoreServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: rmcp::model::Implementation {
                name: "lore".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..ServerInfo::default()
        }
    }
}

// ── Public entry point ─────────────────────────────────────────────────────────

/// Start the MCP server on stdin/stdout.
///
/// Blocks until the client closes the connection.
///
/// # Errors
///
/// Returns [`LoreError`] if the embedder cannot be initialised or if
/// the transport encounters a fatal I/O error.
pub async fn serve_stdio(packages_dir: PathBuf) -> Result<(), LoreError> {
    let cache = model_cache_dir();
    // Model loading (~130 MB) is CPU-bound and must not block the async reactor.
    let embedder = tokio::task::spawn_blocking(move || Embedder::new(&cache))
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))??;
    let server = LoreServer::new_inner(packages_dir, embedder);

    rmcp::ServiceExt::serve(server, rmcp::transport::io::stdio())
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))?;

    Ok(())
}

// ── Private helpers ────────────────────────────────────────────────────────────

impl LoreServer {
    /// Open the database for `package_key`.
    async fn open_db(&self, package_key: &str) -> Result<Db, rmcp::Error> {
        let path = self.packages_dir.join(format!("{package_key}.db"));
        Db::open(&path).await.map_err(|_| {
            rmcp::Error::invalid_params(
                format!("package '{package_key}' is not installed — run `lore add {package_key}`"),
                None,
            )
        })
    }
}

/// Returns the shared embedding model cache directory.
pub fn model_cache_dir() -> PathBuf {
    dirs_next::cache_dir().unwrap_or_else(std::env::temp_dir).join("lore").join("models")
}

/// Scan `packages_dir` for `*.db` files and return `(key, Package)` pairs.
pub async fn scan_packages(
    packages_dir: &Path,
) -> Result<Vec<(String, lore_core::Package)>, LoreError> {
    let dir = packages_dir.to_path_buf();
    let paths = tokio::task::spawn_blocking(move || -> Result<Vec<PathBuf>, LoreError> {
        let mut paths = Vec::new();
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(LoreError::Io(e)),
        };
        for entry in rd {
            let entry = entry.map_err(LoreError::Io)?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("db") {
                paths.push(path);
            }
        }
        paths.sort();
        Ok(paths)
    })
    .await
    .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))??;

    let mut join_set = tokio::task::JoinSet::new();
    for path in paths {
        join_set.spawn(async move {
            let key = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
            let db = Db::open(&path).await.ok()?;
            let meta = db.get_package_meta().await.ok()?;
            Some((key, meta))
        });
    }

    let mut out = Vec::new();
    while let Some(res) = join_set.join_next().await {
        if let Ok(Some(pair)) = res {
            out.push(pair);
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Format [`SearchResult`]s as a human-readable string for the MCP caller.
fn format_search_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.".into();
    }
    let mut parts = Vec::with_capacity(results.len());
    for (i, r) in results.iter().enumerate() {
        let path = if r.heading_path.is_empty() {
            r.doc_title.clone()
        } else {
            format!("{} › {}", r.doc_title, r.heading_path.join(" › "))
        };
        let content = r.node.content.as_deref().unwrap_or("").trim();
        parts.push(format!(
            "[{}] (id={}, score={:.3})\n{}\n{}",
            i + 1,
            r.node.id,
            r.score,
            path,
            content
        ));
    }
    parts.join("\n\n")
}
