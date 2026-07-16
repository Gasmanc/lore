#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `lore` command-line interface.
//!
//! Subcommands:
//! - `add`      — install a package from the registry
//! - `remove`   — remove an installed package
//! - `list`     — list installed packages
//! - `search`   — hybrid search across an installed package
//! - `build`    — build a package from a local source directory
//! - `update`   — rebuild installed packages from their upstream sources
//! - `manifest` — print the compressed API surface manifest for a package
//! - `info`     — show detailed metadata and statistics for a package
//! - `mcp`      — start the MCP server on stdin/stdout

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use console::style;
use dialoguer::{FuzzySelect, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};
use lore_core::{LoreError, Package, validate_package_key};
use lore_registry::RegistryClient;

/// Maximum number of content characters shown in a search result preview.
const PREVIEW_LEN: usize = 200;

// ── CLI definition ─────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "lore", about = "Local documentation server for AI coding assistants", version)]
struct Cli {
    /// Override the default packages directory (~/.local/share/lore/packages).
    #[arg(long, env = "LORE_PACKAGES_DIR", global = true)]
    packages_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a package from the Lore registry.
    Add {
        /// Package name (e.g. `next`, `react`, `tokio`).
        package: String,
        /// Specific version to install. Defaults to latest.
        #[arg(long, short)]
        version: Option<String>,
    },
    /// Remove an installed package.
    Remove {
        /// Package key (e.g. `npm-next@15.0.0`).
        package: String,
    },
    /// List all locally installed packages.
    List,
    /// Search documentation in an installed package.
    Search {
        /// Package key (e.g. `npm-next@15.0.0`).
        package: String,
        /// Query string.
        query: String,
        /// Maximum tokens to include in results.
        #[arg(long, default_value = "2000")]
        budget: u32,
        /// Keyword-only (BM25) search — skips loading the embedding model for a
        /// ~300 ms → single-digit-ms lookup. Best for exact API-name queries.
        #[arg(long)]
        fast: bool,
    },
    /// Build a package from a local source directory.
    Build {
        /// Directory containing documentation source files.
        source_dir: PathBuf,
        /// Package name.
        #[arg(long)]
        name: String,
        /// Package version.
        #[arg(long)]
        version: String,
        /// Registry identifier (e.g. `npm`, `cargo`, `pypi`). Defaults to `local`.
        #[arg(long, default_value = "local")]
        registry: String,
        /// Output path for the `.db` file. Defaults to `<registry>-<name>@<version>.db`
        /// in the packages directory.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Human-readable description.
        #[arg(long)]
        description: Option<String>,
        /// URL of the upstream source.
        #[arg(long)]
        source_url: Option<String>,
        /// Exclude `examples/`, `tests/`, and similar directories.
        #[arg(long)]
        exclude_examples: bool,
    },
    /// Build a package from a live website (`llms.txt` digest or a crawl).
    ///
    /// By default tries the site's `llms-full.txt` / `llms.txt` first (clean,
    /// LLM-ready Markdown) and falls back to crawling HTML pages. Pass
    /// `--crawl` to force a crawl.
    BuildWebsite {
        /// Site URL (root, or a direct `llms.txt` link).
        url: String,
        /// Package name.
        #[arg(long)]
        name: String,
        /// Package version.
        #[arg(long)]
        version: String,
        /// Registry identifier. Defaults to `web`.
        #[arg(long, default_value = "web")]
        registry: String,
        /// Output path for the `.db`. Defaults to the packages directory.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Human-readable description.
        #[arg(long)]
        description: Option<String>,
        /// Upstream source URL to persist (defaults to `url`).
        #[arg(long)]
        source_url: Option<String>,
        /// Force an HTML crawl instead of trying `llms.txt` first.
        #[arg(long)]
        crawl: bool,
    },
    /// Build a package from a Rust crate's `rustdoc --output-format json`.
    ///
    /// Either point `--json` at an existing rustdoc JSON file, or pass `--crate`
    /// (and optionally `--manifest-dir`) to run `cargo +nightly rustdoc` for a
    /// dependency and index the exact locked version's API.
    BuildRustdoc {
        /// Path to an existing rustdoc JSON file.
        #[arg(long, conflicts_with = "crate_name")]
        json: Option<PathBuf>,
        /// Crate to document via `cargo +nightly rustdoc` (requires nightly).
        #[arg(long = "crate")]
        crate_name: Option<String>,
        /// Directory containing the Cargo.toml (defaults to current directory).
        #[arg(long)]
        manifest_dir: Option<PathBuf>,
        /// Package name to store as. Defaults to the crate name.
        #[arg(long)]
        name: Option<String>,
        /// Package version.
        #[arg(long)]
        version: String,
        /// Registry identifier. Defaults to `cargo`.
        #[arg(long, default_value = "cargo")]
        registry: String,
        /// Output path for the `.db`. Defaults to the packages directory.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Diff the API surface of two installed package versions.
    ///
    /// Reports items added, removed, and changed between two `.db` packages —
    /// most precise on `build-rustdoc` packages, where headings are item paths.
    Diff {
        /// The older package key (e.g. `cargo-axum@0.7.9`).
        old: String,
        /// The newer package key (e.g. `cargo-axum@0.8.9`).
        new: String,
    },
    /// Rebuild installed packages from their upstream sources.
    ///
    /// Re-fetches each package's source (git repository or website), runs the
    /// full build pipeline, and atomically replaces the existing `.db` file.
    /// The old database is never touched until the new one is complete —
    /// a failed rebuild leaves the installed package intact.
    Update {
        /// Packages to update (e.g. `npm-next@15.0.0` or just `next`).
        /// Omit to update every installed package.
        packages: Vec<String>,
        /// Show what would be rebuilt without actually rebuilding.
        #[arg(long)]
        check: bool,
    },

    /// Print the compressed API surface manifest for an installed package.
    ///
    /// The manifest is a ~500-token index of the package's public API,
    /// suitable for pasting into CLAUDE.md as a fingerpost.
    Manifest {
        /// Package key (e.g. `npm-next@15.0.0`).
        package: String,
        /// Copy the manifest to the clipboard (macOS: pbcopy, Linux: xclip/xsel).
        #[arg(long)]
        copy: bool,
    },
    /// Show detailed metadata and statistics for an installed package.
    Info {
        /// Package key (e.g. `npm-next@15.0.0`).
        package: String,
    },
    /// Check installed packages against their upstream registries for newer versions.
    ///
    /// Queries crates.io, npm, and `PyPI` for the latest stable version of each
    /// installed package and prints a table of any drift.  Exit code 1 if any
    /// package is out of date.
    CheckUpdates,
    /// Report indexing + retrieval-quality health for an installed package.
    ///
    /// Prints structural stats and an unsupervised self-retrieval score: for a
    /// sample of sections it queries by heading title and checks whether the
    /// section's own chunk is retrieved, giving a label-free quality signal.
    Doctor {
        /// Package key (e.g. `npm-next@15.0.0`).
        package: String,
    },
    /// Start the MCP server on stdin/stdout (for use by AI coding assistants).
    Mcp,
}

// ── Entry point ────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_env("LORE_LOG")
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let packages_dir = cli.packages_dir.unwrap_or_else(default_packages_dir);

    let result = match cli.command {
        Command::Add { package, version } => cmd_add(package, version, &packages_dir).await,
        Command::Remove { package } => cmd_remove(&package, &packages_dir),
        Command::List => cmd_list(&packages_dir).await,
        Command::Search { package, query, budget, fast } => {
            cmd_search(package, query, budget, fast, &packages_dir).await
        }
        Command::Build {
            source_dir,
            name,
            version,
            registry,
            output,
            description,
            source_url,
            exclude_examples,
        } => {
            let meta = Package { name, version, registry, description, source_url, git_sha: None };
            cmd_build(source_dir, meta, output, exclude_examples, &packages_dir).await
        }
        Command::BuildWebsite {
            url,
            name,
            version,
            registry,
            output,
            description,
            source_url,
            crawl,
        } => {
            let meta = Package {
                name,
                version,
                registry,
                description,
                source_url: Some(source_url.unwrap_or_else(|| url.clone())),
                git_sha: None,
            };
            cmd_build_website(url, meta, output, crawl, &packages_dir).await
        }
        Command::BuildRustdoc { json, crate_name, manifest_dir, name, version, registry, output } => {
            cmd_build_rustdoc(
                json,
                crate_name,
                manifest_dir,
                name,
                version,
                registry,
                output,
                &packages_dir,
            )
            .await
        }
        Command::Diff { old, new } => cmd_diff(old, new, &packages_dir).await,
        Command::Update { packages, check } => cmd_update(packages, check, &packages_dir).await,
        Command::Manifest { package, copy } => cmd_manifest(package, copy, &packages_dir).await,
        Command::Info { package } => cmd_info(package, &packages_dir).await,
        Command::CheckUpdates => cmd_check_updates(&packages_dir).await,
        Command::Doctor { package } => cmd_doctor(package, &packages_dir).await,
        Command::Mcp => cmd_mcp(packages_dir).await,
    };

    if let Err(e) = result {
        eprintln!("{} {e}", style("error:").red().bold());
        std::process::exit(1);
    }
}

// ── Command implementations ────────────────────────────────────────────────────

/// `lore add <package>` — search the registry and download a package.
async fn cmd_add(
    package: String,
    version: Option<String>,
    packages_dir: &std::path::Path,
) -> Result<(), LoreError> {
    let client = RegistryClient::new(RegistryClient::DEFAULT_URL)?;

    let spinner = make_spinner(format!("Searching registry for \"{package}\"…"));
    let search_result = client.search(&package).await;
    spinner.finish_and_clear();
    let mut matches = search_result?;

    if let Some(ref ver) = version {
        matches.retain(|e| &e.metadata.package.version == ver);
    }

    if matches.is_empty() {
        return Err(LoreError::NotFound(format!(
            "no packages matching \"{package}\" found in the registry"
        )));
    }

    // Choose which entry to install.
    let entry = if matches.len() == 1 {
        matches.remove(0)
    } else {
        let labels: Vec<String> = matches
            .iter()
            .map(|e| {
                let key = e.metadata.package.display_key();
                let desc = e.metadata.package.description.as_deref().unwrap_or("");
                if desc.is_empty() { key } else { format!("{key} — {desc}") }
            })
            .collect();
        // FuzzySelect::interact() is blocking — run it off the async reactor.
        let idx: usize = tokio::task::spawn_blocking(move || {
            FuzzySelect::with_theme(&ColorfulTheme::default())
                .with_prompt("Select a package to install")
                .items(&labels)
                .default(0)
                .interact()
        })
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))?
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))?;
        matches.remove(idx)
    };

    // The entry comes from the registry index — an untrusted source. Validate
    // every identity component before turning it into a filesystem path.
    entry.metadata.package.validate()?;
    let key = entry.metadata.package.display_key();
    std::fs::create_dir_all(packages_dir).map_err(LoreError::Io)?;
    let target = packages_dir.join(format!("{key}.db"));

    let pb = ProgressBar::new_spinner();
    println!("Downloading {}…", style(&key).bold());
    client.download(&entry, &target, Some(&pb)).await?;

    println!("{} Installed {}", style("✓").green().bold(), style(&key).bold());
    Ok(())
}

/// `lore remove <package>` — deletes the package `.db` file.
fn cmd_remove(package: &str, packages_dir: &std::path::Path) -> Result<(), LoreError> {
    let path = package_db_path(packages_dir, package)?;
    match std::fs::remove_file(&path) {
        Ok(()) => println!("Removed {package}."),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(LoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("package '{package}' is not installed"),
            )));
        }
        Err(e) => return Err(LoreError::Io(e)),
    }
    Ok(())
}

/// `lore list` — prints all installed packages.
async fn cmd_list(packages_dir: &std::path::Path) -> Result<(), LoreError> {
    let packages = lore_mcp::scan_packages(packages_dir).await?;
    if packages.is_empty() {
        println!("No packages installed.");
        println!("Use `lore build` to add a package from local source.");
        return Ok(());
    }
    for (key, meta) in &packages {
        let desc = meta.description.as_deref().unwrap_or("");
        if desc.is_empty() {
            println!("{}", style(key).bold());
        } else {
            println!("{} — {desc}", style(key).bold());
        }
    }
    Ok(())
}

/// `lore search <package> <query>` — runs the search pipeline.
async fn cmd_search(
    package: String,
    query: String,
    budget: u32,
    fast: bool,
    packages_dir: &std::path::Path,
) -> Result<(), LoreError> {
    let db = open_installed(packages_dir, &package).await?;
    let config = lore_core::SearchConfig { token_budget: budget, ..Default::default() };

    let results = if fast {
        // Keyword-only: no embedding model load (~300 ms saved per invocation).
        lore_search::search_keyword(&db, &query, &config).await?
    } else {
        let cache = lore_mcp::model_cache_dir();
        let embedder = tokio::task::spawn_blocking(move || lore_build::Embedder::new(&cache))
            .await
            .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))??;
        let embedding = embedder.embed(&query)?;
        lore_search::search(&db, &query, &embedding, &config).await?
    };

    if results.is_empty() {
        println!("No results found.");
        return Ok(());
    }

    for (i, r) in results.iter().enumerate() {
        let heading = if r.heading_path.is_empty() {
            r.doc_title.clone()
        } else {
            format!("{} › {}", r.doc_title, r.heading_path.join(" › "))
        };
        println!(
            "{} {} (score {:.3})",
            style(format!("[{}]", i + 1)).cyan().bold(),
            style(&heading).bold(),
            r.score,
        );
        if let Some(content) = &r.node.content {
            let preview = content.trim();
            let preview = if preview.len() > PREVIEW_LEN {
                // Find the last valid UTF-8 char boundary at or before PREVIEW_LEN
                // to avoid panicking on multibyte chars (CJK, emoji, etc.)
                let boundary =
                    (0..=PREVIEW_LEN).rev().find(|&i| preview.is_char_boundary(i)).unwrap_or(0);
                format!("{}…", &preview[..boundary])
            } else {
                preview.to_owned()
            };
            println!("{preview}");
        }
        println!();
    }
    Ok(())
}

/// `lore build` — builds a package from a local source directory.
async fn cmd_build(
    source_dir: PathBuf,
    meta: Package,
    output: Option<PathBuf>,
    exclude_examples: bool,
    packages_dir: &std::path::Path,
) -> Result<(), LoreError> {
    meta.validate()?;
    let display_key = meta.display_key();
    let output_path = output.unwrap_or_else(|| packages_dir.join(format!("{display_key}.db")));

    std::fs::create_dir_all(packages_dir).map_err(LoreError::Io)?;

    let spinner = make_spinner(format!("Building {display_key}…"));

    let cache = lore_mcp::model_cache_dir();
    let builder = tokio::task::spawn_blocking(move || lore_build::PackageBuilder::new(&cache))
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))??;

    // Build into a temp file and atomically rename on success, so a failed or
    // interrupted build never leaves a half-written package at `output_path`
    // (mirrors `lore update`'s rebuild path).
    let tmp_path = output_path.with_extension("db.building");
    let _ = std::fs::remove_file(&tmp_path);

    let meta_ref = meta.clone();
    let stats = match builder.build(&source_dir, meta, &tmp_path, exclude_examples).await {
        Ok(stats) => stats,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
    };
    std::fs::rename(&tmp_path, &output_path).map_err(LoreError::Io)?;

    spinner.finish_and_clear();

    // Write the JSON manifest sidecar so registry tooling can read build stats.
    let manifest_path = lore_build::write_manifest(&output_path, &meta_ref, &stats)
        .map_err(|e| {
            tracing::warn!(error = %e, "manifest write failed (non-fatal)");
            e
        })
        .ok();

    println!(
        "{} Built {} → {}",
        style("✓").green().bold(),
        style(&display_key).bold(),
        output_path.display(),
    );
    if let Some(mp) = manifest_path {
        println!("    manifest: {}", mp.display());
    }
    println!("{}", stats.summary());
    Ok(())
}

/// `lore build-website <url>` — build a package from a live website.
///
/// Prefers the site's `llms.txt` / `llms-full.txt` digest; falls back to an
/// HTML crawl (or goes straight to crawl with `--crawl`). Writes atomically.
async fn cmd_build_website(
    url: String,
    meta: Package,
    output: Option<PathBuf>,
    crawl: bool,
    packages_dir: &std::path::Path,
) -> Result<(), LoreError> {
    use lore_build::Source as _;

    meta.validate()?;
    let display_key = meta.display_key();
    let output_path = output.unwrap_or_else(|| packages_dir.join(format!("{display_key}.db")));
    std::fs::create_dir_all(packages_dir).map_err(LoreError::Io)?;

    let spinner = make_spinner(format!("Fetching {url}…"));
    let prepared = if crawl {
        lore_build::WebsiteSource::new(&url).prepare().await?
    } else {
        // Try the llms.txt digest first; fall back to a crawl.
        match lore_build::LlmsTxtSource::new(&url).prepare().await {
            Ok(p) => p,
            Err(e) => {
                tracing::info!(error = %e, "no llms.txt; falling back to crawl");
                lore_build::WebsiteSource::new(&url).prepare().await?
            }
        }
    };
    spinner.set_message(format!("Building {display_key}…"));

    let cache = lore_mcp::model_cache_dir();
    let builder = tokio::task::spawn_blocking(move || lore_build::PackageBuilder::new(&cache))
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))??;

    let tmp_path = output_path.with_extension("db.building");
    let _ = std::fs::remove_file(&tmp_path);
    let meta_ref = meta.clone();
    let stats = match builder.build(&prepared.dir, meta, &tmp_path, false).await {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
    };
    std::fs::rename(&tmp_path, &output_path).map_err(LoreError::Io)?;
    spinner.finish_and_clear();

    let _ = lore_build::write_manifest(&output_path, &meta_ref, &stats);
    println!(
        "{} Built {} → {}",
        style("✓").green().bold(),
        style(&display_key).bold(),
        output_path.display()
    );
    println!("{}", stats.summary());
    Ok(())
}

/// `lore build-rustdoc` — build a package from rustdoc JSON.
#[allow(clippy::too_many_arguments)] // each flag is a distinct, independent input
async fn cmd_build_rustdoc(
    json: Option<PathBuf>,
    crate_name: Option<String>,
    manifest_dir: Option<PathBuf>,
    name: Option<String>,
    version: String,
    registry: String,
    output: Option<PathBuf>,
    packages_dir: &std::path::Path,
) -> Result<(), LoreError> {
    use lore_build::Source as _;

    let pkg_name = name
        .or_else(|| crate_name.clone())
        .or_else(|| {
            json.as_ref().and_then(|p| p.file_stem()).and_then(|s| s.to_str()).map(str::to_owned)
        })
        .ok_or_else(|| LoreError::InvalidConfig("provide --name, --crate, or --json".into()))?;

    let meta = Package {
        name: pkg_name,
        version,
        registry,
        description: None,
        source_url: None,
        git_sha: None,
    };
    meta.validate()?;
    let display_key = meta.display_key();
    let output_path = output.unwrap_or_else(|| packages_dir.join(format!("{display_key}.db")));
    std::fs::create_dir_all(packages_dir).map_err(LoreError::Io)?;

    let source = match (json, crate_name) {
        (Some(path), _) => lore_build::RustdocSource::from_json(path),
        (None, Some(c)) => lore_build::RustdocSource::from_crate(c, manifest_dir.unwrap_or_default()),
        (None, None) => {
            return Err(LoreError::InvalidConfig("provide either --json or --crate".into()));
        }
    };

    let spinner = make_spinner(format!("Generating rustdoc for {display_key}…"));
    let prepared = source.prepare().await?;
    spinner.set_message(format!("Building {display_key}…"));

    let cache = lore_mcp::model_cache_dir();
    let builder = tokio::task::spawn_blocking(move || lore_build::PackageBuilder::new(&cache))
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))??;

    let tmp_path = output_path.with_extension("db.building");
    let _ = std::fs::remove_file(&tmp_path);
    let meta_ref = meta.clone();
    let stats = match builder.build(&prepared.dir, meta, &tmp_path, false).await {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
    };
    std::fs::rename(&tmp_path, &output_path).map_err(LoreError::Io)?;
    spinner.finish_and_clear();

    let _ = lore_build::write_manifest(&output_path, &meta_ref, &stats);
    println!(
        "{} Built {} → {}",
        style("✓").green().bold(),
        style(&display_key).bold(),
        output_path.display()
    );
    println!("{}", stats.summary());
    Ok(())
}

/// `lore diff <old> <new>` — report API changes between two package versions.
async fn cmd_diff(
    old: String,
    new: String,
    packages_dir: &std::path::Path,
) -> Result<(), LoreError> {
    let old_db = open_installed(packages_dir, &old).await?;
    let new_db = open_installed(packages_dir, &new).await?;

    let old_api = lore_build::api_surface(&old_db).await?;
    let new_api = lore_build::api_surface(&new_db).await?;

    let diff = lore_build::diff_api(&old_api, &new_api);
    if diff.added.is_empty() && diff.removed.is_empty() && diff.changed.is_empty() {
        println!("No API differences detected between {old} and {new}.");
        return Ok(());
    }

    println!("{}", style(format!("API diff: {old} → {new}")).bold());
    if !diff.removed.is_empty() {
        println!("\n{} Removed ({})", style("−").red().bold(), diff.removed.len());
        for item in &diff.removed {
            println!("  {} {item}", style("−").red());
        }
    }
    if !diff.added.is_empty() {
        println!("\n{} Added ({})", style("+").green().bold(), diff.added.len());
        for item in &diff.added {
            println!("  {} {item}", style("+").green());
        }
    }
    if !diff.changed.is_empty() {
        println!("\n{} Changed ({})", style("~").yellow().bold(), diff.changed.len());
        for (item, from, to) in &diff.changed {
            println!("  {} {item}", style("~").yellow());
            println!("      {} {from}", style("was:").dim());
            println!("      {} {to}", style("now:").dim());
        }
    }
    if !diff.removed.is_empty() || !diff.changed.is_empty() {
        println!(
            "\n{} removals and signature changes are potential breaking changes.",
            style("⚠").yellow().bold()
        );
    }
    Ok(())
}

/// `lore manifest <package>` — prints the compressed API surface manifest.
async fn cmd_manifest(
    package: String,
    copy: bool,
    packages_dir: &std::path::Path,
) -> Result<(), LoreError> {
    let db = open_installed(packages_dir, &package).await?;

    let manifest =
        db.get_meta("manifest".to_owned()).await?.filter(|m| !m.is_empty()).ok_or_else(|| {
            LoreError::NotFound(format!(
                "package '{package}' has no manifest — rebuild with `lore build`"
            ))
        })?;

    if copy {
        // Try pbcopy (macOS), then xclip, then xsel.
        let copied = try_copy_to_clipboard(&manifest);
        if copied {
            println!("{manifest}");
            println!("{} Copied to clipboard", style("✓").green().bold());
        } else {
            eprintln!(
                "{} clipboard copy failed (pbcopy/xclip/xsel not found) — printing to stdout",
                style("warning:").yellow().bold()
            );
            println!("{manifest}");
        }
    } else {
        println!("{manifest}");
    }

    Ok(())
}

/// `lore info <package>` — shows detailed package metadata and statistics.
async fn cmd_info(package: String, packages_dir: &std::path::Path) -> Result<(), LoreError> {
    let path = package_db_path(packages_dir, &package)?;
    let db = open_installed(packages_dir, &package).await?;

    let meta = db.get_package_meta().await?;

    // File size.
    let size_bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
    let size_display = format_bytes(size_bytes);

    // Node counts by kind.
    let chunk_count = db.get_nodes_by_kind(lore_core::NodeKind::Chunk).await?.len();
    let code_block_count = db.get_nodes_by_kind(lore_core::NodeKind::CodeBlock).await?.len();
    let heading_count = db.get_nodes_by_kind(lore_core::NodeKind::Heading).await?.len();

    // Build date from meta.
    let build_date = db.get_meta("build_date".to_owned()).await?.unwrap_or_else(|| "—".into());

    println!("{}", style(format!("Package: {}", meta.display_key())).bold());
    println!("  Name:        {}", meta.name);
    println!("  Registry:    {}", meta.registry);
    println!("  Version:     {}", meta.version);
    if let Some(desc) = &meta.description {
        println!("  Description: {desc}");
    }
    if let Some(url) = &meta.source_url {
        println!("  Source URL:  {url}");
    }
    if let Some(sha) = &meta.git_sha {
        println!("  Git SHA:     {sha}");
    }
    println!("  Build Date:  {build_date}");
    println!("  File Size:   {size_display}");
    println!("  Chunks:      {chunk_count}");
    println!("  Code Blocks: {code_block_count}");
    println!("  Headings:    {heading_count}");

    Ok(())
}

/// `lore check-updates` — query upstream registries for newer versions of installed packages.
///
/// Returns `Err` (exit code 1) if any package is out of date so the command
/// composes cleanly with `launchd`/cron and macOS notification scripts.
async fn cmd_check_updates(packages_dir: &std::path::Path) -> Result<(), LoreError> {
    let installed = lore_mcp::scan_packages(packages_dir).await?;
    if installed.is_empty() {
        println!("No packages installed.");
        return Ok(());
    }

    let http = lore_registry::default_http_client()?;

    let mut rows: Vec<(String, String, String, String)> = Vec::new(); // (key, current, latest, status)
    let mut n_stale: u32 = 0;
    let mut n_unknown: u32 = 0;

    for (key, meta) in &installed {
        let latest =
            lore_registry::fetch_latest_upstream_version(&http, &meta.registry, &meta.name).await;
        let (latest_str, status) = match latest {
            Ok(Some(v)) if v == meta.version => (v, style("up to date").dim().to_string()),
            Ok(Some(v)) => {
                n_stale += 1;
                (v, style("UPDATE").yellow().bold().to_string())
            }
            Ok(None) => {
                n_unknown += 1;
                ("—".to_owned(), style("unsupported registry").dim().to_string())
            }
            Err(e) => {
                n_unknown += 1;
                ("?".to_owned(), style(format!("error: {e}")).red().to_string())
            }
        };
        rows.push((key.clone(), meta.version.clone(), latest_str, status));
    }

    let key_w = rows.iter().map(|r| r.0.len()).max().unwrap_or(0).max(7);
    let cur_w = rows.iter().map(|r| r.1.len()).max().unwrap_or(0).max(7);
    let lat_w = rows.iter().map(|r| r.2.len()).max().unwrap_or(0).max(6);

    println!(
        "{:<key_w$}  {:<cur_w$}  {:<lat_w$}  {}",
        style("Package").bold(),
        style("Current").bold(),
        style("Latest").bold(),
        style("Status").bold(),
    );
    for (key, current, latest, status) in &rows {
        println!("{key:<key_w$}  {current:<cur_w$}  {latest:<lat_w$}  {status}");
    }

    println!();
    if n_stale > 0 {
        println!(
            "{} {n_stale} package(s) out of date — bump the version field in the spec and push.",
            style("⚠").yellow().bold()
        );
        return Err(LoreError::Registry(format!("{n_stale} package(s) out of date")));
    }
    if n_unknown > 0 {
        println!("{n_unknown} package(s) could not be checked.");
    } else {
        println!("{} All packages up to date.", style("✓").green().bold());
    }
    Ok(())
}

/// `lore mcp` — starts the MCP server on stdio.
async fn cmd_mcp(packages_dir: PathBuf) -> Result<(), LoreError> {
    lore_mcp::serve_stdio(packages_dir).await
}

/// Maximum number of sections sampled for the self-retrieval quality score.
const DOCTOR_SAMPLE: usize = 40;

/// `lore doctor <package>` — structural stats + unsupervised retrieval quality.
#[allow(clippy::cast_precision_loss)] // percentages/MRR are cosmetic display math
async fn cmd_doctor(package: String, packages_dir: &std::path::Path) -> Result<(), LoreError> {
    let db = open_installed(packages_dir, &package).await?;

    let chunks = db.get_nodes_by_kind(lore_core::NodeKind::Chunk).await?;
    let code_blocks = db.get_nodes_by_kind(lore_core::NodeKind::CodeBlock).await?;
    let headings = db.get_nodes_by_kind(lore_core::NodeKind::Heading).await?;

    // Structural health: how many chunks exceed the embedding model's ~512-token
    // input window and are therefore truncated at embed time.
    let oversized = chunks.iter().filter(|n| n.token_count > 512).count();
    let total_content = chunks.len() + code_blocks.len();

    println!("{}", style(format!("Doctor report: {package}")).bold());
    println!("  Chunks:       {}", chunks.len());
    println!("  Code blocks:  {}", code_blocks.len());
    println!("  Headings:     {}", headings.len());
    if total_content > 0 {
        let pct = (oversized as f64 / total_content as f64) * 100.0;
        println!("  Oversized:    {oversized} chunks > 512 tokens ({pct:.1}% — truncated at embed)");
    }

    // Self-retrieval: query each sampled chunk by a short prefix of its own
    // content and check whether that chunk comes back near the top. A low score
    // means the index is over-chunked, noisy, or the relevance filter is
    // dropping legitimate matches — all things that hurt real queries too.
    let sample: Vec<_> = chunks
        .iter()
        .filter(|n| n.content.as_deref().is_some_and(|c| c.split_whitespace().count() >= 4))
        .step_by(1.max(chunks.len() / DOCTOR_SAMPLE))
        .take(DOCTOR_SAMPLE)
        .collect();

    if sample.is_empty() {
        println!("\n  Not enough content to compute a retrieval score.");
        return Ok(());
    }

    let cache = lore_mcp::model_cache_dir();
    let embedder = tokio::task::spawn_blocking(move || lore_build::Embedder::new(&cache))
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))??;
    let config = lore_core::SearchConfig::default();

    let (mut rr_sum, mut hit1, mut hit3) = (0.0_f64, 0u32, 0u32);
    for node in &sample {
        let Some(content) = node.content.as_deref() else { continue };
        // Query with the first ~10 words — enough to be specific without being
        // the whole chunk verbatim.
        let query: String = content.split_whitespace().take(10).collect::<Vec<_>>().join(" ");
        let embedding = embedder.embed(&query)?;
        let results = lore_search::search(&db, &query, &embedding, &config).await?;
        if let Some(rank) = results.iter().position(|r| r.node.id == node.id) {
            rr_sum += 1.0 / (rank as f64 + 1.0);
            if rank == 0 {
                hit1 += 1;
            }
            if rank < 3 {
                hit3 += 1;
            }
        }
    }
    let n = sample.len() as f64;
    println!("\n  {} ({} sampled sections)", style("Self-retrieval quality").bold(), sample.len());
    println!("    MRR:    {:.3}", rr_sum / n);
    println!("    Hit@1:  {hit1}/{} ({:.0}%)", sample.len(), f64::from(hit1) / n * 100.0);
    println!("    Hit@3:  {hit3}/{} ({:.0}%)", sample.len(), f64::from(hit3) / n * 100.0);
    if rr_sum / n < 0.5 {
        println!(
            "  {} low self-retrieval — the index may be over-chunked or noisy.",
            style("⚠").yellow().bold()
        );
    }
    Ok(())
}

/// `lore update [packages] [--check]` — rebuild installed packages from their upstream sources.
///
/// For each package the update pipeline is:
/// 1. Read `source_url` and `git_sha` from the installed package's `meta` table.
/// 2. Determine the source type (git repository or website crawl).
/// 3. Fetch/clone the source into a temporary directory.
/// 4. Run the full build pipeline, writing output to `<key>.db.tmp`.
/// 5. Atomically rename `<key>.db.tmp` → `<key>.db`.
///
/// On any failure the `.tmp` file is removed and the existing `.db` is left intact.
/// Per-package failures are reported and do not abort the remaining updates.
async fn cmd_update(
    packages: Vec<String>,
    check: bool,
    packages_dir: &std::path::Path,
) -> Result<(), LoreError> {
    let installed = lore_mcp::scan_packages(packages_dir).await?;

    if installed.is_empty() {
        println!("No packages installed. Use `lore add` or `lore build` first.");
        return Ok(());
    }

    // Filter to the requested subset (match on full key or bare name).
    let to_update: Vec<_> = if packages.is_empty() {
        installed
    } else {
        installed
            .into_iter()
            .filter(|(key, meta)| packages.iter().any(|p| key == p || meta.name == *p))
            .collect()
    };

    if to_update.is_empty() {
        return Err(LoreError::NotFound(format!(
            "no installed packages match: {}",
            packages.join(", ")
        )));
    }

    if check {
        println!("Packages that would be rebuilt:\n");
        for (key, meta) in &to_update {
            let src = update_source_description(meta);
            println!("  {} — {src}", style(key).bold());
        }
        println!("\n{} package(s) total (dry run — nothing changed)", to_update.len());
        return Ok(());
    }

    // Initialise the builder once; the embedding model is shared across all rebuilds.
    let cache = lore_mcp::model_cache_dir();
    let spinner = make_spinner("Loading embedding model…");
    let builder = tokio::task::spawn_blocking(move || lore_build::PackageBuilder::new(&cache))
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))??;
    spinner.finish_and_clear();

    let mut n_updated: u32 = 0;
    let mut n_skipped: u32 = 0;
    let mut n_failed: u32 = 0;

    for (key, meta) in &to_update {
        let Some(source_url) = meta.source_url.clone() else {
            println!(
                "  {} {} — skipped (no remote source; use `lore build <dir>` to rebuild)",
                style("⟳").yellow().bold(),
                style(key).bold()
            );
            n_skipped += 1;
            continue;
        };

        let spinner = make_spinner(format!("Updating {key}…"));
        match rebuild_package(&builder, meta, &source_url, key, packages_dir).await {
            Ok(stats) => {
                spinner.finish_and_clear();
                println!(
                    "  {} {} — {}",
                    style("✓").green().bold(),
                    style(key).bold(),
                    stats.summary()
                );
                n_updated += 1;
            }
            Err(e) => {
                spinner.finish_and_clear();
                eprintln!("  {} {} — {e}", style("✗").red().bold(), style(key).bold());
                n_failed += 1;
            }
        }
    }

    println!();
    println!("Updated: {n_updated}  Skipped: {n_skipped}  Failed: {n_failed}");

    if n_failed > 0 {
        Err(LoreError::Registry(format!("{n_failed} package(s) failed to update")))
    } else {
        Ok(())
    }
}

/// Returns a human-readable description of where a package's source lives.
fn update_source_description(meta: &lore_core::Package) -> String {
    match &meta.source_url {
        Some(url) if looks_like_git_url(url) => {
            format!("git {url}")
        }
        Some(url) => format!("website {url}"),
        None => "no remote source".to_owned(),
    }
}

/// Returns `true` if `url` looks like a git repository URL.
///
/// Matches `https://{github,gitlab,bitbucket}.*`, URLs ending with `.git`,
/// and `git://` / `git@` schemes.
#[allow(clippy::case_sensitive_file_extension_comparisons)] // git URLs use lowercase `.git`
fn looks_like_git_url(url: &str) -> bool {
    url.ends_with(".git")
        || url.starts_with("git://")
        || url.starts_with("git@")
        || ["github.com", "gitlab.com", "bitbucket.org", "codeberg.org", "sr.ht"]
            .iter()
            .any(|h| url.contains(h))
}

/// Fetch the source, run the build pipeline, and atomically replace the `.db`.
///
/// Writes to `<key>.db.tmp` first.  On success the tmp file is renamed over
/// the live `.db`.  On any error the tmp file is removed and the error is
/// returned — the existing `.db` is never corrupted.
async fn rebuild_package(
    builder: &lore_build::PackageBuilder,
    meta: &lore_core::Package,
    source_url: &str,
    key: &str,
    packages_dir: &std::path::Path,
) -> Result<lore_build::BuildStats, LoreError> {
    // Bring the Source trait into scope so `.prepare()` is callable.
    use lore_build::Source as _;

    // Materialise the source into a temporary directory.
    let prepared = if looks_like_git_url(source_url) {
        lore_build::GitSource::new(source_url).prepare().await?
    } else {
        lore_build::WebsiteSource::new(source_url).prepare().await?
    };

    let source_dir = prepared.dir.clone();
    let new_sha = prepared.git_sha.clone();

    // Build to a tmp path so the live .db is never half-written.
    let live_path = packages_dir.join(format!("{key}.db"));
    let tmp_path = packages_dir.join(format!("{key}.db.tmp"));

    // Update meta with the new git SHA if we got one.
    let mut updated_meta = meta.clone();
    if new_sha.is_some() {
        updated_meta.git_sha = new_sha;
    }

    let result = builder.build(&source_dir, updated_meta, &tmp_path, false).await;

    match result {
        Ok(stats) => {
            // Atomic rename — on POSIX this is guaranteed atomic.
            tokio::fs::rename(&tmp_path, &live_path).await.map_err(|e| {
                // Best-effort cleanup on rename failure.
                let _ = std::fs::remove_file(&tmp_path);
                LoreError::Io(e)
            })?;
            Ok(stats)
        }
        Err(e) => {
            // Clean up the partial tmp file; leave the live .db untouched.
            let _ = std::fs::remove_file(&tmp_path);
            Err(e)
        }
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Attempts to copy `text` to the system clipboard.
///
/// Tries `pbcopy` (macOS), then `xclip`, then `xsel` in order.
/// Returns `true` if the copy succeeded.
fn try_copy_to_clipboard(text: &str) -> bool {
    let tools: &[(&str, &[&str])] = &[
        ("pbcopy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];

    for (tool, args) in tools {
        if let Ok(mut child) =
            std::process::Command::new(tool).args(*args).stdin(std::process::Stdio::piped()).spawn()
        {
            if let Some(stdin) = child.stdin.take() {
                use std::io::Write as _;
                let mut stdin = stdin;
                if stdin.write_all(text.as_bytes()).is_ok() {
                    drop(stdin);
                    if child.wait().is_ok_and(|s| s.success()) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Formats a byte count as a human-readable string (e.g. `"12.3 MB"`).
#[allow(clippy::cast_precision_loss)] // display rounding; exact bytes not needed
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Creates a cyan spinner with `msg` already ticking.
#[allow(clippy::literal_string_with_formatting_args)] // indicatif template, not a format! arg
fn make_spinner(msg: impl Into<std::borrow::Cow<'static, str>>) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    // Cosmetic: fall back to the default spinner if the template ever fails.
    if let Ok(style) = ProgressStyle::default_spinner().template("{spinner:.cyan} {msg}") {
        pb.set_style(style);
    }
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb.set_message(msg);
    pb
}

/// Resolves the on-disk `.db` path for `package_key`, rejecting any key that
/// could escape `packages_dir`.
///
/// This is the CLI's single guard for the path-traversal invariant defined by
/// [`validate_package_key`]; every subcommand that opens or removes a package
/// database routes through it.
fn package_db_path(
    packages_dir: &std::path::Path,
    package_key: &str,
) -> Result<PathBuf, LoreError> {
    validate_package_key(package_key)?;
    Ok(packages_dir.join(format!("{package_key}.db")))
}

/// Opens an already-installed package database for reading.
///
/// Validates the key, verifies the file exists (so a typo'd name reports
/// "not installed" instead of silently creating an empty database — `SQLite`
/// opens with `CREATE` by default), then opens it.
async fn open_installed(
    packages_dir: &std::path::Path,
    package: &str,
) -> Result<lore_core::Db, LoreError> {
    let path = package_db_path(packages_dir, package)?;
    if !path.is_file() {
        return Err(LoreError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("package '{package}' is not installed"),
        )));
    }
    lore_core::Db::open(&path).await.map_err(|_| {
        LoreError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("package '{package}' is not installed"),
        ))
    })
}

/// Returns the default packages directory: `~/.local/share/lore/packages`.
fn default_packages_dir() -> PathBuf {
    dirs_next::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("lore").join("packages")
}
