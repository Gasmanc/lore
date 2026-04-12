# Lore — Architecture

## Overview

Lore is a local-first, offline documentation server for AI coding assistants. It indexes library
documentation into portable SQLite databases and serves them via the Model Context Protocol (MCP).
A companion CLI enables package management, custom builds, and compressed manifest generation for
CLAUDE.md fingerposting.

---

## Workspace Structure

```
lore/
  Cargo.toml              # workspace root — lists all members
  crates/
    lore-core/            # shared types, DB schema, connection management
    lore-build/           # parsing, chunking, embedding, indexing pipeline
    lore-search/          # BM25 + vector search, RRF, MMR, token budget
    lore-mcp/             # MCP server, tool definitions, transports
    lore-registry/        # registry API client, package download, YAML defs
    lore-cli/             # the `lore` binary, all subcommands
  registry/               # community package YAML definitions
  docs/                   # project documentation
```

---

## Crate Dependency Graph

```
lore-cli
  ├── lore-mcp
  │     ├── lore-search
  │     │     └── lore-core
  │     └── lore-core
  ├── lore-build
  │     └── lore-core
  ├── lore-registry
  │     └── lore-core
  └── lore-core
```

`lore-core` is the only shared dependency. No circular dependencies.

---

## Crate Responsibilities

### `lore-core`

Shared types and database primitives. No business logic.

**Key types:**

```rust
pub struct Node {
    pub id:          i64,
    pub parent_id:   Option<i64>,
    pub path:        String,         // path enumeration: "1/4/9/23"
    pub doc_id:      i64,
    pub kind:        NodeKind,
    pub level:       Option<u8>,     // heading level 1–6; None for chunks
    pub title:       Option<String>, // heading text or None
    pub content:     Option<String>, // chunk text; None for heading-only nodes
    pub token_count: u32,
    pub lang:        Option<String>, // code block language if kind == CodeBlock
}

pub enum NodeKind {
    Heading,
    Chunk,
    CodeBlock,
}

pub struct Package {
    pub name:        String,
    pub registry:    String,
    pub version:     String,
    pub description: Option<String>,
    pub source_url:  Option<String>,
    pub git_sha:     Option<String>,
}

pub struct SearchResult {
    pub node:        Node,
    pub doc_title:   String,
    pub heading_path: Vec<String>,  // breadcrumb from root to this node
    pub score:       f64,
}

pub struct SearchConfig {
    pub limit:              usize,   // default: 20 candidates before filtering
    pub relevance_threshold: f64,   // default: 0.5 × top score
    pub token_budget:       u32,    // default: 2000
    pub mmr_lambda:         f64,    // default: 0.7 (relevance vs diversity)
}
```

**Database:**
- `Db` struct wrapping `tokio_rusqlite::Connection`
- Schema migrations (versioned, append-only)
- `sqlite_vec::load()` called on every connection open
- Helper methods: `insert_node`, `get_node`, `get_children`, `get_ancestors`

**Schema:**

```sql
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE docs (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    path      TEXT NOT NULL,
    title     TEXT,
    UNIQUE(path)
);

CREATE TABLE nodes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id   INTEGER REFERENCES nodes(id),
    path        TEXT NOT NULL,
    doc_id      INTEGER NOT NULL REFERENCES docs(id),
    kind        TEXT NOT NULL CHECK(kind IN ('heading','chunk','code_block')),
    level       INTEGER,
    title       TEXT,
    content     TEXT,
    token_count INTEGER NOT NULL DEFAULT 0,
    lang        TEXT
);

CREATE INDEX nodes_path   ON nodes(path);
CREATE INDEX nodes_doc    ON nodes(doc_id);
CREATE INDEX nodes_parent ON nodes(parent_id);

CREATE VIRTUAL TABLE nodes_fts USING fts5(
    content='nodes',
    content_rowid='id',
    title,
    content,
    tokenize='porter unicode61'
);

CREATE VIRTUAL TABLE node_embeddings USING vec0(
    embedding FLOAT[384]
);
```

---

### `lore-build`

The indexing pipeline. Transforms raw documentation sources into indexed `.db` files.

**Parser trait:**

```rust
pub trait Parser: Send + Sync {
    fn can_parse(&self, path: &Path) -> bool;
    fn parse(&self, content: &str, path: &Path) -> Result<ParsedDoc>;
}

pub struct ParsedDoc {
    pub title: Option<String>,
    pub root:  HeadingNode,
}

pub struct HeadingNode {
    pub level:    u8,
    pub title:    String,
    pub blocks:   Vec<ContentBlock>,
    pub children: Vec<HeadingNode>,
}

pub enum ContentBlock {
    Paragraph(String),
    Code { lang: Option<String>, content: String },  // always atomic
    Table(String),
    Other(String),
}
```

**Implementations:** `MarkdownParser` (pulldown-cmark), `HtmlParser` (scraper + htmd),
`AsciidocParser` (line-based), `RstParser` (underline-based).

**Chunker:**
1. Walk `HeadingNode` tree, detect primary heading level per document
2. Sections under 300 tokens: kept as single chunk
3. Sections 300–800 tokens: kept as single chunk (single-topic assumption)
4. Sections over 800 tokens: paragraph-level semantic similarity pass
   - Embed each paragraph with `bge-small-en-v1.5`
   - Find similarity valleys (below mean − 1.5σ)
   - Split at valleys, never inside code blocks
   - Merge fragments under 50 tokens back into adjacent chunk
5. Assign `path` string based on parent node paths

**Embedder:**
- Wraps `fastembed::TextEmbedding` with `bge-small-en-v1.5`
- Before embedding any chunk: prepend full breadcrumb path
  - `"Next.js Docs > Caching > Configuration > cacheLife()\n\n{content}"`
- Returns `Vec<f32>` of length 384
- Model downloaded to `~/.cache/lore/models/` on first use
- Batch embedding for performance

**Indexer:**
1. Discover documentation files (`.md`, `.mdx`, `.html`, `.adoc`, `.rst`)
2. Filter: exclude `node_modules/`, `CHANGELOG`, `CODE_OF_CONDUCT`, test dirs
3. For each file: parse → chunk → embed → write nodes + embeddings to DB
4. Build FTS5 index after all nodes inserted
5. Write package metadata to `meta` table
6. Compute and store manifest

---

### `lore-search`

Pure search logic. No I/O except SQLite reads via `lore-core`.

**Pipeline:**

```
query string
    │
    ├─ sanitize()           strip FTS special chars
    ├─ embed()              Vec<f32> via fastembed
    │
    ├─ fts_search()         BM25 via FTS5 MATCH → Vec<ScoredNode>
    ├─ vec_search()         cosine via sqlite-vec → Vec<ScoredNode>
    │
    ├─ rrf_merge()          Reciprocal Rank Fusion → Vec<ScoredNode>
    ├─ mmr_filter()         Maximal Marginal Relevance → Vec<ScoredNode>
    ├─ fetch_parents()      small-to-big: replace chunk with parent section
    ├─ apply_budget()       stop at token_budget → Vec<Node>
    │
    └─ Vec<SearchResult>
```

**RRF:** `score(d) = Σ 1 / (k + rank_i)` where k = 60. Combines BM25 and vector ranks
without requiring weight tuning.

**MMR:** `score(d) = λ · relevance(d, q) − (1−λ) · max_sim(d, selected)` where λ = 0.7.
Penalises chunks too similar to already-selected results.

**FTS5 field weights:**
```sql
SELECT *, rank FROM nodes_fts
WHERE nodes_fts MATCH ?
ORDER BY bm25(nodes_fts, 5.0, 10.0) -- title weight=5, content weight=10
LIMIT ?
```

**Small-to-big retrieval:** After ranking, replace each matched `Chunk` node with its nearest
`Heading` ancestor that fits within the token budget. Returns more complete context.

---

### `lore-mcp`

MCP server using the `rmcp` crate. Exposes four tools.

**Tools:**

```
get_docs(library: Enum, topic: String, config?: SearchConfig) → SearchResponse
    - library is a dynamic enum built from installed packages
    - Runs full search pipeline
    - Returns results with breadcrumb paths

search_packages(registry: String, name: String, version?: String) → PackageList
    - Queries registry API
    - Returns matching packages sorted by version descending

download_package(registry: String, name: String, version: String) → InstallResult
    - Downloads .db to temp file
    - Validates schema
    - Moves to ~/.lore/packages/{registry}-{name}@{version}.db
    - Refreshes installed package list
    - Regenerates get_docs enum

get_manifest(library: Enum) → String
    - Returns compressed ~300 token index of package contents
    - Suitable for pasting into CLAUDE.md
```

**Transports:**
- Stdio (default): `lore serve`
- HTTP: `lore serve --http --port 3000`

**Package store:** In-memory map of installed packages, rebuilt on startup by scanning
`~/.lore/packages/`. The `get_docs` and `get_manifest` tool enums are regenerated whenever
a package is installed or removed.

---

### `lore-registry`

Registry API client and community package build infrastructure.

**Package definition (YAML):**

```yaml
name: next
registry: npm
description: The React Framework for the Web
repository: https://github.com/vercel/next.js

versions:
  - min_version: "14.0.0"
    source:
      type: git
      url: https://github.com/vercel/next.js
      tag: "v{version}"
      docs_path: /docs
```

**Registry API:**
```
GET  /search?registry={r}&name={n}&version={v}   → PackageList
GET  /packages/{registry}/{name}/{version}        → PackageMetadata
GET  /packages/{registry}/{name}/{version}/download → .db file
POST /packages/{registry}/{name}/{version}        → publish (auth required)
```

**Version discovery:**
- npm: `registry.npmjs.org/{name}` JSON
- PyPI: `pypi.org/pypi/{name}/json`
- crates.io: `crates.io/api/v1/crates/{name}`
- Filters: exclude pre-releases, keep latest patch per major.minor

---

### `lore-cli`

The `lore` binary. Uses `clap` with derive macros.

**Subcommands:**

```
lore install <lib@version>         download and install a package
lore remove <lib@version>          uninstall a package
lore list                          show all installed packages
lore get <lib> <topic>             query docs directly (outside MCP)
lore manifest <lib>                print compressed index for CLAUDE.md
lore search <registry> <name>      search registry for a package
lore build <source> [--name] [--version]  build .db from local dir/git/URL
lore serve [--http] [--port]       start MCP server
lore info <lib>                    show package metadata and stats
```

---

## Data Flow

### Build time
```
Source (git/dir/URL)
  → file discovery
  → format detection
  → Parser::parse() → ParsedDoc (HeadingNode tree)
  → Chunker → Vec<Node>
  → Embedder (breadcrumb + content → Vec<f32>)
  → DB writer → nodes table + node_embeddings table + nodes_fts index
  → manifest builder → meta table ("manifest" key)
  → .db file written to disk
```

### Query time
```
MCP: get_docs(library="next@15", topic="cache expiry")
  → sanitize query
  → embed query (no breadcrumb — it's a question)
  → FTS5 MATCH "cache expiry" → ranked list A (BM25)
  → sqlite-vec KNN "cache expiry embedding" → ranked list B (cosine)
  → RRF merge → unified ranked list
  → MMR filter (λ=0.7) → deduplicated list
  → small-to-big: fetch parent heading sections
  → token budget: stop at 2000 tokens
  → return Vec<SearchResult> with heading_path breadcrumbs
```

---

## File Layout

```
~/.lore/
  packages/
    npm-next@15.0.0.db
    npm-react@18.3.0.db
    pypi-fastapi@0.110.0.db
  config.json           # registry servers, default settings
~/.cache/lore/
  models/
    bge-small-en-v1.5/  # downloaded on first use, ~130MB
```

---

## Key Constraints

- Every `.db` file is self-contained and portable — no external dependencies at query time
- Embedding model is downloaded once and reused across all packages
- All search runs locally — no network calls during `get_docs`
- `sqlite-vec` and FTS5 both live inside the same SQLite file
- The MCP server is stateless across sessions — all state is in `.db` files
