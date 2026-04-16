# Quick Start

Get lore running in under five minutes.

---

## 1. Install

```sh
curl -fsSL https://raw.githubusercontent.com/lore-dev/lore/main/install/install.sh | sh
```

See [INSTALL.md](INSTALL.md) for Homebrew, Windows, and Cargo alternatives.

---

## 2. Add your first package

lore ships with a registry of pre-built packages for popular libraries.

```sh
lore add tokio
```

This downloads a pre-indexed SQLite database for the Tokio async runtime docs.
If no pre-built package exists, lore falls back to building one from the
project's source repository.

**Browse the available packages:**

```sh
lore list --available
```

**Add multiple packages at once:**

```sh
lore add tokio axum serde
```

---

## 3. Search a package

```sh
lore search tokio "spawn async task"
```

lore uses hybrid retrieval (BM25 keyword search + bge-small-en-v1.5 vector
search, fused with Reciprocal Rank Fusion) to return the most relevant chunks.

---

## 4. Connect lore to your AI assistant

### Claude Desktop

Edit `~/Library/Application Support/Claude/claude_desktop_config.json`:

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

Restart Claude Desktop.  The `get_docs`, `search_docs`, and `get_manifest`
tools are now available in every conversation.

### Cursor

Create or edit `.cursor/mcp.json` in your project root:

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

### Other MCP clients

`lore mcp` speaks the [Model Context Protocol](https://modelcontextprotocol.io)
over stdin/stdout.  Any MCP-compatible client can use it with the command
`lore mcp`.

---

## 5. Add lore context to your CLAUDE.md

For persistent context in Claude Code projects, paste the API surface manifest
directly into your `CLAUDE.md`:

```sh
lore manifest tokio --copy      # copies to clipboard on macOS
```

Or redirect it into a file:

```sh
lore manifest tokio >> CLAUDE.md
```

See [CLAUDE_MD_INTEGRATION.md](CLAUDE_MD_INTEGRATION.md) for the recommended
structure.

---

## 6. Build a custom package

Index any local directory, git repository, or website.

**Local directory:**

```sh
lore build ./docs --name mylib --version 1.0.0 --registry cargo
```

**Git repository:**

```yaml
# mylib.yaml
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

```sh
lore build mylib.yaml
```

**Website:**

```yaml
name: myframework
registry: npm
version: "2.0.0"
source:
  type: website
  url: "https://docs.myframework.dev"
  max_pages: 200
```

---

## Next steps

- [INSTALL.md](INSTALL.md) — all installation methods and options
- [CLAUDE_MD_INTEGRATION.md](CLAUDE_MD_INTEGRATION.md) — how to structure `CLAUDE.md`
- [ARCHITECTURE.md](ARCHITECTURE.md) — internals and retrieval pipeline
- `lore help <subcommand>` — full CLI flag reference
