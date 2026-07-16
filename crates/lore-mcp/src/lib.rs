#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! MCP server for Lore — exposes documentation search and retrieval to AI
//! coding assistants over the Model Context Protocol.
//!
//! The server exposes these tools:
//! - `search_docs` — semantic + keyword search across a loaded package DB,
//!   with per-session deduplication of already-returned chunks.
//! - `search_stack` — federated search across every installed package matching
//!   the current project's declared dependencies.
//! - `resolve_package` — map a bare library name to installed package key(s).
//! - `stack_status` — report missing/stale docs vs the project's dependencies.
//! - `version_diff` — diff the API surface of two installed package versions.
//! - `list_packages` — enumerate locally installed package databases.
//! - `get_manifest` — return the compact API surface for a specific package.
//! - `get_node` — retrieve the full content of a specific node by id.
//! - `reset_session` — clear the per-session dedup memory.
//!
//! # Entry point
//!
//! Call [`serve_stdio`] from `main`; it blocks until the client disconnects.

/// Project dependency detection for stack-scoped search.
pub mod stack;

use std::collections::{HashMap, HashSet};
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
use tokio::sync::RwLock;

/// Upper bound on the byte length of an MCP `query` string.  A natural-language
/// documentation query is a handful of words; anything past this is malformed
/// or abusive and is rejected before it reaches the tokenizer/embedder.
const MAX_QUERY_BYTES: usize = 4096;

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
    /// When `true` (the default), chunks already returned earlier in this MCP
    /// session are omitted so repeated searches surface fresh material instead
    /// of re-spending the token budget on content the agent has already seen.
    #[serde(default = "default_true")]
    pub fresh_only: bool,
}

/// Parameters for the `resolve_package` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ResolveParams {
    /// A bare library name or fragment (e.g. `tokio`, `react`) to match against
    /// installed package keys.
    pub name: String,
}

/// Parameters for the `search_stack` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchStackParams {
    /// Natural-language or keyword query string.
    pub query: String,
    /// Maximum tokens to return across all results. Defaults to `2000`.
    #[serde(default)]
    pub token_budget: Option<u32>,
    /// Project directory whose manifests declare the stack. Defaults to the
    /// server's working directory.
    #[serde(default)]
    pub project_dir: Option<String>,
}

/// Parameters for the `stack_status` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackStatusParams {
    /// Project directory whose manifests declare the stack. Defaults to the
    /// server's working directory.
    #[serde(default)]
    pub project_dir: Option<String>,
}

/// Parameters for the `version_diff` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct VersionDiffParams {
    /// The older package key (e.g. `cargo-axum@0.7.9`).
    pub old: String,
    /// The newer package key (e.g. `cargo-axum@0.8.9`).
    pub new: String,
}

const fn default_true() -> bool {
    true
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
    /// Cache of open database handles keyed by package key.  `Db` is a cheap
    /// clone over a shared connection thread, so caching avoids re-spawning a
    /// `tokio_rusqlite` thread, re-running the migration check, and discarding
    /// the prepared-statement + page caches on every tool call.
    dbs: Arc<RwLock<HashMap<String, Db>>>,
    /// Node ids already returned this session, keyed by `(package_key, node_id)`,
    /// used to deduplicate `search_docs` results across repeated queries.
    seen: Arc<RwLock<HashSet<(String, i64)>>>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
#[allow(missing_docs)] // #[tool] macro generates undocumented wrapper fns
impl LoreServer {
    /// Construct a new [`LoreServer`].
    fn new_inner(packages_dir: PathBuf, embedder: Embedder) -> Self {
        Self {
            packages_dir,
            embedder: Arc::new(embedder),
            dbs: Arc::new(RwLock::new(HashMap::new())),
            seen: Arc::new(RwLock::new(HashSet::new())),
            tool_router: Self::tool_router(),
        }
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
        check_query_len(&p.query)?;
        let budget = p.token_budget.unwrap_or(2000);
        let mut results = self.run_search(&p.package, &p.query, budget).await?;

        if p.fresh_only {
            results = self.drop_seen_and_record(&p.package, results).await;
            if results.is_empty() {
                return Ok("No new results (all matches already returned this session). Call \
                     reset_session to search from scratch."
                    .into());
            }
        }
        Ok(format_search_results(&results))
    }

    /// Resolve a bare library name to installed package keys.
    #[tool(
        description = "Resolve a bare library name (e.g. 'tokio', 'react') to the installed package key(s) like 'cargo-tokio@1.44.2'. Use this before search_docs when you don't know the exact key."
    )]
    async fn resolve_package(
        &self,
        Parameters(p): Parameters<ResolveParams>,
    ) -> Result<String, rmcp::Error> {
        let packages = scan_packages(&self.packages_dir)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
        let needle = p.name.to_lowercase();
        let matches: Vec<String> = packages
            .into_iter()
            .filter(|(_, meta)| meta.name.to_lowercase().contains(&needle))
            .map(|(key, meta)| {
                let desc = meta.description.as_deref().unwrap_or("");
                if desc.is_empty() { format!("- {key}") } else { format!("- {key}: {desc}") }
            })
            .collect();
        if matches.is_empty() {
            Ok(format!("No installed package matches '{}'.", p.name))
        } else {
            Ok(matches.join("\n"))
        }
    }

    /// Search across every installed package matching the project's declared
    /// dependencies, ranked together.
    #[tool(
        description = "Search across ALL installed packages that match the current project's declared dependencies (from Cargo.toml/package.json/pyproject.toml), ranked together. Use when you don't know which library holds the answer."
    )]
    async fn search_stack(
        &self,
        Parameters(p): Parameters<SearchStackParams>,
    ) -> Result<String, rmcp::Error> {
        check_query_len(&p.query)?;
        let budget = p.token_budget.unwrap_or(2000);
        let keys = self.stack_package_keys(p.project_dir.as_deref()).await?;
        if keys.is_empty() {
            return Ok("No installed packages match this project's dependencies. Run \
                 `lore list` and `lore add <pkg>` to install the ones you need."
                .into());
        }

        // Search each matching package with a generous per-package budget, then
        // merge and trim to the global budget so the best hits win regardless of
        // which library they came from.
        let mut all: Vec<(String, SearchResult)> = Vec::new();
        for key in &keys {
            if let Ok(results) = self.run_search(key, &p.query, budget).await {
                for r in results {
                    all.push((key.clone(), r));
                }
            }
        }
        all.sort_by(|a, b| b.1.score.partial_cmp(&a.1.score).unwrap_or(std::cmp::Ordering::Equal));

        let mut out = Vec::new();
        let mut total: u32 = 0;
        for (key, r) in all {
            let next = total.saturating_add(r.node.token_count);
            if !out.is_empty() && next > budget {
                break;
            }
            total = next;
            out.push(format_stack_result(&key, &r));
        }
        if out.is_empty() {
            Ok("No results found across the project's packages.".into())
        } else {
            Ok(out.join("\n\n"))
        }
    }

    /// Report drift between the project's declared dependency versions and the
    /// installed documentation packages.
    #[tool(
        description = "Report which of the project's dependencies have matching installed docs, which are missing, and where the indexed doc version differs from the declared dependency version."
    )]
    async fn stack_status(
        &self,
        Parameters(p): Parameters<StackStatusParams>,
    ) -> Result<String, rmcp::Error> {
        let dir = resolve_project_dir(p.project_dir.as_deref());
        let deps = stack::detect_dependencies(&dir);
        if deps.is_empty() {
            return Ok(format!("No dependencies detected in {}.", dir.display()));
        }
        let installed = scan_packages(&self.packages_dir)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;

        let mut lines = Vec::new();
        for dep in deps {
            let matched = installed
                .iter()
                .find(|(_, m)| m.registry == dep.registry && m.name == dep.name);
            match matched {
                Some((key, meta)) => {
                    let want = dep.version.as_deref().unwrap_or("*");
                    let drift = dep
                        .version
                        .as_deref()
                        .is_some_and(|v| !version_matches(v, &meta.version));
                    let flag = if drift { "  ⚠ indexed docs may be stale" } else { "" };
                    lines.push(format!(
                        "✓ {}/{} (want {want}, indexed {}) → {key}{flag}",
                        dep.registry, dep.name, meta.version
                    ));
                }
                None => lines.push(format!(
                    "✗ {}/{} — no docs installed (try `lore add {}`)",
                    dep.registry, dep.name, dep.name
                )),
            }
        }
        Ok(lines.join("\n"))
    }

    /// Report API changes between two installed package versions.
    #[tool(
        description = "Diff the API surface of two installed package versions (e.g. cargo-axum@0.7.9 vs cargo-axum@0.8.9). Reports items added, removed, and changed — use to answer 'what changed between these versions?' and to spot breaking changes."
    )]
    async fn version_diff(
        &self,
        Parameters(p): Parameters<VersionDiffParams>,
    ) -> Result<String, rmcp::Error> {
        use std::fmt::Write as _;
        let old_db = self.open_db(&p.old).await?;
        let new_db = self.open_db(&p.new).await?;
        let old_api = lore_build::api_surface(&old_db)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
        let new_api = lore_build::api_surface(&new_db)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
        let diff = lore_build::diff_api(&old_api, &new_api);

        if diff.added.is_empty() && diff.removed.is_empty() && diff.changed.is_empty() {
            return Ok(format!("No API differences detected between {} and {}.", p.old, p.new));
        }
        let mut out = format!("API diff {} → {}:\n", p.old, p.new);
        for k in &diff.removed {
            let _ = writeln!(out, "- REMOVED {k}");
        }
        for k in &diff.added {
            let _ = writeln!(out, "+ ADDED   {k}");
        }
        for (k, from, to) in &diff.changed {
            let _ = writeln!(out, "~ CHANGED {k}\n    was: {from}\n    now: {to}");
        }
        Ok(out)
    }

    /// Clear the session's returned-chunk memory so `search_docs`/`search_stack`
    /// can surface previously-seen results again.
    #[tool(
        description = "Reset the per-session dedup memory so previously-returned documentation chunks can appear again. Call when starting a new task."
    )]
    async fn reset_session(&self) -> Result<String, rmcp::Error> {
        self.seen.write().await.clear();
        Ok("Session memory cleared.".into())
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
    ///
    /// The key is validated against [`lore_core::validate_package_key`] before
    /// being turned into a path — the `package` parameter is model-driven MCP
    /// input and must never be able to escape `packages_dir`.  The database
    /// must already exist; a missing file reports "not installed" rather than
    /// silently creating an empty one.
    async fn open_db(&self, package_key: &str) -> Result<Db, rmcp::Error> {
        lore_core::validate_package_key(package_key)
            .map_err(|e| rmcp::Error::invalid_params(e.to_string(), None))?;

        // Fast path: an already-open handle for this package.
        if let Some(db) = self.dbs.read().await.get(package_key) {
            return Ok(db.clone());
        }

        let path = self.packages_dir.join(format!("{package_key}.db"));
        if !path.is_file() {
            return Err(rmcp::Error::invalid_params(
                format!("package '{package_key}' is not installed — run `lore add {package_key}`"),
                None,
            ));
        }
        let db = Db::open(&path).await.map_err(|_| {
            rmcp::Error::invalid_params(
                format!("package '{package_key}' is not installed — run `lore add {package_key}`"),
                None,
            )
        })?;

        // Populate the cache; a concurrent opener may have won the race, in
        // which case both handles point at the same file — either is fine.
        self.dbs.write().await.entry(package_key.to_owned()).or_insert_with(|| db.clone());
        Ok(db)
    }

    /// Open a package, embed the query, and run the hybrid search pipeline.
    /// Shared by `search_docs` and `search_stack`.
    async fn run_search(
        &self,
        package_key: &str,
        query: &str,
        budget: u32,
    ) -> Result<Vec<SearchResult>, rmcp::Error> {
        let db = self.open_db(package_key).await?;
        let embedder = self.embedder.clone();
        let owned = query.to_owned();
        let embedding = tokio::task::spawn_blocking(move || embedder.embed(&owned))
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
        let config = SearchConfig { token_budget: budget, ..SearchConfig::default() };
        lore_search::search(&db, query, &embedding, &config)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))
    }

    /// Drop results whose `(package, node_id)` was already returned this
    /// session, then record the survivors as seen.
    async fn drop_seen_and_record(
        &self,
        package_key: &str,
        results: Vec<SearchResult>,
    ) -> Vec<SearchResult> {
        let mut seen = self.seen.write().await;
        results
            .into_iter()
            .filter(|r| seen.insert((package_key.to_owned(), r.node.id)))
            .collect()
    }

    /// Installed package keys whose `(registry, name)` matches a dependency
    /// declared by the project's manifests.
    async fn stack_package_keys(
        &self,
        project_dir: Option<&str>,
    ) -> Result<Vec<String>, rmcp::Error> {
        let dir = resolve_project_dir(project_dir);
        let deps = stack::detect_dependencies(&dir);
        let installed = scan_packages(&self.packages_dir)
            .await
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
        Ok(installed
            .into_iter()
            .filter(|(_, meta)| {
                deps.iter().any(|d| d.registry == meta.registry && d.name == meta.name)
            })
            .map(|(key, _)| key)
            .collect())
    }
}

/// Validate the length of an MCP query string.
fn check_query_len(query: &str) -> Result<(), rmcp::Error> {
    if query.len() > MAX_QUERY_BYTES {
        return Err(rmcp::Error::invalid_params(
            format!("query too long ({} bytes; max {MAX_QUERY_BYTES})", query.len()),
            None,
        ));
    }
    Ok(())
}

/// Resolve the project directory for stack scoping: the given path, or the
/// server's current working directory.
fn resolve_project_dir(project_dir: Option<&str>) -> PathBuf {
    project_dir.map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        PathBuf::from,
    )
}

/// Loose version-equality check: exact match, or the declared requirement is a
/// prefix of the indexed version (so `1.44` matches indexed `1.44.2`). Common
/// requirement sigils are stripped first.
fn version_matches(declared: &str, indexed: &str) -> bool {
    let d = declared.trim_start_matches(['^', '~', '=', '>', '<', ' ']);
    if d.is_empty() || d == "*" {
        return true;
    }
    indexed == d || indexed.starts_with(&format!("{d}."))
}

/// Format a federated (`search_stack`) result, prefixing the source package.
fn format_stack_result(package_key: &str, r: &SearchResult) -> String {
    let path = if r.heading_path.is_empty() {
        r.doc_title.clone()
    } else {
        format!("{} › {}", r.doc_title, r.heading_path.join(" › "))
    };
    let content = r.node.content.as_deref().unwrap_or("").trim();
    format!("[{package_key}] (id={}, score={:.3})\n{}\n{}", r.node.id, r.score, path, content)
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
