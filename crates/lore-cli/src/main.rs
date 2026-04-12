//! `lore` command-line interface.
//!
//! Subcommands:
//! - `add`    — install a package from the registry
//! - `remove` — remove an installed package
//! - `list`   — list installed packages
//! - `search` — hybrid search across an installed package
//! - `build`  — build a package from a local source directory
//! - `mcp`    — start the MCP server on stdin/stdout

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use lore_core::{LoreError, Package};

/// Maximum number of content characters shown in a search result preview.
const PREVIEW_LEN: usize = 200;

// ── CLI definition ─────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "lore",
    about = "Local documentation server for AI coding assistants",
    version
)]
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
        Command::Add { package, version } => cmd_add(package, version).await,
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
        Command::Mcp => cmd_mcp(packages_dir).await,
    };

    if let Err(e) = result {
        eprintln!("{} {e}", style("error:").red().bold());
        std::process::exit(1);
    }
}

// ── Command implementations ────────────────────────────────────────────────────

/// `lore add <package>` — placeholder until lore-registry is implemented.
async fn cmd_add(package: String, _version: Option<String>) -> Result<(), LoreError> {
    eprintln!(
        "{} Registry download is not yet implemented.\n\
         To install a package manually, build it with:\n  \
         lore build <source-dir> --name {package} --version <version>",
        style("info:").blue().bold(),
    );
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
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))?
        ?;

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

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .expect("valid template"),
    );
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));
    spinner.set_message(format!("Building {display_key}…"));

    let cache = lore_mcp::model_cache_dir();
    let builder = tokio::task::spawn_blocking(move || lore_build::PackageBuilder::new(&cache))
        .await
        .map_err(|e| LoreError::Io(std::io::Error::other(e.to_string())))?
        ?;

    let stats = builder
        .build(&source_dir, meta, &output_path, exclude_examples)
        .await?;

    spinner.finish_and_clear();
    println!(
        "{} Built {} → {}",
        style("✓").green().bold(),
        style(&display_key).bold(),
        output_path.display(),
    );
    println!("{}", stats.summary());
    Ok(())
}

/// `lore mcp` — starts the MCP server on stdio.
async fn cmd_mcp(packages_dir: PathBuf) -> Result<(), LoreError> {
    lore_mcp::serve_stdio(packages_dir).await
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Returns the default packages directory: `~/.local/share/lore/packages`.
fn default_packages_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lore")
        .join("packages")
}
