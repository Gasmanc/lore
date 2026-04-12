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
  add     <package>          Install a package from the registry
  remove  <package>          Remove an installed package
  list                       List installed packages
  search  <package> <query>  Search a package
  build   <dir>              Build a package from a local source directory
  mcp                        Start the MCP server on stdin/stdout
```

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
