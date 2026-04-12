# Lore — Implementation Plan

## Phase Overview

| Phase | Name | Description |
|-------|------|-------------|
| 1 | Foundation | Workspace setup, lore-core types, database schema |
| 2 | Document Parsing | Parser trait, Markdown/HTML/AsciiDoc/RST implementations |
| 3 | Chunking Pipeline | Structural chunking, semantic refinement, code block preservation |
| 4 | Embedding Pipeline | fastembed integration, contextual embeddings, batch processing |
| 5 | Database & Indexing | Node insertion, FTS5 index, sqlite-vec, manifest storage |
| 6 | Search Engine | FTS5 search, vector search, RRF, MMR, small-to-big, token budget |
| 7 | MCP Server | rmcp integration, all four tools, dynamic enum, transports |
| 8 | CLI | All subcommands, progress indicators, interactive prompts |
| 9 | Registry Client | API client, package download, validation, install workflow |
| 10 | Custom Build | Local dir, git clone, website/llms.txt, existing .db sources |
| 11 | Manifest Generation | Heading extraction, API signature extraction, token-budget output |
| 12 | Registry Infrastructure | YAML definitions, version discovery, build+publish CI |
| 13 | Testing | Unit tests, integration tests, retrieval quality benchmarks |
| 14 | Distribution | Binary packaging, install script, documentation |

---

## Phase 1: Foundation

**Goal:** Establish the Cargo workspace, define all shared types in `lore-core`, and set up
the SQLite schema with migrations.

### 1.1 Convert to Cargo workspace

The existing `~/Coding/lore` directory has a single-crate `Cargo.toml` from `cargo init`.
This must be replaced with a workspace manifest that lists all six member crates.

- Replace the root `Cargo.toml` with a workspace manifest
- Define `[workspace.dependencies]` for all shared dependencies with pinned versions
- Create the `crates/` directory
- Initialise all six member crates: `lore-core`, `lore-build`, `lore-search`, `lore-mcp`,
  `lore-registry`, `lore-cli`
- Delete the root `src/` directory created by `cargo init`
- Verify `cargo build` succeeds for the empty workspace
- Verify `cargo test` succeeds for the empty workspace

### 1.2 Define core types in lore-core

Define all shared data structures. These types are the contract between all other crates.

- Add dependencies to `lore-core/Cargo.toml`: `serde` (with derive), `thiserror`,
  `rusqlite` (bundled), `tokio-rusqlite`, `sqlite-vec`
- Create `src/error.rs` with `LoreError` enum covering: database errors, parse errors,
  IO errors, embedding errors, schema errors, registry errors
- Create `src/node.rs` with `NodeKind` enum (`Heading`, `Chunk`, `CodeBlock`) and
  `Node` struct with all fields as specified in ARCHITECTURE.md
- Create `src/package.rs` with `Package` struct and `PackageMetadata` struct
- Create `src/search.rs` with `SearchResult`, `SearchConfig` (with all defaults), and
  `ScoredNode` structs
- Create `src/doc.rs` with `Doc` struct (id, path, title)
- Implement `Default` for `SearchConfig` with values: limit=20, relevance_threshold=0.5,
  token_budget=2000, mmr_lambda=0.7
- Implement `Display` and `serde::Serialize`/`Deserialize` for all public types
- Re-export all types from `src/lib.rs`

### 1.3 Database connection and schema management

Set up the database abstraction and schema migrations.

- Create `src/db.rs` in `lore-core`
- Define a `Db` struct wrapping `tokio_rusqlite::Connection`
- Implement `Db::open(path: &Path) -> Result<Db>` that:
  - Opens the `tokio_rusqlite::Connection`
  - Loads the `sqlite-vec` extension via `sqlite_vec::load()`
  - Calls `run_migrations()` to create tables if not present
  - Enables WAL mode for performance
  - Sets `PRAGMA journal_mode = WAL` and `PRAGMA synchronous = NORMAL`
- Implement `run_migrations()` with the full schema as defined in ARCHITECTURE.md:
  - `meta` table
  - `docs` table
  - `nodes` table with all columns and constraints
  - All three indexes on `nodes` (path, doc_id, parent_id)
  - `nodes_fts` FTS5 virtual table with Porter + unicode61 tokeniser
  - `node_embeddings` vec0 virtual table with 384 dimensions
- Implement a `schema_version` mechanism using the `meta` table so future migrations
  can be applied incrementally
- Implement basic CRUD methods on `Db`:
  - `insert_doc(path, title) -> Result<i64>`
  - `insert_node(node: &NewNode) -> Result<i64>` where `NewNode` is the insert struct
  - `get_node(id: i64) -> Result<Node>`
  - `get_children(parent_id: i64) -> Result<Vec<Node>>`
  - `get_ancestors(node_id: i64) -> Result<Vec<Node>>`
  - `get_heading_path(node_id: i64) -> Result<Vec<String>>`
  - `insert_embedding(node_id: i64, embedding: &[f32]) -> Result<()>`
  - `get_package_meta() -> Result<Package>`
  - `set_meta(key: &str, value: &str) -> Result<()>`
  - `get_meta(key: &str) -> Result<Option<String>>`

### 1.4 Verify foundation compiles and tests pass

- Write a test in `lore-core` that opens an in-memory SQLite database, runs migrations,
  inserts a doc, inserts a node, and reads it back
- Verify the FTS5 virtual table is created correctly
- Verify the vec0 virtual table is created correctly
- Verify `cargo test -p lore-core` passes

---

## Phase 2: Document Parsing

**Goal:** Implement the `Parser` trait and format-specific parsers that produce a
`ParsedDoc` (heading tree) from raw file content.

### 2.1 Define the Parser trait and AST types in lore-build

- Add dependencies to `lore-build/Cargo.toml`: `pulldown-cmark`, `scraper`, `htmd`,
  `lore-core` (path dependency)
- Create `src/parser/mod.rs` with the `Parser` trait, `ParsedDoc`, `HeadingNode`, and
  `ContentBlock` types as defined in ARCHITECTURE.md
- Implement `ParserRegistry` — a struct that holds a list of `Box<dyn Parser>` and
  selects the correct parser by file extension
- Implement `detect_primary_heading_level(root: &HeadingNode) -> u8` — walks the heading
  tree and returns the heading level that represents the primary topic unit:
  - Count heading nodes at each level
  - Find the deepest level where headings contain substantial content (>100 tokens average)
  - Return that level, defaulting to 2 if ambiguous

### 2.2 Implement MarkdownParser

- Create `src/parser/markdown.rs`
- Use `pulldown-cmark` in AST mode (not renderer mode)
- `can_parse`: returns true for `.md`, `.mdx`, `.qmd`, `.rmd` extensions
- `parse` implementation:
  - Extract YAML frontmatter if present (between `---` delimiters) and use `title` field
    as the document title if available
  - Walk the pulldown-cmark event stream
  - Build `HeadingNode` objects for each heading event, tracking current heading level
    to maintain correct parent-child relationships
  - Accumulate `Paragraph` and `Code` blocks between headings
  - Strip MDX-specific JSX tags (`<ComponentName>` / `</ComponentName>`) from paragraph text
  - Detect Table of Contents sections: skip sections whose title is "table of contents",
    "contents", "toc" (case-insensitive) OR whose content has >60% link density
  - Preserve code block language from the info string (e.g., ` ```rust ` → lang="rust")
  - Handle table events — collect the raw markdown table as a `Table` block
- Write tests for: basic document, nested headings, code blocks, frontmatter, MDX tags,
  TOC detection

### 2.3 Implement HtmlParser

- Create `src/parser/html.rs`
- `can_parse`: returns true for `.html`, `.htm` extensions
- `parse` implementation:
  - Use `scraper` to parse the HTML
  - Remove `<script>`, `<style>`, `<nav>`, `<footer>`, `<header>` elements before processing
  - Extract `<title>` or first `<h1>` as document title
  - Use `htmd` (HTML-to-Markdown) to convert the cleaned body to Markdown
  - Pass the resulting Markdown to `MarkdownParser::parse()` for heading tree construction
- Write tests for: standard HTML, HTML with nav/footer, inline scripts

### 2.4 Implement AsciidocParser

- Create `src/parser/asciidoc.rs`
- `can_parse`: returns true for `.adoc`, `.asciidoc` extensions
- `parse` implementation:
  - Line-based parser — no external AST library
  - Detect document title: first line matching `= Title` pattern
  - Detect headings: lines matching `== H2`, `=== H3`, `==== H4` patterns
  - Detect code blocks: lines between `----` delimiters (source blocks with `[source,lang]`
    attribute on the preceding line)
  - Build `HeadingNode` tree from heading lines
  - Accumulate paragraph text between headings
  - Extract code block language from the `[source,rust]` style attribute
- Write tests for: basic document, nested headings, source blocks with language annotation

### 2.5 Implement RstParser

- Create `src/parser/rst.rs`
- `can_parse`: returns true for `.rst` extensions
- `parse` implementation:
  - Line-based parser
  - Detect headings by underline character patterns: a heading is a non-empty line
    followed by a line of `=`, `-`, `~`, `^`, `"` of equal or greater length
  - Build hierarchy by tracking underline characters in order of first appearance
    (first character seen = H1, second = H2, etc.)
  - Detect code blocks: `.. code-block:: lang` directives followed by indented content
  - Detect `.. code::` as well as `::` block endings
  - Build `HeadingNode` tree
- Write tests for: basic document, mixed underline characters, code blocks

### 2.6 Integration tests for all parsers

- Test each parser with a realistic documentation file from a known library
- Verify heading tree structure is correct
- Verify code blocks are detected with correct language annotations
- Verify `detect_primary_heading_level` returns correct results for each doc type

---

## Phase 3: Chunking Pipeline

**Goal:** Walk the `ParsedDoc` heading tree and produce a flat list of `Node` records,
respecting code block atomicity, detecting primary heading levels, and applying semantic
refinement to oversized sections.

### 3.1 Implement token counter

- Add `tiktoken-rs` to `lore-build/Cargo.toml`
- Create `src/tokens.rs` in `lore-build`
- Implement `TokenCounter` struct that holds an initialised `cl100k_base` tokenizer
- Implement `TokenCounter::count(text: &str) -> u32`
- Implement `TokenCounter::new() -> Result<TokenCounter>` — initialise tokenizer once
- Write tests comparing `length / 4` heuristic to actual tiktoken counts for code-heavy
  and prose-heavy samples

### 3.2 Implement structural chunker

- Create `src/chunker/mod.rs` in `lore-build`
- Create `src/chunker/structural.rs`
- Implement `StructuralChunker` with a `ChunkConfig`:
  ```
  min_tokens: u32          = 50    (merge smaller fragments up)
  soft_max_tokens: u32     = 800   (target max before semantic refinement)
  hard_max_tokens: u32     = 1200  (absolute max; semantic refinement required above this)
  ```
- Implement `chunk(doc: &ParsedDoc, primary_level: u8) -> Vec<RawChunk>` where `RawChunk`
  is an intermediate struct holding:
  - heading path (Vec<String>)
  - content blocks (Vec<ContentBlock>)
  - estimated token count
  - whether any block is a CodeBlock
  - doc_path and doc_title reference
- Walking algorithm:
  - Recurse the HeadingNode tree
  - At the detected primary heading level: start a new chunk
  - Accumulate all ContentBlocks from that node (not its children) into the current chunk
  - If the heading has children at deeper levels, recurse — each child generates its own chunk
  - Code blocks are always placed in their own `RawChunk` as a `NodeKind::CodeBlock`
    regardless of size
  - After all content is accumulated: if a prose chunk exceeds `soft_max_tokens`, mark it
    for semantic refinement
- Write tests for: flat H2 document, deep H3 API reference, mixed content, huge code block,
  tiny sections that need merging

### 3.3 Implement semantic refinement

- Create `src/chunker/semantic.rs` in `lore-build`
- This module is called only for `RawChunk` items marked for refinement (>800 tokens, prose)
- Implement `SemanticRefiner` that accepts an `Embedder` reference (see Phase 4)
- Implement `refine(chunk: RawChunk, embedder: &Embedder) -> Vec<RawChunk>`:
  - Split chunk into individual paragraphs
  - Embed each paragraph (without breadcrumb — these are short fragments, not final chunks)
  - Compute cosine similarity between each consecutive pair of paragraph embeddings
  - Calculate mean and standard deviation of all similarity scores
  - Identify valleys: positions where similarity < (mean − 1.5 × std_dev)
  - Split at valley positions
  - After splitting: merge any resulting fragment under `min_tokens` into its adjacent
    fragment (prefer merging with the following fragment, fall back to preceding)
  - Return the list of refined `RawChunk` objects
- Semantic refinement is skipped entirely if fewer than 3 paragraphs exist (not enough
  data points for meaningful statistics)
- Write tests for: clearly multi-topic section, single-topic section (should not split),
  section with one very large paragraph (no split possible)

### 3.4 Implement path assignment

- Create `src/chunker/path.rs` in `lore-build`
- Implement `assign_paths(chunks: &mut Vec<RawChunk>, root_id: i64)` which assigns
  the slash-separated path string to each chunk based on parent relationships
- The path is built during DB insertion (Phase 5), not here — this phase produces a
  tree of `RawChunk` with parent references, and the path is materialised after IDs
  are assigned by the database
- Implement `ChunkTree` struct that holds parent-child relationships for the insertion
  phase

---

## Phase 4: Embedding Pipeline

**Goal:** Integrate `fastembed-rs`, implement contextual embedding (breadcrumb prepended),
and provide batch embedding for efficient build-time indexing.

### 4.1 Integrate fastembed-rs

- Add `fastembed` to `lore-build/Cargo.toml`
- Create `src/embedder.rs` in `lore-build`
- Implement `Embedder` struct wrapping `fastembed::TextEmbedding`
- Implement `Embedder::new(cache_dir: &Path) -> Result<Embedder>`:
  - Initialise `TextEmbedding` with `EmbeddingModel::BGESmallENV15`
  - Pass `cache_dir` for model storage (`~/.cache/lore/models/`)
  - If model is not cached, fastembed downloads it automatically
  - Print a user-visible message if download is required: "Downloading embedding model
    (130MB, one-time setup)..."
- Implement `Embedder::embed_one(text: &str) -> Result<Vec<f32>>`
- Implement `Embedder::embed_batch(texts: &[String]) -> Result<Vec<Vec<f32>>>`:
  - Use fastembed's native batch API
  - Batch size of 32 to balance memory and throughput
- Write a test that embeds two similar strings and verifies cosine similarity > 0.8,
  and two dissimilar strings with cosine similarity < 0.5

### 4.2 Implement contextual embedding

- Create `src/embedder/contextual.rs` in `lore-build`
- Implement `build_contextual_text(heading_path: &[String], content: &str) -> String`:
  - Joins the heading path with " > "
  - Concatenates: `"{joined_path}\n\n{content}"`
  - Example: `"Next.js Docs > Caching > cacheLife()\n\ncacheLife() accepts a profile name..."`
- This function is called before embedding every chunk
- Query embeddings at search time do NOT use contextual text — they embed the raw query
- Write tests verifying that contextual embeddings for topic-adjacent chunks are more
  similar to each other than to random chunks from unrelated topics

### 4.3 Embedding during semantic refinement

- Update `SemanticRefiner` from Phase 3.3 to use `Embedder`
- Paragraph-level embeddings used for refinement decisions are NOT contextual
  (they are fragments, not final chunks)
- After refinement, the final chunks are embedded contextually by the Indexer (Phase 5)

---

## Phase 5: Database & Indexing

**Goal:** Wire together the parse → chunk → embed pipeline and write the results to the
SQLite database with correct path enumeration, FTS5 indexing, and vector storage.

### 5.1 Implement the Indexer

- Create `src/indexer.rs` in `lore-build`
- Implement `Indexer` struct holding a `Db`, `ParserRegistry`, `StructuralChunker`,
  `SemanticRefiner`, `Embedder`, and `TokenCounter`
- Implement `Indexer::index_file(path: &Path, content: &str) -> Result<()>`:
  1. Select parser by file extension
  2. Parse content into `ParsedDoc`
  3. Detect primary heading level
  4. Structural chunk → `Vec<RawChunk>`
  5. For each `RawChunk` marked for refinement: semantic refinement
  6. Insert `Doc` record, get `doc_id`
  7. For each final chunk:
     a. Insert a `Heading` node for each heading in the path (if not already inserted)
     b. Insert the `Chunk` or `CodeBlock` node with correct `parent_id` and `path`
     c. Build contextual text using heading path
     d. Embed contextual text → `Vec<f32>`
     e. Insert embedding into `node_embeddings` linked to the node's id
  8. Update FTS5 index (trigger rebuild or incremental insert)

### 5.2 Implement file discovery

- Create `src/discovery.rs` in `lore-build`
- Implement `discover_files(root: &Path) -> Result<Vec<PathBuf>>`:
  - Walk directory tree recursively
  - Include files with extensions: `.md`, `.mdx`, `.qmd`, `.rmd`, `.html`, `.htm`,
    `.adoc`, `.asciidoc`, `.rst`
  - Exclude paths containing: `node_modules`, `.git`, `__pycache__`, `target`,
    `dist`, `build`, `.next`, `.nuxt`
  - Exclude files whose name contains: `CHANGELOG`, `CHANGELOG.md`, `CODE_OF_CONDUCT`,
    `LICENSE`, `CONTRIBUTING`
  - Exclude files under directories named: `test`, `tests`, `__tests__`, `spec`, `specs`,
    `fixtures`, `examples` (optional — configurable flag)
  - Return sorted list for deterministic processing order

### 5.3 Implement package builder

- Create `src/builder.rs` in `lore-build`
- Implement `PackageBuilder` struct holding all pipeline components
- Implement `PackageBuilder::build(source_dir: &Path, meta: Package, output: &Path) -> Result<BuildStats>`:
  1. Open `Db` at the output path (creates new file)
  2. Write package metadata to `meta` table
  3. Discover all documentation files via `discover_files()`
  4. For each file: read content, call `indexer.index_file()`
  5. After all files: rebuild FTS5 content table (`INSERT INTO nodes_fts(nodes_fts) VALUES('rebuild')`)
  6. Run `PRAGMA optimize` for query planning
  7. Run `VACUUM` to compact the database
  8. Return `BuildStats`: file count, chunk count, code block count, total tokens, duration
- Implement `BuildStats::display()` for CLI output

### 5.4 FTS5 index population

- The `nodes_fts` table uses `content='nodes'` — it is a content table backed by `nodes`.
- Implement correct FTS5 trigger setup so insertions into `nodes` automatically update
  `nodes_fts`. Create the standard three triggers: after insert, after delete, after update.
- Alternatively: insert all nodes first, then run the bulk `rebuild` command.
- Decision: use bulk rebuild after all nodes are inserted. It is faster for the build
  pipeline and there is no incremental update requirement (build is a one-shot operation).

### 5.5 Write integration test for full build pipeline

- Create a small synthetic documentation set (5 files, various heading depths, code blocks)
- Run the full `PackageBuilder::build()` pipeline against it
- Verify:
  - All files processed without error
  - Node count matches expected
  - FTS5 index returns results for known queries
  - Vector search returns results for known queries
  - No code block is split across multiple chunks
  - Path strings are correctly formed

---

## Phase 6: Search Engine

**Goal:** Implement the full search pipeline: FTS5 query, vector KNN, RRF merge, MMR filter,
small-to-big retrieval, and token budget enforcement.

### 6.1 Implement FTS5 search

- Add `lore-core` and `lore-build` as dependencies in `lore-search/Cargo.toml`
- Create `src/fts.rs` in `lore-search`
- Implement `fts_search(db: &Db, query: &str, limit: usize) -> Result<Vec<ScoredNode>>`:
  - Sanitise query: remove characters `( ) [ ] { } : * ^ ~ \ / | & !`
    that have special meaning in FTS5 queries; preserve alphanumerics, spaces, quotes
  - If sanitised query is empty or whitespace-only: return empty vec
  - Execute FTS5 MATCH query with BM25 field weights:
    title weight = 5.0, content weight = 10.0
  - Map results to `ScoredNode` with rank as score
  - Return up to `limit` results

### 6.2 Implement vector search

- Create `src/vec.rs` in `lore-search`
- Implement `vec_search(db: &Db, embedding: &[f32], limit: usize) -> Result<Vec<ScoredNode>>`:
  - Serialise the query embedding as a packed f32 BLOB
  - Execute `SELECT rowid, distance FROM node_embeddings WHERE embedding MATCH ? ORDER BY distance LIMIT ?`
  - Join with `nodes` table to fetch full `Node` data
  - Convert distance to similarity score: `1.0 - distance` (for cosine distance)
  - Return up to `limit` results as `ScoredNode`

### 6.3 Implement RRF merge

- Create `src/rrf.rs` in `lore-search`
- Implement `rrf_merge(list_a: Vec<ScoredNode>, list_b: Vec<ScoredNode>, k: f64) -> Vec<ScoredNode>`:
  - Build a map of `node_id → rrf_score`
  - For each node in `list_a` at position `rank` (0-indexed): add `1.0 / (k + rank + 1)`
  - For each node in `list_b` at position `rank`: add `1.0 / (k + rank + 1)`
  - Merge node data from both lists (list_a takes precedence for duplicates)
  - Sort descending by RRF score
  - Return merged list
- k = 60.0 (standard RRF constant, exposed as parameter)

### 6.4 Implement MMR filter

- Create `src/mmr.rs` in `lore-search`
- Implement `mmr_filter(candidates: Vec<ScoredNode>, embeddings: &HashMap<i64, Vec<f32>>, lambda: f64, limit: usize) -> Vec<ScoredNode>`:
  - Start with an empty `selected` list
  - On each iteration: score each remaining candidate as
    `λ × relevance_score - (1-λ) × max_cosine_similarity(candidate, selected)`
  - Add the highest-scoring candidate to `selected`
  - Repeat until `selected.len() == limit` or no candidates remain
  - Return `selected`
- Implement `cosine_similarity(a: &[f32], b: &[f32]) -> f64` as a standalone function
- Write tests: verify that when two identical chunks are in candidates, only one is
  selected; verify that diverse chunks are both selected when λ=0.7

### 6.5 Implement small-to-big retrieval

- Create `src/expand.rs` in `lore-search`
- Implement `expand_to_parent(db: &Db, nodes: Vec<ScoredNode>, budget: u32) -> Result<Vec<ScoredNode>>`:
  - For each `ScoredNode` of kind `Chunk`:
    - Fetch its nearest `Heading` ancestor that is at or above the primary heading level
    - If the parent heading's total token count (sum of all descendant chunks) is within
      budget: return the parent node instead of the chunk
    - Otherwise: return the original chunk
  - For `CodeBlock` nodes: return as-is (code blocks are already atomic and complete)

### 6.6 Implement token budget

- Create `src/budget.rs` in `lore-search`
- Implement `apply_token_budget(nodes: Vec<ScoredNode>, budget: u32) -> Vec<ScoredNode>`:
  - Accumulate `token_count` as nodes are added
  - Stop adding when the running total would exceed `budget`
  - Return the nodes that fit within budget
- This is called after `expand_to_parent` — the expanded nodes may have larger token counts

### 6.7 Implement the top-level search function

- Create `src/search.rs` in `lore-search`
- Implement `search(db: &Db, embedder: &Embedder, query: &str, config: &SearchConfig) -> Result<Vec<SearchResult>>`:
  1. Sanitise query
  2. Embed query (raw, no breadcrumb)
  3. `fts_search(db, query, config.limit)`
  4. `vec_search(db, &embedding, config.limit)`
  5. `rrf_merge(fts_results, vec_results, 60.0)`
  6. Apply relevance threshold: drop results below `max_score * config.relevance_threshold`
  7. Fetch embeddings for candidate nodes from `node_embeddings`
  8. `mmr_filter(candidates, &embeddings, config.mmr_lambda, config.limit)`
  9. `expand_to_parent(db, filtered, config.token_budget)`
  10. `apply_token_budget(expanded, config.token_budget)`
  11. For each final node: fetch `heading_path` via `db.get_heading_path()`
  12. Construct and return `Vec<SearchResult>`

### 6.8 Write search integration tests

- Build a test package with known content
- Write queries that require: exact term match, synonym/vocabulary bridging, specific
  API name retrieval, multi-section coverage
- Verify: correct top result for each query, no split code blocks in any result,
  token budget respected, MMR prevents duplicate content

---

## Phase 7: MCP Server

**Goal:** Expose the search engine and package management as MCP tools using `rmcp`.
Implement all four tools. Support stdio and HTTP transports.

### 7.1 Set up rmcp and basic server structure

- Add `rmcp`, `tokio`, `serde_json` to `lore-mcp/Cargo.toml`
- Add `lore-core`, `lore-search`, `lore-registry` as path dependencies
- Create `src/server.rs` implementing the `rmcp::ServerHandler` trait
- Implement `LoreServer` struct holding:
  - `PackageStore`: in-memory map of package name → open `Db` instance
  - `Embedder`: shared across all searches
  - `RegistryClient`: for search and download operations
- Implement `LoreServer::new(packages_dir: &Path, cache_dir: &Path) -> Result<LoreServer>`:
  - Scan `packages_dir` for all `.db` files
  - Open each as a `Db` instance and store in `PackageStore`
  - Initialise `Embedder` (triggers model download if needed)
  - Return the populated server

### 7.2 Implement get_docs tool

- Create `src/tools/get_docs.rs`
- Define `GetDocsParams` struct: `library: String`, `topic: String`, `config: Option<SearchConfig>`
- Implement `handle_get_docs(server: &LoreServer, params: GetDocsParams) -> Result<GetDocsResponse>`:
  1. Look up `library` in `PackageStore` — return error if not installed
  2. Call `search::search(db, embedder, &params.topic, &config)`
  3. Format results as `GetDocsResponse`: library name, version, Vec of results each with
     doc_path, heading_path (as breadcrumb string), content
- The `library` parameter enum is dynamically built from `PackageStore` keys
- Register the tool with `rmcp` using the correct schema

### 7.3 Implement search_packages tool

- Create `src/tools/search_packages.rs`
- Define `SearchPackagesParams`: `registry: String`, `name: String`, `version: Option<String>`
- Call `RegistryClient::search()` and return `PackageList`
- Handle network errors gracefully with a clear error message

### 7.4 Implement download_package tool

- Create `src/tools/download_package.rs`
- Define `DownloadPackageParams`: `registry: String`, `name: String`, `version: String`
- Call `RegistryClient::download()`:
  1. Stream `.db` to `~/.lore/packages/.downloading-{timestamp}-{name}.db`
  2. Validate schema on temp file
  3. Move to final location
- After successful install:
  - Open the new `Db` and add to `PackageStore`
  - Regenerate the `get_docs` tool enum
- Return `InstallResult` with package metadata

### 7.5 Implement get_manifest tool

- Create `src/tools/get_manifest.rs`
- Define `GetManifestParams`: `library: String`
- Look up the package's `Db`, read `meta` table key `"manifest"`
- If manifest not found: return an error indicating the package needs to be rebuilt
- Return the manifest string directly

### 7.6 Implement stdio transport

- Create `src/transport/stdio.rs`
- Implement `run_stdio(server: LoreServer)` using `rmcp`'s stdio server runner
- Wire up signal handling: on SIGTERM/SIGINT, flush any pending responses and exit cleanly

### 7.7 Implement HTTP transport

- Create `src/transport/http.rs`
- Implement `run_http(server: LoreServer, host: String, port: u16)` using `rmcp`'s HTTP
  server runner
- The `LoreServer` must be wrapped in `Arc<RwLock<>>` for shared access across HTTP requests
- Bind to `host:port`, log the listening address

### 7.8 Write MCP integration tests

- Use `rmcp`'s test utilities to exercise each tool
- Test `get_docs` with an installed test package
- Test `download_package` against a mock registry server
- Test enum regeneration after `download_package`

---

## Phase 8: CLI

**Goal:** Implement all `lore` subcommands using `clap` with progress indicators and
interactive prompts.

### 8.1 Set up lore-cli with clap

- Add `clap` (with derive feature), `indicatif`, `dialoguer`, `console`, `tokio` to
  `lore-cli/Cargo.toml`
- Add all workspace crates as path dependencies
- Create `src/main.rs` with a top-level `Cli` struct and `Commands` enum covering all
  subcommands
- Implement `main()` as an async tokio entry point

### 8.2 Implement `lore install`

- Parse `<lib@version>` argument, splitting on `@`
- If version is omitted: call `RegistryClient::search()` and display interactive version
  picker using `dialoguer::Select`
- Show download progress with `indicatif::ProgressBar` in bytes transferred style
- Call the download and validation pipeline from `lore-registry`
- Print success message with package name, version, chunk count, and file size

### 8.3 Implement `lore remove`

- Parse `<lib@version>` argument
- Confirm deletion with `dialoguer::Confirm` unless `--force` flag is passed
- Delete the `.db` file from `~/.lore/packages/`
- Print confirmation

### 8.4 Implement `lore list`

- Scan `~/.lore/packages/` for all `.db` files
- Open each, read `meta` table for name/version/description/chunk count
- Display as a formatted table using `console`

### 8.5 Implement `lore get`

- Parse `<lib>` and `<topic>` arguments
- Accept optional `--budget`, `--threshold`, `--lambda` flags mapping to `SearchConfig`
- Open the package `Db`, run full search pipeline
- Print results to stdout with section separators, breadcrumb paths, and content
- Suitable for piping: add `--json` flag for machine-readable output

### 8.6 Implement `lore manifest`

- Parse `<lib>` argument
- Open the package `Db`, read `meta` key `"manifest"`
- Print manifest to stdout
- Add `--copy` flag (macOS: pipe to `pbcopy`) for one-command CLAUDE.md integration

### 8.7 Implement `lore search`

- Parse `<registry>` and `<name>` arguments
- Call `RegistryClient::search()`
- Display results as a table with name, version, description, size

### 8.8 Implement `lore build`

- Accept `<source>` positional argument
- Detect source type: local path (exists on disk), git URL (starts with https:// and ends
  with .git or matches github.com pattern), website URL, existing `.db` file
- Accept `--name` and `--version` flags (required for git/dir sources)
- Accept `--output` flag (default: current directory as `{name}@{version}.db`)
- Show build progress with a spinner and live chunk count
- Print `BuildStats` on completion

### 8.9 Implement `lore serve`

- Accept `--http` flag and `--port` argument (default: 3000)
- Initialise `LoreServer`
- If model not cached: show one-time download message with progress
- Launch stdio or HTTP transport
- Print "Lore MCP server running" to stderr (not stdout, which is the MCP channel)

### 8.10 Implement `lore info`

- Parse `<lib>` argument
- Open the package `Db`
- Display: name, version, description, source URL, git SHA, chunk count, code block count,
  total tokens, file size, build date (from meta table)

---

## Phase 9: Registry Client

**Goal:** Implement the HTTP registry client for searching, downloading, and publishing packages.

### 9.1 Implement RegistryClient

- Add `reqwest` (with `stream` feature), `serde_json`, `tokio` to `lore-registry/Cargo.toml`
- Create `src/client.rs`
- Implement `RegistryClient` struct holding base URL and optional auth token
- Implement `RegistryClient::default()` using the Neuledge API URL
- Implement `RegistryClient::search(registry, name, version) -> Result<Vec<PackageMetadata>>`
  via `GET /search`
- Implement `RegistryClient::get_package(registry, name, version) -> Result<PackageMetadata>`
  via `GET /packages/{registry}/{name}/{version}`

### 9.2 Implement package download

- Create `src/download.rs` in `lore-registry`
- Implement `download_package(client: &RegistryClient, registry, name, version, dest_dir: &Path, progress: Option<ProgressCallback>) -> Result<PathBuf>`:
  1. GET `/packages/{registry}/{name}/{version}/download` with streaming response
  2. Write chunks to `dest_dir/.downloading-{timestamp}-{name}.db` as they arrive
  3. Call progress callback with bytes received (for progress bar)
  4. On completion: open the temp file with `Db::open()`
  5. Validate: check that `meta`, `nodes`, `nodes_fts`, `node_embeddings` tables exist
  6. Validate: check that `meta` has `name` and `version` keys
  7. On validation success: move temp file to `dest_dir/{registry}-{name}@{version}.db`
  8. On validation failure: delete temp file, return error
- Implement `safe_filename(registry: &str, name: &str, version: &str) -> String`:
  - Replace `/` with `_` (for scoped npm packages like `@scope/pkg`)
  - Replace `@` with `@` in package names only (version separator is preserved)
  - Result: `npm-@scope_pkg@1.0.0.db`

### 9.3 Implement config file

- Create `src/config.rs` in `lore-registry`
- Implement `LoreConfig` struct: `packages_dir: PathBuf`, `cache_dir: PathBuf`,
  `registries: Vec<RegistryConfig>`, `default_registry: String`
- Implement `LoreConfig::load() -> Result<LoreConfig>`:
  - Read `~/.lore/config.json`, or return defaults if not present
  - Defaults: `packages_dir = ~/.lore/packages`, `cache_dir = ~/.cache/lore`,
    `default_registry = "https://api.context.neuledge.com"`
- Implement `LoreConfig::save() -> Result<()>`

---

## Phase 10: Custom Build Sources

**Goal:** Support building documentation packages from local directories, git repositories,
websites with llms.txt, and existing `.db` files.

### 10.1 Local directory source

- Create `src/sources/local.rs` in `lore-build`
- Implement `LocalSource::build(dir: &Path, meta: Package, output: &Path) -> Result<BuildStats>`:
  - Directly use `PackageBuilder` from Phase 5.3
  - No preprocessing needed

### 10.2 Git repository source

- Add `git2` to `lore-build/Cargo.toml`
- Create `src/sources/git.rs`
- Implement `GitSource::build(url: &str, tag: Option<&str>, docs_path: &str, meta: Package, output: &Path) -> Result<BuildStats>`:
  1. Create a temp directory
  2. Shallow clone (`depth=1`) the repository using `git2`
  3. If `tag` is provided: checkout the tag
  4. Navigate to `docs_path` within the cloned repo
  5. Call `LocalSource::build()` on that subdirectory
  6. Clean up temp directory
- Handle SSH and HTTPS URLs
- Show clone progress with indicatif spinner

### 10.3 Website / llms.txt source

- Add `reqwest`, `scraper` to `lore-build/Cargo.toml` (if not already present)
- Create `src/sources/website.rs`
- Implement `WebsiteSource::build(url: &str, meta: Package, output: &Path) -> Result<BuildStats>`:
  1. Fetch `{url}/llms.txt` — if present, parse it for a list of documentation URLs
  2. If no `llms.txt`: fetch the root URL and extract links matching the same domain
  3. For each URL: fetch the HTML, convert to Markdown via `HtmlParser`
  4. Write each page as a temp file
  5. Call `LocalSource::build()` on the temp directory
- Respect `robots.txt` — skip URLs disallowed by robots
- Concurrent fetching with a semaphore limiting to 5 simultaneous requests
- Detect and skip duplicate content via MD5 hash of page content

### 10.4 Existing .db file source

- Implement passthrough: if source is a `.db` file, validate schema and copy to output path
- This handles the case where a user has a pre-built package from another tool

---

## Phase 11: Manifest Generation

**Goal:** Implement manifest generation — a compressed ~500 token index of a package's
API surface, suitable for embedding in CLAUDE.md.

### 11.1 Implement heading tree extractor

- Create `src/manifest.rs` in `lore-build`
- Implement `extract_headings(db: &Db) -> Result<Vec<HeadingEntry>>` where `HeadingEntry`
  has: `path: Vec<String>`, `level: u8`, `title: String`
- Query all `Heading` nodes from the database, ordered by `path`

### 11.2 Implement API signature extractor

- Implement `extract_signatures(db: &Db) -> Result<Vec<ApiSignature>>` where `ApiSignature`
  has: `name: String`, `signature: String`, `heading_path: Vec<String>`
- Query all `CodeBlock` nodes
- For each code block with `lang` in `["js", "ts", "javascript", "typescript", "python",
  "rust", "go", "java"]`:
  - Extract function/class/const/type definitions using line-by-line heuristics:
    - Lines matching: `function name(`, `const name =`, `class Name`, `def name(`,
      `fn name(`, `type Name`, `interface Name`, `export function`, `export const`
  - Store the first line of each definition as the signature

### 11.3 Implement manifest formatter

- Implement `build_manifest(headings: Vec<HeadingEntry>, signatures: Vec<ApiSignature>) -> String`:
  - Group signatures by their top-level heading section
  - For each top-level section with signatures:
    - Output `SECTION_NAME: sig1, sig2, sig3`
  - Include sections with no signatures only if they are top-level sections
  - Target output of under 500 tokens (measured with `tiktoken-rs`)
  - If over 500 tokens: drop lower-level heading entries first, then trim signature lists
- Example output format:
  ```
  CACHING: use cache (directive), cacheLife(profile), cacheTag(...tags), revalidateTag(tag)
  ROUTING: forbidden(), unauthorized(), connection(), after(callback)
  METADATA: generateMetadata({params}), generateStaticParams()
  ```

### 11.4 Store manifest during build

- Call `build_manifest()` at the end of `PackageBuilder::build()`
- Store result in `meta` table with key `"manifest"`
- Include token count in `BuildStats`

---

## Phase 12: Registry Infrastructure

**Goal:** Implement the community package build and publish infrastructure.

### 12.1 Define YAML package definition format

- Create `registry/` directory at workspace root
- Define the YAML schema in `docs/REGISTRY_SCHEMA.md`
- Create initial package definitions for: `next`, `react`, `vue`, `svelte`, `astro`,
  `remix`, `hono`, `drizzle`, `prisma`, `zod`, `fastapi`, `django`, `flask`, `numpy`,
  `pandas`, `sqlalchemy`, `serde`, `tokio`, `axum`, `clap`

### 12.2 Implement version discovery

- Create `src/discover.rs` in `lore-registry`
- Implement `discover_npm_versions(package: &str) -> Result<Vec<String>>`
  via `https://registry.npmjs.org/{package}`
- Implement `discover_pypi_versions(package: &str) -> Result<Vec<String>>`
  via `https://pypi.org/pypi/{package}/json`
- Implement `discover_crates_versions(package: &str) -> Result<Vec<String>>`
  via `https://crates.io/api/v1/crates/{package}`
- Implement version filtering: remove pre-releases, keep latest patch per major.minor,
  limit to N most recent minor versions

### 12.3 Implement CI build and publish workflow

- Create `.github/workflows/registry.yml`
- Trigger: push to `registry/` directory on `main` branch
- For each modified YAML definition:
  1. Discover available versions
  2. Check which versions are already published
  3. Build missing versions using `lore build`
  4. Publish to registry API using `REGISTRY_PUBLISH_KEY` secret

---

## Phase 13: Testing

**Goal:** Comprehensive test coverage across all crates. Retrieval quality benchmarks.

### 13.1 Unit tests (embedded in each crate)

- `lore-core`: schema creation, CRUD operations, path enumeration queries
- `lore-build`: each parser, chunker, embedder, file discovery
- `lore-search`: FTS query sanitisation, RRF merge, MMR filter, token budget
- `lore-mcp`: tool parameter parsing, response formatting
- `lore-registry`: download/validate/install pipeline, config loading

### 13.2 Integration tests

- Full build pipeline from raw files to queryable `.db`
- Full search pipeline from query string to `SearchResult`
- MCP tool round-trip: install package → query via `get_docs` → verify results
- Download pipeline with mock HTTP server

### 13.3 Retrieval quality benchmarks

- Build a test package from a known documentation set (e.g., Next.js 15 docs)
- Define a set of 20 queries with known correct answers:
  - 5 queries with exact term overlap (should work with BM25 alone)
  - 5 queries with vocabulary mismatch (require vector search)
  - 5 queries for specific API names (test heading weight)
  - 5 queries for conceptual topics (test semantic chunking)
- Measure: recall@5, recall@10, MRR (Mean Reciprocal Rank)
- These benchmarks run in CI and fail if metrics drop below baseline

---

## Phase 14: Distribution

**Goal:** Package the `lore` binary for easy installation on macOS, Linux, and Windows.

### 14.1 Release binary builds

- Configure `cargo build --release` with optimisation flags in `.cargo/config.toml`:
  `opt-level = 3`, `lto = true`, `codegen-units = 1`
- Set up GitHub Actions release workflow:
  - Build for: `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`,
    `x86_64-pc-windows-msvc`
  - Use `cross` for Linux cross-compilation
  - Upload binaries as release assets

### 14.2 Install script

- Create `install.sh` for macOS/Linux: download correct binary for detected platform,
  place in `/usr/local/bin/lore`, verify with `lore --version`
- Create `install.ps1` for Windows

### 14.3 Homebrew formula

- Create `Formula/lore.rb` in a `homebrew-lore` tap repository
- Formula downloads the correct binary for the platform

### 14.4 User documentation

- Create `docs/INSTALL.md`: installation instructions for all platforms
- Create `docs/QUICKSTART.md`: first install, first package, CLAUDE.md setup
- Create `docs/CLAUDE_MD_INTEGRATION.md`: how to use `lore manifest` and structure CLAUDE.md
- Update `README.md` at workspace root
