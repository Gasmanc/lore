# CLAUDE.md Integration

This guide explains how to use `lore manifest` to give your AI coding assistant
instant access to the API surface of every library in your project — without
sending documentation pages in every message.

---

## What is a manifest?

A **manifest** is a compact (~500 tokens) index of the key API symbols from a
documentation package: function signatures, class names, important constants.
It is stored in the package database and retrieved with:

```sh
lore manifest <package>
```

Example output:

```
Client: new, query, from_config
Config: timeout_secs, url, token
QueryResult: items, total
Error: Http, Auth, Deserialise
```

The manifest is machine-readable and designed to fit inside a `CLAUDE.md` file
without bloating every conversation.

---

## Recommended CLAUDE.md structure

```markdown
# Project

Brief description of what this project does.

## Libraries

The following libraries are indexed in lore.  Use the `get_docs` MCP tool to
search for full documentation and code examples.

### tokio (async runtime)

LORE_MANIFEST:tokio
```tokio
runtime: Runtime, Builder, Handle
task: spawn, spawn_blocking, sleep, timeout
sync: Mutex, RwLock, Semaphore, oneshot, mpsc, broadcast
io: AsyncRead, AsyncWrite, BufReader
```

### axum (HTTP server)

LORE_MANIFEST:axum
```axum
Router: new, route, layer, nest, merge, with_state
handlers: get, post, put, delete, patch
extractors: Path, Query, Json, State, Extension, Form
response: IntoResponse, Response, Json, Html, Redirect
```

## Development notes

- All database access goes through `Db` (see `crates/lore-core/src/db/`)
- Run tests with `cargo test --workspace`
```

---

## Pasting a manifest into CLAUDE.md

### Automatic (clipboard)

```sh
lore manifest tokio --copy
# Copied to clipboard.  Paste into CLAUDE.md.
```

### Manual

```sh
lore manifest tokio
```

Copy the output and paste it into the relevant section of your `CLAUDE.md`.

### Batch update

To refresh manifests for all installed packages:

```sh
for pkg in $(lore list --names); do
    echo "### $pkg"
    lore manifest "$pkg"
    echo
done >> CLAUDE.md
```

---

## How the MCP tools use packages

When the lore MCP server is connected, the AI assistant can call:

| Tool | What it does |
|---|---|
| `get_manifest` | Returns the compact API surface index (~500 tokens) |
| `get_docs` | Runs hybrid search and returns the most relevant chunks |
| `search_docs` | Lower-level search with configurable result count |

The manifest in `CLAUDE.md` helps the AI decide *which* package to call
`get_docs` on.  Without the manifest, the assistant has to guess or ask.

---

## Token budget guidance

| Content | Approx. tokens |
|---|---|
| One manifest | ≤ 500 |
| One `get_docs` result | 800 – 2 000 |
| 10 manifests | ≤ 5 000 |

A `CLAUDE.md` with 10 manifests consumes roughly the same token budget as a
single `get_docs` call — a worthwhile trade for always-available API context.

---

## Keeping manifests up to date

Manifests are regenerated every time a package is rebuilt:

```sh
lore build ./docs --name mylib --version 1.0.1 --registry cargo
lore manifest mylib  # reflects the new version
```

If you commit `CLAUDE.md` to version control, update the manifests whenever
the library version changes.

---

## Troubleshooting

**`lore manifest tokio` says "no manifest — rebuild with lore build":**

The package was installed from a registry that predates manifest generation.
Force a local rebuild:

```sh
lore build --from-registry tokio
```

**The manifest is empty or very short:**

The documentation source may not contain structured headings or code blocks.
lore extracts signatures from headings and fenced code blocks — plain prose
produces a shorter manifest.  This is expected for narrative-heavy docs.
