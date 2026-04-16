//! `lore` command-line interface.
//!
//! Subcommands:
//! - `add`      — install a package from the registry
//! - `remove`   — remove an installed package
//! - `list`     — list installed packages
//! - `search`   — hybrid search across an installed package
//! - `build`    — build a package from a local source directory
//! - `manifest` — print the compressed API surface manifest for a package
//! - `info`     — show detailed metadata and statistics for a package
//! - `mcp`      — start the MCP server on stdin/stdout

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use console::style;
use dialoguer::{FuzzySelect, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};
use lore_core::{LoreError, Package};
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
        Command::Remove { package } => cmd_remove(package, &packages_dir),
        Command::List => cmd_list(&packages_dir).await,
        Command::Search { package, query, budget } => {
            cmd_search(package, query, budget, &packages_dir).await
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
        Command::Manifest { package, copy } => cmd_manifest(package, copy, &packages_dir).await,
        Command::Info { package } => cmd_info(package, &packages_dir).await,
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
fn cmd_remove(package: String, packages_dir: &std::path::Path) -> Result<(), LoreError> {
    let path = packages_dir.join(format!("{package}.db"));
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
    packages_dir: &std::path::Path,
) -> Result<(), LoreError> {
    let path = packages_dir.join(format!("{package}.db"));
    let db = lore_core::Db::open(&path).await.map_err(|_| {
        LoreError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("package '{package}' is not installed"),
        ))
    })?;

    let cache = lore_mcp::model_cache_dir();
    let embedder = tokio::task::spawn_blocking(move || lore_build::Embedder::new(&cache))
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))??;

    let embedding = embedder.embed(&query)?;
    let config = lore_core::SearchConfig { token_budget: budget, ..Default::default() };
    let results = lore_search::search(&db, &query, &embedding, &config).await?;

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
                format!("{}…", &preview[..PREVIEW_LEN])
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
    let display_key = meta.display_key();
    let output_path = output.unwrap_or_else(|| packages_dir.join(format!("{display_key}.db")));

    std::fs::create_dir_all(packages_dir).map_err(LoreError::Io)?;

    let spinner = make_spinner(format!("Building {display_key}…"));

    let cache = lore_mcp::model_cache_dir();
    let builder = tokio::task::spawn_blocking(move || lore_build::PackageBuilder::new(&cache))
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))??;

    let meta_ref = meta.clone();
    let stats = builder.build(&source_dir, meta, &output_path, exclude_examples).await?;

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

/// `lore manifest <package>` — prints the compressed API surface manifest.
async fn cmd_manifest(
    package: String,
    copy: bool,
    packages_dir: &std::path::Path,
) -> Result<(), LoreError> {
    let path = packages_dir.join(format!("{package}.db"));
    let db = lore_core::Db::open(&path).await.map_err(|_| {
        LoreError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("package '{package}' is not installed"),
        ))
    })?;

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
            println!("{}", manifest);
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
    let path = packages_dir.join(format!("{package}.db"));
    let db = lore_core::Db::open(&path).await.map_err(|_| {
        LoreError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("package '{package}' is not installed"),
        ))
    })?;

    let meta = db.get_package_meta().await?;

    // File size.
    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
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

/// `lore mcp` — starts the MCP server on stdio.
async fn cmd_mcp(packages_dir: PathBuf) -> Result<(), LoreError> {
    lore_mcp::serve_stdio(packages_dir).await
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
                    if child.wait().map(|s| s.success()).unwrap_or(false) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Formats a byte count as a human-readable string (e.g. `"12.3 MB"`).
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
fn make_spinner(msg: impl Into<std::borrow::Cow<'static, str>>) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner().template("{spinner:.cyan} {msg}").expect("valid template"),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb.set_message(msg);
    pb
}

/// Returns the default packages directory: `~/.local/share/lore/packages`.
fn default_packages_dir() -> PathBuf {
    dirs_next::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("lore").join("packages")
}
