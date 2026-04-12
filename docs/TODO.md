# Lore — TODO

Status legend: [ ] not started  [~] in progress  [x] complete

---

## Phase 1: Foundation

### 1.1 Convert to Cargo workspace

- [ ] Delete `src/main.rs` from `~/Coding/lore/`
- [ ] Delete `src/` directory from `~/Coding/lore/`
- [ ] Replace `~/Coding/lore/Cargo.toml` with a workspace manifest containing:
      `[workspace]`, `members = ["crates/*"]`, resolver = "2"
- [ ] Add `[workspace.dependencies]` section to root `Cargo.toml` with pinned versions for:
      `serde`, `thiserror`, `tokio`, `tokio-rusqlite`, `rusqlite`, `sqlite-vec`,
      `fastembed`, `tiktoken-rs`, `pulldown-cmark`, `scraper`, `htmd`, `git2`,
      `reqwest`, `rmcp`, `clap`, `indicatif`, `dialoguer`, `console`, `serde_json`,
      `serde_yaml`
- [ ] Create `~/Coding/lore/crates/` directory
- [ ] Run `cargo new --lib crates/lore-core` from workspace root
- [ ] Run `cargo new --lib crates/lore-build` from workspace root
- [ ] Run `cargo new --lib crates/lore-search` from workspace root
- [ ] Run `cargo new --lib crates/lore-mcp` from workspace root
- [ ] Run `cargo new --lib crates/lore-registry` from workspace root
- [ ] Run `cargo new --bin crates/lore-cli` from workspace root
- [ ] Run `cargo build` from workspace root and confirm all six crates compile
- [ ] Run `cargo test` from workspace root and confirm all six crates pass (empty tests)

### 1.2 Define core types in lore-core

- [ ] Add to `crates/lore-core/Cargo.toml`: `serde = { workspace = true, features = ["derive"] }`
- [ ] Add to `crates/lore-core/Cargo.toml`: `thiserror = { workspace = true }`
- [ ] Add to `crates/lore-core/Cargo.toml`: `rusqlite = { workspace = true, features = ["bundled"] }`
- [ ] Add to `crates/lore-core/Cargo.toml`: `tokio-rusqlite = { workspace = true }`
- [ ] Add to `crates/lore-core/Cargo.toml`: `sqlite-vec = { workspace = true }`
- [ ] Create `crates/lore-core/src/error.rs` with `LoreError` enum using `thiserror::Error`:
      variants: `Database(#[from] rusqlite::Error)`, `Io(#[from] std::io::Error)`,
      `Parse(String)`, `Embed(String)`, `Schema(String)`, `Registry(String)`,
      `NotFound(String)`, `InvalidConfig(String)`
- [ ] Create `crates/lore-core/src/node.rs` with:
      `NodeKind` enum: `Heading`, `Chunk`, `CodeBlock` (with `serde` derives)
      `Node` struct: all fields as per ARCHITECTURE.md (with `serde` derives and `Clone`)
      `NewNode` struct: same fields minus `id` (for insertion)
- [ ] Create `crates/lore-core/src/package.rs` with:
      `Package` struct: `name`, `registry`, `version`, `description`, `source_url`, `git_sha`
      `PackageMetadata` struct: `name`, `registry`, `version`, `description`, `size_bytes`,
      `chunk_count`, `build_date`
- [ ] Create `crates/lore-core/src/search.rs` with:
      `SearchConfig` struct with fields and `Default` impl as per ARCHITECTURE.md
      `ScoredNode` struct: `node: Node`, `score: f64`
      `SearchResult` struct: `node: Node`, `doc_title: String`, `heading_path: Vec<String>`,
      `score: f64`, `content: String`
- [ ] Create `crates/lore-core/src/doc.rs` with `Doc` struct: `id: i64`, `path: String`,
      `title: Option<String>`
- [ ] Update `crates/lore-core/src/lib.rs` to `pub mod` all new modules and `pub use` all
      public types at the crate root
- [ ] Run `cargo build -p lore-core` and confirm it compiles without warnings

### 1.3 Database connection and schema management

- [ ] Create `crates/lore-core/src/db.rs`
- [ ] Define `Db` struct with a single field: `conn: tokio_rusqlite::Connection`
- [ ] Implement `Db::open(path: &Path) -> Result<Db, LoreError>`:
      - Call `tokio_rusqlite::Connection::open(path).await`
      - Inside `conn.call`: call `sqlite_vec::load(&conn)` to register the extension
      - Inside `conn.call`: execute `PRAGMA journal_mode = WAL`
      - Inside `conn.call`: execute `PRAGMA synchronous = NORMAL`
      - Inside `conn.call`: execute `PRAGMA foreign_keys = ON`
      - Call `run_migrations(&conn).await`
- [ ] Implement private `run_migrations(conn: &tokio_rusqlite::Connection) -> Result<()>`:
      - Create `meta` table if not exists (key TEXT PRIMARY KEY, value TEXT NOT NULL)
      - Read `schema_version` from `meta` table (0 if missing)
      - Apply migration 1 if version < 1: create `docs` table, `nodes` table with all columns
        and CHECK constraint on `kind`, all three indexes on `nodes`
      - Apply migration 2 if version < 2: create `nodes_fts` FTS5 virtual table with
        `content='nodes'`, `content_rowid='id'`, `title`, `content`,
        `tokenize='porter unicode61'`
      - Apply migration 3 if version < 3: create `node_embeddings` vec0 virtual table with
        `embedding FLOAT[384]`
      - Apply migration 4 if version < 4: create the three FTS5 content sync triggers
        (after_nodes_insert, after_nodes_delete, after_nodes_update)
      - After each migration: update `schema_version` in `meta` table
- [ ] Implement `Db::set_meta(key: &str, value: &str) -> Result<()>`
- [ ] Implement `Db::get_meta(key: &str) -> Result<Option<String>>`
- [ ] Implement `Db::insert_doc(path: &str, title: Option<&str>) -> Result<i64>`
- [ ] Implement `Db::insert_node(node: &NewNode) -> Result<i64>`
- [ ] Implement `Db::insert_embedding(node_id: i64, embedding: &[f32]) -> Result<()>`:
      - Serialise `embedding` as packed little-endian f32 bytes
      - Execute `INSERT INTO node_embeddings(rowid, embedding) VALUES (?, ?)`
- [ ] Implement `Db::get_node(id: i64) -> Result<Node>`
- [ ] Implement `Db::get_children(parent_id: i64) -> Result<Vec<Node>>`
- [ ] Implement `Db::get_ancestors(node_id: i64) -> Result<Vec<Node>>`:
      - Use the `path` column: split on `/`, collect ancestor ids, query each
- [ ] Implement `Db::get_heading_path(node_id: i64) -> Result<Vec<String>>`:
      - Fetch all ancestor nodes where `kind = 'heading'`
      - Return their `title` fields in order from root to nearest ancestor
- [ ] Implement `Db::get_package_meta() -> Result<Package>`:
      - Read `name`, `version`, `registry`, `description`, `source_url`, `git_sha` from `meta`
- [ ] Implement `Db::rebuild_fts() -> Result<()>`:
      - Execute `INSERT INTO nodes_fts(nodes_fts) VALUES('rebuild')`
- [ ] Add `pub mod db` and `pub use db::Db` to `src/lib.rs`

### 1.4 Verify foundation

- [ ] Write `crates/lore-core/tests/db_test.rs` with a test `test_open_and_migrate`:
      - Call `Db::open(":memory:")` (use temp file since tokio-rusqlite may not support :memory: directly)
      - Call `db.set_meta("name", "test-pkg")`
      - Call `db.get_meta("name")` and assert it equals `Some("test-pkg")`
- [ ] Write test `test_insert_and_retrieve_node`:
      - Insert a doc, insert a heading node, insert a chunk node as child
      - Assert `get_node` returns the chunk with correct fields
      - Assert `get_children` returns the chunk when called with the heading's id
      - Assert `get_heading_path` returns the heading title
- [ ] Write test `test_fts5_created`:
      - After `Db::open`, execute a raw query `SELECT name FROM sqlite_master WHERE type='table' AND name='nodes_fts'`
      - Assert the result is non-empty
- [ ] Write test `test_vec0_created`:
      - After `Db::open`, execute `SELECT name FROM sqlite_master WHERE type='table' AND name='node_embeddings'`
      - Assert the result is non-empty
- [ ] Run `cargo test -p lore-core` and confirm all tests pass

---

## Phase 2: Document Parsing

### 2.1 Parser trait and registry

- [ ] Add to `crates/lore-build/Cargo.toml`: `lore-core = { path = "../lore-core" }`
- [ ] Add to `crates/lore-build/Cargo.toml`: `pulldown-cmark = { workspace = true }`
- [ ] Add to `crates/lore-build/Cargo.toml`: `scraper = { workspace = true }`
- [ ] Add to `crates/lore-build/Cargo.toml`: `htmd = { workspace = true }`
- [ ] Create `crates/lore-build/src/parser/mod.rs` with:
      - `Parser` trait: `can_parse(&self, path: &Path) -> bool` and
        `parse(&self, content: &str, path: &Path) -> Result<ParsedDoc, LoreError>`
      - `ParsedDoc` struct: `title: Option<String>`, `root: HeadingNode`
      - `HeadingNode` struct: `level: u8`, `title: String`, `blocks: Vec<ContentBlock>`,
        `children: Vec<HeadingNode>`
      - `ContentBlock` enum: `Paragraph(String)`, `Code { lang: Option<String>, content: String }`,
        `Table(String)`, `Other(String)`
      - `ParserRegistry` struct with a `Vec<Box<dyn Parser + Send + Sync>>`
      - `ParserRegistry::new() -> ParserRegistry` that adds all four parsers
      - `ParserRegistry::parse(path: &Path, content: &str) -> Result<ParsedDoc, LoreError>`
        that iterates parsers, calls `can_parse`, and delegates to the first match
      - `detect_primary_heading_level(root: &HeadingNode) -> u8` function
- [ ] Implement `detect_primary_heading_level`:
      - Walk the tree, count heading nodes at each level (2, 3, 4)
      - For each level, compute average content block count of nodes at that level
      - Return the shallowest level where average content block count > 1.5
      - Default to 2 if no level meets the criterion
      - Return 2 if the tree has no children (flat document)

### 2.2 MarkdownParser

- [ ] Create `crates/lore-build/src/parser/markdown.rs`
- [ ] Implement `can_parse` returning true for: `.md`, `.mdx`, `.qmd`, `.rmd`
- [ ] Implement frontmatter extraction:
      - If content starts with `---\n`: find the closing `---\n`
      - Extract the YAML between them
      - Parse as key-value pairs, extract `title` field if present
      - Strip the frontmatter from content before passing to pulldown-cmark
- [ ] Implement `parse` using pulldown-cmark `Parser` with `Options::all()`:
      - Maintain a stack of `HeadingNode` to track nesting
      - On `Event::Start(Tag::Heading(level, ..))`: push a new `HeadingNode` onto the stack
      - On `Event::End(Tag::Heading(..))`: pop from stack, attach to parent
      - On `Event::Text` within a heading: accumulate the heading title string
      - On `Event::Start(Tag::Paragraph)`: begin accumulating paragraph text
      - On `Event::End(Tag::Paragraph)`: push `ContentBlock::Paragraph` to current heading
      - On `Event::Start(Tag::CodeBlock(kind))`: note language from `kind`
      - On `Event::Text` within a code block: accumulate code content
      - On `Event::End(Tag::CodeBlock(..))`: push `ContentBlock::Code { lang, content }`
      - On `Event::Start(Tag::Table(..))`: begin accumulating table content
      - On `Event::End(Tag::Table(..))`: push `ContentBlock::Table`
      - Strip MDX JSX tags from text: remove patterns matching `<[A-Z][a-zA-Z]*[^>]*>` and
        `</[A-Z][a-zA-Z]*>`
      - TOC detection: after building tree, remove heading nodes where title (lowercased) is
        one of: "table of contents", "contents", "toc", "on this page", "in this article"
        OR where the `Paragraph` blocks contain >60% link patterns `[text](url)`
- [ ] Write test `test_basic_markdown`: H1 + two H2 sections each with a paragraph
- [ ] Write test `test_frontmatter`: document with `---\ntitle: My Doc\n---` frontmatter
- [ ] Write test `test_code_block`: document with a fenced Rust code block
- [ ] Write test `test_nested_headings`: H2 containing H3 children
- [ ] Write test `test_mdx_tag_stripping`: content containing `<AppOnly>` JSX tags
- [ ] Write test `test_toc_skipped`: document with a "Table of Contents" section

### 2.3 HtmlParser

- [ ] Create `crates/lore-build/src/parser/html.rs`
- [ ] Implement `can_parse` returning true for `.html`, `.htm`
- [ ] Implement `parse`:
      - Parse HTML with `scraper::Html::parse_document`
      - Build a scraper `Selector` for: `script`, `style`, `nav`, `footer`, `header`,
        `[role="navigation"]`, `[role="banner"]`, `[role="contentinfo"]`
      - Remove matched elements by collecting their inner text and replacing with empty
        (scraper is immutable — rebuild the HTML string without those tags using string
        replacement, or use htmd with a custom config that ignores those tags)
      - Extract document title: try `title` element first, then first `h1` element
      - Use `htmd::convert` to convert the cleaned HTML to Markdown
      - Pass the result to `MarkdownParser.parse()`
- [ ] Write test `test_html_basic`: simple HTML page with headings and paragraphs
- [ ] Write test `test_html_strips_nav`: HTML with `<nav>` block that should be excluded

### 2.4 AsciidocParser

- [ ] Create `crates/lore-build/src/parser/asciidoc.rs`
- [ ] Implement `can_parse` returning true for `.adoc`, `.asciidoc`
- [ ] Implement `parse` with line-by-line processing:
      - Line 1 matching `= Title`: document title
      - Lines matching `^(={2,6}) (.+)$` regex: heading with level = (number of `=`) - 1
      - Lines `[source,lang]` followed by `----` block: start code block with `lang`
      - Lines `----` alone after a `[source]` attribute: end code block
      - All other lines between headings: accumulate as paragraph text
      - Blank lines separate paragraphs
      - Build `HeadingNode` tree by tracking heading level stack
- [ ] Write test `test_asciidoc_basic`: document with `= Title`, `== Section`, `=== Subsection`
- [ ] Write test `test_asciidoc_source_block`: `[source,java]` followed by `----` block

### 2.5 RstParser

- [ ] Create `crates/lore-build/src/parser/rst.rs`
- [ ] Implement `can_parse` returning true for `.rst`
- [ ] Implement `parse` with line-by-line processing:
      - Heading detection: line N is a heading if line N+1 is all the same character
        (`=`, `-`, `~`, `^`, `"`, `#`, `*`) and length >= length of line N
      - Track which underline characters have been seen (in order of first appearance)
        to assign heading levels 1, 2, 3, etc.
      - `.. code-block:: lang` directive followed by indented block: code block
      - `.. code::` directive: same
      - A paragraph ending with `::` followed by an indented block: anonymous code block
      - All other non-directive lines: paragraph text
      - Blank lines separate paragraphs
- [ ] Write test `test_rst_headings`: document using `=` and `-` underlines
- [ ] Write test `test_rst_code_block`: `.. code-block:: python` directive

### 2.6 Parser integration tests

- [ ] Create `crates/lore-build/tests/parser_test.rs`
- [ ] Write test `test_detect_primary_level_api_reference`:
      - Create a `HeadingNode` tree simulating an API reference (many H3 nodes with code blocks)
      - Assert `detect_primary_heading_level` returns 3
- [ ] Write test `test_detect_primary_level_tutorial`:
      - Create a tree with meaty H2 sections
      - Assert returns 2
- [ ] Run `cargo test -p lore-build` and confirm all parser tests pass

---

## Phase 3: Chunking Pipeline

### 3.1 Token counter

- [ ] Add to `crates/lore-build/Cargo.toml`: `tiktoken-rs = { workspace = true }`
- [ ] Create `crates/lore-build/src/tokens.rs`
- [ ] Implement `TokenCounter` struct with a `tiktoken_rs::CoreBPE` field
- [ ] Implement `TokenCounter::new() -> Result<TokenCounter, LoreError>`:
      - Call `tiktoken_rs::cl100k_base()` and store the result
- [ ] Implement `TokenCounter::count(&self, text: &str) -> u32`:
      - Call `self.bpe.encode_with_special_tokens(text).len() as u32`
- [ ] Write test `test_token_count_prose`: count tokens in a 100-word paragraph and assert
      the result is between 80 and 130
- [ ] Write test `test_token_count_code`: count tokens in a 10-line Rust function and assert
      the result is more than `text.len() / 6` (code tokenises more than prose per character)

### 3.2 Structural chunker

- [ ] Create `crates/lore-build/src/chunker/mod.rs`
- [ ] Create `crates/lore-build/src/chunker/structural.rs`
- [ ] Define `ChunkConfig` struct:
      `min_tokens: u32 = 50`, `soft_max_tokens: u32 = 800`, `hard_max_tokens: u32 = 1200`
- [ ] Define `RawChunk` struct:
      `heading_path: Vec<String>`, `blocks: Vec<ContentBlock>`, `token_count: u32`,
      `has_code: bool`, `needs_refinement: bool`, `doc_path: String`, `doc_title: Option<String>`,
      `kind: NodeKind`
- [ ] Define `ChunkTree` struct: list of `(RawChunk, Option<usize>)` where the second element
      is the index of the parent `RawChunk` in the list
- [ ] Implement `StructuralChunker::new(config: ChunkConfig, counter: TokenCounter) -> StructuralChunker`
- [ ] Implement `StructuralChunker::chunk(doc: &ParsedDoc, primary_level: u8) -> ChunkTree`:
      - Call the recursive `walk` function starting at the root `HeadingNode`
      - `walk(node: &HeadingNode, primary_level: u8, parent_idx: Option<usize>, path: Vec<String>, tree: &mut ChunkTree)`:
        - If `node.level == primary_level` or node has blocks and no heading parent:
          - Separate `node.blocks` into prose blocks and code blocks
          - Each code block becomes its own `RawChunk` with `kind = CodeBlock`
          - Remaining prose blocks are accumulated into one `RawChunk` with `kind = Chunk`
          - Count tokens for each chunk
          - If prose chunk `token_count > soft_max_tokens`: set `needs_refinement = true`
          - Add both to `tree` with correct parent index
        - Recurse into `node.children`
- [ ] Write test `test_chunk_flat_document`: 3 H2 sections → 3 chunks
- [ ] Write test `test_chunk_code_block_atomic`: H2 section with a large code block →
      code block chunk is separate and unsplit
- [ ] Write test `test_chunk_api_reference`: H2 with 10 H3 children → 10 separate chunks
- [ ] Write test `test_chunk_marks_large_for_refinement`: section with 900 tokens of prose →
      `needs_refinement = true`

### 3.3 Semantic refinement

- [ ] Create `crates/lore-build/src/chunker/semantic.rs`
- [ ] Implement `cosine_similarity(a: &[f32], b: &[f32]) -> f32`:
      dot product of a and b divided by (magnitude of a × magnitude of b)
- [ ] Implement `SemanticRefiner::refine(chunk: RawChunk, embedder: &Embedder) -> Vec<RawChunk>`:
      - If `!chunk.needs_refinement` or chunk has fewer than 2 prose paragraphs: return `vec![chunk]`
      - Extract individual paragraphs from `chunk.blocks` (split on blank lines, exclude code blocks)
      - If fewer than 3 paragraphs: return `vec![chunk]` (insufficient data for statistics)
      - Embed each paragraph individually (no breadcrumb — these are short fragments)
      - Compute similarity between each consecutive pair
      - Compute mean and std_dev of all similarities
      - Identify split positions: indices where `similarity[i] < mean - 1.5 * std_dev`
      - If no split positions: return `vec![chunk]`
      - Build new `RawChunk` instances for each split segment
      - For each resulting chunk with `token_count < min_tokens`: merge into adjacent chunk
        (prefer merging with the following chunk, fall back to preceding)
      - Return the list of refined `RawChunk` instances
- [ ] Write test `test_no_split_single_topic`: paragraph sequence all about the same topic →
      single chunk returned unchanged
- [ ] Write test `test_splits_multi_topic`: two clearly different topics separated by a
      paragraph boundary → two chunks returned
- [ ] Write test `test_merges_tiny_fragment`: refinement that would produce a 20-token fragment →
      fragment is merged back

---

## Phase 4: Embedding Pipeline

### 4.1 Embedder

- [ ] Add to `crates/lore-build/Cargo.toml`: `fastembed = { workspace = true }`
- [ ] Create `crates/lore-build/src/embedder.rs`
- [ ] Implement `Embedder` struct wrapping `fastembed::TextEmbedding`
- [ ] Implement `Embedder::new(cache_dir: &Path) -> Result<Embedder, LoreError>`:
      - Check if model files exist in `cache_dir/bge-small-en-v1.5/`
      - If not: print to stderr "Downloading embedding model bge-small-en-v1.5 (~130MB)..."
      - Initialise `fastembed::TextEmbedding` with `EmbeddingModel::BGESmallENV15`
        and `cache_dir` as the model directory
- [ ] Implement `Embedder::embed(&self, text: &str) -> Result<Vec<f32>, LoreError>`
- [ ] Implement `Embedder::embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, LoreError>`:
      - Use fastembed's batch method with a batch size of 32
- [ ] Implement `Embedder::dimensions() -> usize` returning 384
- [ ] Write test `test_embed_returns_384_dims`: embed a short string, assert result len == 384
- [ ] Write test `test_similar_texts_higher_similarity`: embed "dog" and "puppy", embed "dog"
      and "quantum mechanics", assert the first pair has higher cosine similarity

### 4.2 Contextual embedding helper

- [ ] Create `crates/lore-build/src/embedder/contextual.rs` (or add to `embedder.rs`)
- [ ] Implement `build_contextual_text(heading_path: &[String], content: &str) -> String`:
      - If `heading_path` is empty: return `content.to_string()`
      - Join path with " > "
      - Return `format!("{}\n\n{}", joined_path, content)`
- [ ] Write test `test_contextual_text_format`:
      heading_path = ["Next.js", "Caching", "cacheLife()"], content = "Controls TTL."
      assert output = "Next.js > Caching > cacheLife()\n\nControls TTL."
- [ ] Write test `test_contextual_text_empty_path`:
      heading_path = [], assert output equals content unchanged

---

## Phase 5: Database & Indexing

### 5.1 File discovery

- [ ] Create `crates/lore-build/src/discovery.rs`
- [ ] Implement `INCLUDED_EXTENSIONS: &[&str]` constant:
      `["md", "mdx", "qmd", "rmd", "html", "htm", "adoc", "asciidoc", "rst"]`
- [ ] Implement `EXCLUDED_DIRS: &[&str]` constant:
      `["node_modules", ".git", "__pycache__", "target", "dist", "build", ".next",
       ".nuxt", ".svelte-kit", "vendor"]`
- [ ] Implement `EXCLUDED_NAMES: &[&str]` constant:
      `["CHANGELOG", "CODE_OF_CONDUCT", "LICENSE", "CONTRIBUTING", "AUTHORS",
       "CODEOWNERS"]`
- [ ] Implement `discover_files(root: &Path, exclude_examples: bool) -> Result<Vec<PathBuf>>`:
      - Walk directory recursively (use `walkdir` crate — add to Cargo.toml)
      - Skip entries whose ancestor path contains any string from `EXCLUDED_DIRS`
      - Skip files whose stem (without extension) matches any string from `EXCLUDED_NAMES`
        (case-insensitive)
      - If `exclude_examples`: also skip entries under dirs named `examples`, `example`,
        `fixtures`, `fixture`, `test`, `tests`, `spec`, `specs`
      - Include files whose extension matches `INCLUDED_EXTENSIONS`
      - Sort results lexicographically for deterministic ordering
- [ ] Add `walkdir = { workspace = true }` to `crates/lore-build/Cargo.toml` and
      to `[workspace.dependencies]` in root `Cargo.toml`
- [ ] Write test `test_discovers_md_files`: temp dir with 3 `.md` files → all found
- [ ] Write test `test_excludes_node_modules`: temp dir with `node_modules/docs.md` → not found
- [ ] Write test `test_excludes_changelog`: temp dir with `CHANGELOG.md` → not found

### 5.2 Indexer

- [ ] Create `crates/lore-build/src/indexer.rs`
- [ ] Define `Indexer` struct:
      `db: Db`, `parsers: ParserRegistry`, `chunker: StructuralChunker`,
      `refiner: SemanticRefiner`, `embedder: Embedder`, `counter: TokenCounter`
- [ ] Implement `Indexer::new(db: Db, embedder: Embedder) -> Result<Indexer>`:
      - Initialise `ParserRegistry::new()`
      - Initialise `StructuralChunker::new(ChunkConfig::default(), TokenCounter::new()?)`
      - Initialise `SemanticRefiner` with a reference to the embedder
- [ ] Implement `Indexer::index_file(&self, file_path: &Path, content: &str) -> Result<u32>`:
      returns count of chunks inserted
      1. Select parser, parse content → `ParsedDoc`
      2. Call `detect_primary_heading_level`
      3. Call `chunker.chunk(&doc, primary_level)` → `ChunkTree`
      4. For chunks with `needs_refinement`: call `refiner.refine(chunk, &self.embedder)`
      5. Insert `Doc` record: `db.insert_doc(file_path.to_str(), doc.title.as_deref()).await`
      6. Walk the `ChunkTree`, inserting nodes in parent-before-child order:
         - For each heading in `heading_path`: insert a `Heading` node if not already inserted
           (deduplicate by checking if an identical path already exists)
         - Build the node's `path` string from the parent's `path` + "/" + new id
         - Insert the `Chunk` or `CodeBlock` node
         - Build contextual text: `build_contextual_text(&heading_path, &content)`
         - Embed the contextual text: `self.embedder.embed(&contextual_text)`
         - Insert embedding: `db.insert_embedding(node_id, &embedding)`
      7. Return total chunks inserted

### 5.3 Package builder

- [ ] Create `crates/lore-build/src/builder.rs`
- [ ] Define `BuildStats` struct:
      `file_count: u32`, `chunk_count: u32`, `code_block_count: u32`,
      `heading_count: u32`, `total_tokens: u64`, `duration_secs: f64`
- [ ] Implement `BuildStats::display(&self)` formatting a summary table
- [ ] Define `PackageBuilder` struct holding `Indexer` and `LoreConfig` (packages dir, cache dir)
- [ ] Implement `PackageBuilder::build(source_dir: &Path, meta: Package, output: &Path) -> Result<BuildStats>`:
      1. Create/open `Db` at `output` path
      2. Write all `Package` fields to `meta` table
      3. Write build date to `meta` table as `"build_date"` key (ISO 8601 string)
      4. Call `discover_files(source_dir, false)`
      5. Record start time
      6. For each file:
         - Read file contents as UTF-8 string (skip on read error, log warning)
         - Call `indexer.index_file(path, &content)`
         - Accumulate stats
      7. Call `db.rebuild_fts()` to populate FTS5 index
      8. Call `build_manifest(&db)` (Phase 11) and store in meta
      9. Execute `PRAGMA optimize`
      10. Execute `VACUUM`
      11. Record end time, compute duration
      12. Return `BuildStats`

### 5.4 Integration test for full build pipeline

- [ ] Create `crates/lore-build/tests/build_test.rs`
- [ ] Create a test helper `create_test_docs(dir: &Path)` that writes 5 synthetic `.md` files:
      - `api.md`: H2 sections with H3 function descriptions and code blocks
      - `guide.md`: H2 tutorial sections with prose and examples
      - `config.md`: flat H2 sections with configuration reference
      - `CHANGELOG.md`: should be excluded by discovery
      - `node_modules/internal.md`: should be excluded by discovery
- [ ] Write test `test_full_build_pipeline`:
      - Call `PackageBuilder::build()` on the test docs directory
      - Assert `stats.file_count == 3` (CHANGELOG and node_modules excluded)
      - Assert `stats.chunk_count > 0`
      - Assert `stats.code_block_count > 0`
      - Open the resulting `.db` and run `SELECT count(*) FROM nodes` → assert > 0
      - Run `SELECT count(*) FROM node_embeddings` → assert equals node count for chunks
      - Run FTS5 query for a known term → assert results > 0
      - Run a `get_meta("manifest")` query → assert non-empty string returned

---

## Phase 6: Search Engine

### 6.1 FTS5 search

- [ ] Add to `crates/lore-search/Cargo.toml`: `lore-core = { path = "../lore-core" }`,
      `lore-build = { path = "../lore-build" }`, `tokio = { workspace = true }`
- [ ] Create `crates/lore-search/src/fts.rs`
- [ ] Implement `sanitize_query(query: &str) -> String`:
      - Remove chars: `( ) [ ] { } : * ^ ~ \\ / | & ! < >`
      - Collapse multiple spaces to single space
      - Trim leading/trailing whitespace
- [ ] Implement `fts_search(db: &Db, query: &str, limit: usize) -> Result<Vec<ScoredNode>>`:
      - Call `sanitize_query`
      - If result is empty: return `Ok(vec![])`
      - Execute:
        ```sql
        SELECT nodes.id, nodes.parent_id, nodes.path, nodes.doc_id, nodes.kind,
               nodes.level, nodes.title, nodes.content, nodes.token_count, nodes.lang,
               bm25(nodes_fts, 5.0, 10.0) AS score
        FROM nodes_fts
        JOIN nodes ON nodes.id = nodes_fts.rowid
        WHERE nodes_fts MATCH ?1
        ORDER BY score
        LIMIT ?2
        ```
        (Note: FTS5 bm25() returns negative values — lower is better. Sort ascending.)
      - Map to `ScoredNode` with `score = -bm25_score` (negate to make higher = better)
- [ ] Write test `test_fts_basic_match`: build mini DB, search for exact term, assert result
- [ ] Write test `test_fts_sanitizes_parens`: query "getData()" returns results for "getData"
- [ ] Write test `test_fts_empty_query`: sanitised empty query returns empty vec

### 6.2 Vector search

- [ ] Create `crates/lore-search/src/vec_search.rs`
- [ ] Implement `vec_search(db: &Db, embedding: &[f32], limit: usize) -> Result<Vec<ScoredNode>>`:
      - Serialise `embedding` as little-endian f32 bytes
      - Execute:
        ```sql
        SELECT nodes.id, nodes.parent_id, nodes.path, nodes.doc_id, nodes.kind,
               nodes.level, nodes.title, nodes.content, nodes.token_count, nodes.lang,
               ne.distance
        FROM node_embeddings ne
        JOIN nodes ON nodes.id = ne.rowid
        WHERE ne.embedding MATCH ?1
        ORDER BY ne.distance
        LIMIT ?2
        ```
      - Map to `ScoredNode` with `score = 1.0 - distance`
- [ ] Write test `test_vec_search_returns_results`: build mini DB with embeddings, query with
      a similar embedding, assert non-empty results

### 6.3 RRF merge

- [ ] Create `crates/lore-search/src/rrf.rs`
- [ ] Implement `rrf_merge(list_a: &[ScoredNode], list_b: &[ScoredNode], k: f64) -> Vec<ScoredNode>`:
      - Build `HashMap<i64, f64>` for rrf scores, keyed by node id
      - Build `HashMap<i64, Node>` for node data (list_a takes precedence for duplicates)
      - For each (rank, node) in list_a: add `1.0 / (k + rank as f64 + 1.0)` to score map
      - For each (rank, node) in list_b: add `1.0 / (k + rank as f64 + 1.0)` to score map
      - Collect into `Vec<ScoredNode>`, sort descending by rrf score
      - Return the merged list
- [ ] Write test `test_rrf_combines_both_lists`: a document in list_a rank 1 and list_b rank 3
      should outscore a document only in list_a at rank 2
- [ ] Write test `test_rrf_handles_list_only_in_one`: document in only one list still appears

### 6.4 MMR filter

- [ ] Create `crates/lore-search/src/mmr.rs`
- [ ] Implement `cosine_similarity(a: &[f32], b: &[f32]) -> f64`
- [ ] Implement `mmr_filter(candidates: Vec<ScoredNode>, embeddings: &HashMap<i64, Vec<f32>>, lambda: f64, limit: usize) -> Vec<ScoredNode>`:
      - Greedy MMR loop as described in PLAN.md
      - If a candidate has no embedding in the map: use its relevance score alone (treat
        max_similarity as 0.0 for that candidate)
- [ ] Write test `test_mmr_removes_duplicate`: two near-identical nodes → only one selected
- [ ] Write test `test_mmr_lambda_1_is_pure_relevance`: with λ=1.0, order should match
      original relevance ranking

### 6.5 Small-to-big retrieval

- [ ] Create `crates/lore-search/src/expand.rs`
- [ ] Implement `fetch_embedding(db: &Db, node_id: i64) -> Result<Option<Vec<f32>>>`:
      - Query `node_embeddings` for the row with `rowid = node_id`
      - Deserialise the BLOB as packed f32 bytes
- [ ] Implement `expand_to_parent(db: &Db, nodes: Vec<ScoredNode>, budget: u32) -> Result<Vec<ScoredNode>>`:
      - For each `ScoredNode` with `kind == Chunk`:
        - Fetch parent node via `db.get_node(parent_id)`
        - If parent is a `Heading` and parent's total descendant token count fits in budget:
          replace with parent `ScoredNode` (inherit original chunk's score)
        - Otherwise: keep the chunk
      - For `CodeBlock` nodes: keep as-is

### 6.6 Token budget

- [ ] Create `crates/lore-search/src/budget.rs`
- [ ] Implement `apply_token_budget(nodes: Vec<ScoredNode>, budget: u32) -> Vec<ScoredNode>`:
      - Accumulate `token_count` while adding nodes
      - Include a node if `running_total + node.token_count <= budget`
      - Stop after the first node that would exceed the budget (do not skip and continue)
- [ ] Write test `test_budget_stops_at_limit`
- [ ] Write test `test_budget_includes_exactly_at_limit`

### 6.7 Top-level search function

- [ ] Create `crates/lore-search/src/search.rs`
- [ ] Implement `search(db: &Db, embedder: &Embedder, query: &str, config: &SearchConfig) -> Result<Vec<SearchResult>>` following the full pipeline in PLAN.md Phase 6.7
- [ ] Fetch embeddings for candidate nodes after RRF step:
      - `db.conn.call(|conn| { /* SELECT rowid, embedding FROM node_embeddings WHERE rowid IN (...) */ })`
- [ ] Build `SearchResult` for each final node:
      - Call `db.get_heading_path(node.id)` for `heading_path`
      - `content = node.content.clone().unwrap_or_default()`
      - Fetch doc title via `SELECT title FROM docs WHERE id = ?`
- [ ] Re-export `search` from `crates/lore-search/src/lib.rs`

### 6.8 Search integration tests

- [ ] Create `crates/lore-build/tests/search_integration_test.rs`
      (or a separate integration test binary)
- [ ] Build a test package from synthetic docs covering known topics
- [ ] Write test `test_exact_term_match`: query "cacheLife" → top result mentions cacheLife
- [ ] Write test `test_vocabulary_mismatch`: query "cache expiry" → result about cacheLife/TTL
- [ ] Write test `test_no_split_code_in_results`: iterate all results, assert no ContentBlock
      boundary appears mid-code-fence
- [ ] Write test `test_token_budget_respected`: assert sum of token_count in results ≤ 2000

---

## Phase 7: MCP Server

### 7.1 Setup

- [ ] Add to `crates/lore-mcp/Cargo.toml`: `rmcp = { workspace = true }`,
      `lore-core`, `lore-build`, `lore-search`, `lore-registry` (path deps),
      `tokio`, `serde_json`, `serde`
- [ ] Create `crates/lore-mcp/src/server.rs` with `LoreServer` struct and `PackageStore` type alias
- [ ] Implement `LoreServer::new(packages_dir: &Path, cache_dir: &Path) -> Result<LoreServer>`:
      - Scan `packages_dir` for `*.db` files using `glob` or `std::fs::read_dir`
      - For each: open `Db`, read package meta, store in `PackageStore` keyed by
        `"{registry}-{name}@{version}"` string
      - Initialise shared `Embedder`

### 7.2 get_docs tool

- [ ] Create `crates/lore-mcp/src/tools/get_docs.rs`
- [ ] Define `GetDocsInput` struct with `library: String`, `topic: String`,
      `config: Option<SearchConfig>` (serde-deserializable)
- [ ] Define `GetDocsOutput` struct with `library: String`, `version: String`,
      `results: Vec<GetDocsResultItem>`
- [ ] Define `GetDocsResultItem`: `doc_path: String`, `heading_path: String`,
      `content: String`, `token_count: u32`
- [ ] Implement `handle_get_docs(server: &LoreServer, input: GetDocsInput) -> Result<GetDocsOutput>`:
      - Look up `input.library` in `PackageStore` → return `LoreError::NotFound` if missing
      - Run `lore_search::search(&db, &server.embedder, &input.topic, &config)`
      - Map `SearchResult` vec to `GetDocsResultItem` vec (join heading_path with " > ")
      - Return `GetDocsOutput`
- [ ] Register with `rmcp` as a tool named `"get_docs"` with description and parameter schema

### 7.3 search_packages tool

- [ ] Create `crates/lore-mcp/src/tools/search_packages.rs`
- [ ] Define `SearchPackagesInput` and `SearchPackagesOutput` types
- [ ] Implement handler calling `RegistryClient::search()`
- [ ] Register with `rmcp`

### 7.4 download_package tool

- [ ] Create `crates/lore-mcp/src/tools/download_package.rs`
- [ ] Implement handler calling the download pipeline from `lore-registry`
- [ ] After successful download: call `server.reload_packages()` to add new `Db` to `PackageStore`
- [ ] Implement `LoreServer::reload_packages()`: re-scan `packages_dir`, update `PackageStore`
- [ ] Register with `rmcp`

### 7.5 get_manifest tool

- [ ] Create `crates/lore-mcp/src/tools/get_manifest.rs`
- [ ] Implement handler: look up package in `PackageStore`, call `db.get_meta("manifest")`
- [ ] Return manifest string directly as the tool result content
- [ ] Register with `rmcp`

### 7.6 Stdio transport

- [ ] Create `crates/lore-mcp/src/lib.rs` exporting `run_stdio` and `run_http`
- [ ] Implement `run_stdio(packages_dir: &Path, cache_dir: &Path) -> Result<()>`:
      - Construct `LoreServer`
      - Run rmcp stdio server

### 7.7 HTTP transport

- [ ] Implement `run_http(packages_dir: &Path, cache_dir: &Path, host: &str, port: u16) -> Result<()>`:
      - Construct `LoreServer` wrapped in `Arc<RwLock<LoreServer>>`
      - Run rmcp HTTP server bound to `host:port`
      - Print `"Lore MCP server listening on {host}:{port}"` to stderr

---

## Phase 8: CLI

### 8.1 Setup

- [ ] Add to `crates/lore-cli/Cargo.toml`: `clap = { workspace = true, features = ["derive"] }`,
      `indicatif = { workspace = true }`, `dialoguer = { workspace = true }`,
      `console = { workspace = true }`, `tokio = { workspace = true, features = ["full"] }`,
      `serde_json = { workspace = true }`, all workspace crates as path deps
- [ ] Create `crates/lore-cli/src/main.rs` with `#[tokio::main] async fn main()`
- [ ] Define `Cli` struct with `#[derive(Parser)]` and `#[command(name = "lore", version)]`
- [ ] Define `Commands` enum with variants for all subcommands

### 8.2 Config resolution helper

- [ ] Create `crates/lore-cli/src/config.rs`
- [ ] Implement `resolve_dirs() -> (PathBuf, PathBuf)` returning (packages_dir, cache_dir):
      - packages_dir: `$LORE_PACKAGES_DIR` env var, else `~/.lore/packages`
      - cache_dir: `$LORE_CACHE_DIR` env var, else `~/.cache/lore`
      - Create both directories if they don't exist

### 8.3 `lore install`

- [ ] Create `crates/lore-cli/src/commands/install.rs`
- [ ] Parse `lib_at_version: String` argument
- [ ] Split on last `@`: if no `@` → name only with no version
- [ ] If no version: call `RegistryClient::search()`, display `dialoguer::Select` of versions
- [ ] Show `indicatif::ProgressBar` during download (bytes style)
- [ ] On success: print `"✓ Installed {name}@{version} ({chunk_count} chunks, {size})"`
- [ ] On error: print descriptive error and exit with code 1

### 8.4 `lore remove`

- [ ] Create `crates/lore-cli/src/commands/remove.rs`
- [ ] Confirm with `dialoguer::Confirm` unless `--force` flag passed
- [ ] Find and delete `.db` file matching the package name
- [ ] Print `"✓ Removed {name}@{version}"`

### 8.5 `lore list`

- [ ] Create `crates/lore-cli/src/commands/list.rs`
- [ ] Scan packages dir, open each DB, read meta
- [ ] Print formatted table: Name | Version | Chunks | Size | Build Date

### 8.6 `lore get`

- [ ] Create `crates/lore-cli/src/commands/get.rs`
- [ ] Accept positional args: `lib: String`, `topic: String`
- [ ] Accept flags: `--budget u32`, `--threshold f64`, `--lambda f64`, `--json`
- [ ] Run search, print results to stdout
- [ ] Each result: print `\n--- {heading_path} ---\n{content}\n`
- [ ] With `--json`: print `serde_json::to_string_pretty(&results)`

### 8.7 `lore manifest`

- [ ] Create `crates/lore-cli/src/commands/manifest.rs`
- [ ] Accept positional arg: `lib: String`
- [ ] Accept flag: `--copy` (copy to clipboard via `pbcopy` on macOS, `xclip`/`xsel` on Linux)
- [ ] Print manifest string to stdout
- [ ] With `--copy`: pipe to clipboard tool and print `"✓ Copied to clipboard"`

### 8.8 `lore search`

- [ ] Create `crates/lore-cli/src/commands/search.rs`
- [ ] Accept positional args: `registry: String`, `name: String`
- [ ] Call `RegistryClient::search()`, print results as table

### 8.9 `lore build`

- [ ] Create `crates/lore-cli/src/commands/build.rs`
- [ ] Detect source type: if path exists on disk → `LocalSource`, if starts with `http`/`https`
      and ends with `.git` or matches github.com pattern → `GitSource`, if URL → `WebsiteSource`,
      if ends with `.db` → passthrough copy
- [ ] Show spinner during build with live chunk count
- [ ] On completion: print `BuildStats::display()`

### 8.10 `lore serve`

- [ ] Create `crates/lore-cli/src/commands/serve.rs`
- [ ] Accept flags: `--http` (bool), `--port u16` (default 3000), `--host String` (default "127.0.0.1")
- [ ] Print `"Starting Lore MCP server..."` to stderr
- [ ] Call `lore_mcp::run_stdio()` or `lore_mcp::run_http()` based on flags

### 8.11 `lore info`

- [ ] Create `crates/lore-cli/src/commands/info.rs`
- [ ] Accept positional arg: `lib: String`
- [ ] Open DB, read all meta keys, count nodes by kind
- [ ] Print: Name, Version, Registry, Description, Source URL, Git SHA, Chunks, Code Blocks,
      Total Tokens, File Size, Build Date

---

## Phase 9: Registry Client

### 9.1 HTTP client

- [ ] Add to `crates/lore-registry/Cargo.toml`: `reqwest = { workspace = true, features = ["json", "stream"] }`,
      `lore-core` (path dep), `serde`, `serde_json`, `tokio`
- [ ] Create `crates/lore-registry/src/client.rs`
- [ ] Implement `RegistryClient` with `base_url: String` and `auth_token: Option<String>`
- [ ] Implement `RegistryClient::default()` with Neuledge API base URL
- [ ] Implement `RegistryClient::search(&self, registry, name, version) -> Result<Vec<PackageMetadata>>`
- [ ] Implement `RegistryClient::get_package(&self, registry, name, version) -> Result<PackageMetadata>`

### 9.2 Download pipeline

- [ ] Create `crates/lore-registry/src/download.rs`
- [ ] Implement `safe_filename(registry: &str, name: &str, version: &str) -> String`
- [ ] Implement `download_package(client, registry, name, version, dest_dir, progress_cb) -> Result<PathBuf>`
      following the temp-file-then-move pattern with schema validation

### 9.3 Config

- [ ] Create `crates/lore-registry/src/config.rs`
- [ ] Implement `LoreConfig` struct and `load()`/`save()` methods
- [ ] Implement `LoreConfig::packages_dir() -> PathBuf` returning expanded path
- [ ] Implement `LoreConfig::cache_dir() -> PathBuf` returning expanded path

---

## Phase 10: Custom Build Sources

### 10.1 Local directory source

- [ ] Create `crates/lore-build/src/sources/local.rs`
- [ ] `LocalSource::build(dir, meta, output)` → delegates to `PackageBuilder`

### 10.2 Git repository source

- [ ] Add `git2 = { workspace = true }` to `crates/lore-build/Cargo.toml`
- [ ] Create `crates/lore-build/src/sources/git.rs`
- [ ] Implement `GitSource::build(url, tag, docs_path, meta, output)`:
      - Create temp dir with `tempfile::tempdir()`
      - Add `tempfile = { workspace = true }` to Cargo.toml
      - Shallow clone via `git2::Repository::clone_recurse` with depth option, or via
        `std::process::Command` calling `git clone --depth 1 --branch {tag} {url} {tmp}`
        (prefer git2 if depth is supported, fall back to Command)
      - Navigate to `docs_path` subdirectory
      - Call `LocalSource::build(docs_subpath, meta, output)`

### 10.3 Website source

- [ ] Add `reqwest = { workspace = true }` to `crates/lore-build/Cargo.toml` if not present
- [ ] Create `crates/lore-build/src/sources/website.rs`
- [ ] Implement `WebsiteSource::build(url, meta, output)`:
      - Fetch `{url}/llms.txt` — parse as newline-separated list of URLs if 200
      - If 404: fetch root URL, extract all same-domain `<a href>` links using scraper
      - Create temp dir, for each URL:
        - Fetch HTML
        - Compute MD5 of content, skip if already seen (deduplication)
        - Write to temp dir as `.html` file named by URL hash
      - Call `LocalSource::build(temp_dir, meta, output)`
      - Concurrent fetching with `tokio::sync::Semaphore` (capacity 5)

---

## Phase 11: Manifest Generation

### 11.1 Heading extractor

- [ ] Create `crates/lore-build/src/manifest.rs`
- [ ] Implement `extract_headings(db: &Db) -> Result<Vec<HeadingEntry>>`:
      - Query all nodes where `kind = 'heading'` ordered by `path`
      - Return `HeadingEntry { path: Vec<String>, level: u8, title: String }`

### 11.2 API signature extractor

- [ ] Implement `extract_signatures(db: &Db) -> Result<Vec<ApiSignature>>`:
      - Query all nodes where `kind = 'code_block'`
      - For each code block with `lang` in the supported list:
        - Split content on newlines
        - Test each line against signature patterns:
          - `^(export )?(function|async function) \w+\(`
          - `^(export )?(const|let) \w+ =`
          - `^(export )?class \w+`
          - `^def \w+\(`
          - `^(pub )?(async )?fn \w+\(`
          - `^(export )?(type|interface) \w+`
        - Collect matching lines as signatures (first line only per definition)
      - Fetch heading path for each code block's parent

### 11.3 Manifest formatter

- [ ] Implement `build_manifest(headings: Vec<HeadingEntry>, signatures: Vec<ApiSignature>) -> String`:
      - Group signatures by their top-level heading section (first element of heading_path)
      - For each top-level section, format: `{SECTION}: {sig1}, {sig2}, ...`
      - Section names are uppercased
      - If estimated token count > 500: drop all headings with no signatures first, then
        trim signature lists (keep first N sigs per section until under budget)
      - Use `TokenCounter` to measure
- [ ] Integrate `build_manifest` call at end of `PackageBuilder::build()`
- [ ] Write test `test_manifest_under_500_tokens`: build test package, assert manifest token
      count ≤ 500
- [ ] Write test `test_manifest_contains_api_names`: assert known function names from test
      docs appear in the manifest output

---

## Phase 12: Registry Infrastructure

### 12.1 Package YAML definitions

- [ ] Create `registry/` directory at workspace root
- [ ] Create `registry/npm/` subdirectory
- [ ] Create `registry/npm/next.yaml` with versions covering 13.x and 14.x and 15.x
- [ ] Create `registry/npm/react.yaml`
- [ ] Create `registry/npm/vue.yaml`
- [ ] Create `registry/npm/astro.yaml`
- [ ] Create `registry/npm/remix.yaml`
- [ ] Create `registry/npm/hono.yaml`
- [ ] Create `registry/npm/drizzle-orm.yaml`
- [ ] Create `registry/npm/zod.yaml`
- [ ] Create `registry/pypi/` subdirectory
- [ ] Create `registry/pypi/fastapi.yaml`
- [ ] Create `registry/pypi/django.yaml`
- [ ] Create `registry/pypi/sqlalchemy.yaml`
- [ ] Create `registry/cargo/` subdirectory
- [ ] Create `registry/cargo/tokio.yaml`
- [ ] Create `registry/cargo/axum.yaml`
- [ ] Create `registry/cargo/serde.yaml`

### 12.2 Version discovery

- [ ] Create `crates/lore-registry/src/discover.rs`
- [ ] Implement `discover_npm_versions(package: &str) -> Result<Vec<String>>`
- [ ] Implement `discover_pypi_versions(package: &str) -> Result<Vec<String>>`
- [ ] Implement `discover_crates_versions(package: &str) -> Result<Vec<String>>`
- [ ] Implement `filter_versions(versions: Vec<String>, min: Option<&str>, max: Option<&str>) -> Vec<String>`:
      - Parse as semver, filter by range, remove pre-releases, keep latest patch per minor

### 12.3 CI workflow

- [ ] Create `.github/workflows/registry.yml`
- [ ] Trigger: `paths: ['registry/**']` on push to main
- [ ] Steps: checkout, install Rust toolchain, build lore binary, for each changed YAML:
      discover versions → check published → build missing → publish with auth token

---

## Phase 13: Testing

### 13.1 Ensure all unit tests pass

- [ ] Run `cargo test -p lore-core` — all pass
- [ ] Run `cargo test -p lore-build` — all pass
- [ ] Run `cargo test -p lore-search` — all pass
- [ ] Run `cargo test -p lore-registry` — all pass
- [ ] Run `cargo test -p lore-mcp` — all pass
- [ ] Run `cargo test -p lore-cli` — all pass
- [ ] Run `cargo test --workspace` — all pass

### 13.2 Build and search integration

- [ ] Create `tests/integration/` directory at workspace root
- [ ] Create `tests/integration/build_search_test.rs`
- [ ] Write end-to-end test: build a package from a real documentation directory (e.g.
      a small subset of Next.js docs checked into `tests/fixtures/`), run 10 queries,
      assert all top results are correct

### 13.3 MCP round-trip test

- [ ] Create `tests/integration/mcp_test.rs`
- [ ] Start `LoreServer` with a test package installed
- [ ] Call `handle_get_docs` directly (not via transport) with a known query
- [ ] Assert response contains expected content

### 13.4 Retrieval quality benchmarks

- [ ] Create `benches/retrieval.rs` using Rust's built-in benchmark framework or `criterion`
- [ ] Add `criterion = { workspace = true }` to workspace dependencies and bench crate
- [ ] Define 20 query/expected-answer pairs
- [ ] Implement `recall_at_k(results, expected, k)` helper
- [ ] Run benchmarks, record baseline recall@5 and MRR
- [ ] Fail CI if recall@5 drops below 0.75

---

## Phase 14: Distribution

### 14.1 Release build configuration

- [ ] Create `.cargo/config.toml` with:
      `[profile.release]` `opt-level = 3`, `lto = true`, `codegen-units = 1`, `strip = true`

### 14.2 GitHub Actions release workflow

- [ ] Create `.github/workflows/release.yml`
- [ ] Trigger on: `push` to tags matching `v*`
- [ ] Matrix: `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`,
      `x86_64-pc-windows-msvc`
- [ ] Install `cross` for Linux target
- [ ] Run `cargo build --release --target {target}` (or `cross build` for Linux)
- [ ] Upload binaries as release assets named `lore-{target}.tar.gz`

### 14.3 Install script

- [ ] Create `install.sh` at workspace root
- [ ] Detect platform and architecture via `uname`
- [ ] Download correct binary from latest GitHub release
- [ ] Place in `/usr/local/bin/lore`
- [ ] Print `"lore installed successfully. Run 'lore --version' to verify."`

### 14.4 Verify final binary

- [ ] Run `cargo build --release -p lore-cli`
- [ ] Assert `./target/release/lore --version` prints the version
- [ ] Assert `./target/release/lore --help` lists all subcommands
- [ ] Run `./target/release/lore serve` and verify it starts without error
