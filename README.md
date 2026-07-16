# lore

**Local-first offline documentation server for AI coding assistants.**

lore indexes documentation packages into compact SQLite databases and exposes
them through an [MCP](https://modelcontextprotocol.io) server.  Your AI
assistant can search the docs via fast hybrid retrieval (BM25 + vector search
with RRF fusion) without any network calls at query time.

## Features

- **Offline-first** — all data lives on disk; no cloud dependency at runtime
- **Hybrid search** — FTS5 BM25 + bge-small-en-v1.5 vector search, fused with RRF
- **MMR diversity** — results are diversified to avoid redundant chunks
- **MCP server** — drop-in tool for Claude, Cursor, and any MCP-compatible client
- **Registry** — pre-built packages for popular libraries (npm, cargo, pypi)
- **Build your own** — index any git repo, website, or local directory

## Installation

### macOS / Linux (curl)

```bash
curl -fsSL https://raw.githubusercontent.com/lore-dev/lore/main/install/install.sh | sh
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/lore-dev/lore/main/install/install.ps1 | iex
```

### Homebrew

```bash
brew tap lore-dev/lore
brew install lore
```

### Cargo

```bash
cargo install lore-cli
```

### From source

```bash
git clone https://github.com/lore-dev/lore
cd lore
cargo install --path crates/lore-cli
```

## Quick start

```bash
# Add a pre-built package from the registry
lore add tokio

# Search it
lore search tokio "spawn async task"

# Start the MCP server (reads from stdin, writes to stdout)
lore mcp
```

## MCP tools

The `lore mcp` server exposes these tools to a coding agent:

- **`search_docs`** — hybrid search of one package, with per-session
  deduplication (`fresh_only`, default on) so repeated searches surface new
  material instead of re-spending the token budget on already-seen chunks.
- **`search_stack`** — federated search across *every* installed package that
  matches the current project's declared dependencies (read from
  `Cargo.toml` / `package.json` / `pyproject.toml`), ranked together. Removes
  the need for the agent to know which library holds the answer.
- **`resolve_package`** — map a bare name (`tokio`) to installed keys
  (`cargo-tokio@1.44.2`).
- **`stack_status`** — report which project dependencies have installed docs,
  which are missing, and where the indexed version has drifted from the
  declared one.
- **`list_packages`**, **`get_manifest`**, **`get_node`**, **`reset_session`**.

## MCP configuration

### Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "lore": {
      "command": "lore",
      "args": ["mcp"]
    }
  }
}
```

### Cursor (`.cursor/mcp.json` in project root)

```json
{
  "mcpServers": {
    "lore": {
      "command": "lore",
      "args": ["mcp"]
    }
  }
}
```

## CLI reference

```
lore <SUBCOMMAND>

Subcommands:
  add        <package>          Install a package from the registry
  remove     <package>          Remove an installed package
  list                          List installed packages
  search     <package> <query>  Search a package (--fast = keyword-only, no model load)
  build      <dir>              Build a package from a local source directory
  build-website <url>           Build from a live site (llms.txt digest or crawl)
  build-rustdoc --crate <name>  Build from `cargo rustdoc` JSON (exact locked API)
  update     [packages...]      Rebuild installed packages from their sources
  diff       <old> <new>        Diff the API surface of two package versions
  manifest   <package>          Print the compact API-surface manifest
  info       <package>          Show package metadata and statistics
  doctor     <package>          Report indexing + retrieval-quality health
  check-updates                 Check installed packages against upstream registries
  mcp                           Start the MCP server on stdin/stdout
```

### rustdoc-JSON ingestion

`lore build-rustdoc --crate tokio --version 1.44.2` runs
`cargo +nightly rustdoc --output-format json` for a dependency and indexes the
**exact locked version's** public API — every item, signature, and doc comment.
Pass `--json <path>` instead to ingest a rustdoc JSON you already generated. This
is the most version-precise source for Rust crates.

### Version diffing

`lore diff cargo-axum@0.7.9 cargo-axum@0.8.9` reports the API items added,
removed, and changed between two installed versions — a breaking-change signal.
Most precise on `build-rustdoc` packages.

### Fast keyword search

`lore search <pkg> <query> --fast` skips loading the embedding model entirely
(BM25-only), turning a ~300 ms lookup into single-digit milliseconds — ideal for
exact API-name queries. Omit `--fast` for hybrid semantic + keyword search.

Run `lore help <subcommand>` for full flag documentation.

## Building a custom package

Create a YAML spec file and run `lore build`:

```yaml
# docs/mylib.yaml
name: mylib
registry: cargo
version: "1.0.0"
description: "My Rust library"
source:
  type: git
  url: "https://github.com/me/mylib"
  branch: main
  subdir: docs
```

Or index a local directory directly:

```bash
lore build ./docs --name mylib --version 1.0.0 --registry cargo
```

## Retrieval quality

The search pipeline is benchmarked with 20 natural-language queries against a
20-document synthetic corpus.  Run the benchmark yourself:

```bash
cargo run -p lore-bench --release
```

Typical results on the bge-small-en-v1.5 model:

```
MRR@10 : 0.9250
Hit@1  : 18/20  (90.0%)
Hit@3  : 19/20  (95.0%)
Hit@10 : 20/20  (100.0%)
```

## Architecture

```
lore-core        — shared types, DB schema, math utilities
lore-build       — parse → chunk → embed → index pipeline
lore-search      — FTS5 + vector → RRF → MMR → token budget
lore-registry    — remote registry client + YAML package specs
lore-mcp         — MCP server (rmcp 0.2)
lore-cli         — lore binary (clap)
lore-bench       — retrieval quality benchmarks
```

## License

MIT — see [LICENSE](LICENSE).
