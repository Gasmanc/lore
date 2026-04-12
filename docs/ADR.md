# Lore — Architecture Decision Records

---

## ADR-001: SQLite as the storage engine

**Status:** Accepted

**Context:**
We need a storage engine for documentation indexes. Options considered: SQLite, DuckDB,
RocksDB, and a custom binary format.

**Decision:**
Use SQLite with the `rusqlite` crate (bundled feature).

**Rationale:**
- The core value proposition of lore is a single portable `.db` file that can be shared
  between teammates. SQLite delivers this natively.
- FTS5 full-text search with BM25 ranking and Porter stemming is built into SQLite.
  No separate search engine required.
- `sqlite-vec` extends SQLite with vector search, keeping everything in one file.
- Works fully offline after initial download.
- Zero administration — no server process, no configuration.
- DuckDB was considered but lacks FTS5 and produces multi-file databases, breaking
  the portability guarantee.
- RocksDB is a key-value store with no query language — would require building
  search from scratch.

**Consequences:**
- All search (BM25, vector, metadata) runs in a single file.
- Must use `tokio-rusqlite` to avoid blocking the async runtime (see ADR-003).
- `sqlite-vec` extension must be loaded on every connection open.

---

## ADR-002: Path enumeration for hierarchical schema

**Status:** Accepted

**Context:**
Documentation has a natural tree structure (document → headings → chunks). The original
neuledge/context schema is flat (no parent references). We need to support:
- Fetching all descendants of a heading (for context expansion)
- Fetching the full ancestor path (for breadcrumb generation)
- Small-to-big retrieval (match chunk, return parent section)

Options considered: adjacency list with recursive CTEs, nested sets, closure table,
path enumeration.

**Decision:**
Use path enumeration: each node stores a `path` column containing its full ancestry as
a slash-separated string of IDs (e.g., `"1/4/9/23"`).

**Rationale:**
- The database is write-once at build time. Path enumeration is efficient for read-heavy
  workloads where the tree never changes after creation.
- Ancestor queries: `WHERE '1/4/9/23' LIKE path || '%'` — simple, fast with an index.
- Descendant queries: `WHERE path LIKE '1/4/%'` — prefix scan, fast with an index.
- No recursive CTEs needed at query time.
- Simpler to implement than closure tables (which require a separate table of all
  ancestor-descendant pairs).
- Nested sets are complex to build and offer no advantage for a static tree.

**Consequences:**
- Path strings must be constructed correctly during the build pipeline.
- Path column must be indexed for efficient prefix scans.
- Path strings grow with tree depth but documentation trees are shallow (max depth ~6).

---

## ADR-003: tokio-rusqlite over spawn_blocking

**Status:** Accepted

**Context:**
The MCP server and HTTP client require async (tokio). SQLite via `rusqlite` is
synchronous. We need a strategy for calling SQLite from async code without blocking
the async worker threads.

Options considered: `tokio::task::spawn_blocking`, `tokio-rusqlite`, `async-sqlite`.

**Decision:**
Use `tokio-rusqlite`.

**Rationale:**
- `spawn_blocking` has a connection ownership problem: `rusqlite::Connection` is `Send`
  but not `Clone`. Sharing it across `spawn_blocking` calls requires `Arc<Mutex<Connection>>`,
  which holds a mutex across await points — a deadlock risk and a serialisation bottleneck.
- `tokio-rusqlite` solves this by keeping the connection on a single dedicated thread.
  All operations are sent as closures via a channel: `conn.call(|db| { ... }).await`.
- The API is uniform — all DB code looks the same regardless of which async function calls it.
- Channel round-trip overhead (~microseconds) is negligible compared to DB operation
  latency (~milliseconds).
- `async-sqlite` is similar but `tokio-rusqlite` has better maintenance and ecosystem fit.

**Consequences:**
- All database operations use the `conn.call(|db| { ... }).await` pattern.
- `sqlite_vec::load()` must be called inside a `conn.call` closure immediately after
  the connection is opened.
- One `tokio_rusqlite::Connection` per open `.db` file.

---

## ADR-004: sqlite-vec for vector storage

**Status:** Accepted

**Context:**
Hybrid search requires storing and querying embedding vectors. Options considered:
`sqlite-vec`, `sqlite-vss` (deprecated), a separate vector database (Qdrant, Weaviate),
an in-memory HNSW index (hnswlib), or storing raw BLOBs and computing distance in Rust.

**Decision:**
Use `sqlite-vec` (the Rust crate that bundles and loads the C extension).

**Rationale:**
- Keeps vectors in the same `.db` file as the text — portability is preserved.
- The Rust crate compiles the C extension via `build.rs`, requiring only a C compiler.
  No system library installation needed.
- `sqlite-vss` (the predecessor using Faiss) is deprecated by its author in favour of
  `sqlite-vec`.
- A separate vector database breaks the single-file guarantee and requires a running
  process.
- In-memory HNSW would not persist to disk with the package.
- Raw BLOB + Rust distance computation would require loading all vectors into RAM for
  every query — not viable for large packages.
- `vec0` uses exact KNN, which is fast enough for packages under 50,000 chunks
  (single-digit milliseconds on CPU).

**Consequences:**
- `sqlite_vec::load(conn)` must be called on every connection.
- Vector dimension is fixed at table creation time (384 for `bge-small-en-v1.5`).
- Changing the embedding model requires rebuilding the package.
- Approximate search not available in `vec0` — exact KNN only. Acceptable for
  expected package sizes.

---

## ADR-005: bge-small-en-v1.5 as the embedding model

**Status:** Accepted

**Context:**
We need a local embedding model for: (a) semantic chunking refinement at build time,
(b) chunk embedding at build time, (c) query embedding at search time.
Options considered: `all-MiniLM-L6-v2` (90MB, 384d), `bge-small-en-v1.5` (130MB, 384d),
`nomic-embed-text-v1.5` (274MB, 768d), `bge-base-en-v1.5` (430MB, 768d).

**Decision:**
Use `bge-small-en-v1.5` via `fastembed-rs`.

**Rationale:**
- Better performance than `all-MiniLM-L6-v2` on technical/code-adjacent text while
  remaining small (130MB) and fast on CPU.
- 384 dimensions is a good balance: meaningful semantic representation without excessive
  storage cost (1,536 bytes per chunk as raw f32).
- `nomic-embed-text-v1.5` is better but at 274MB and 768 dimensions, the storage and
  compute cost is doubled for marginal gains in this use case.
- `fastembed-rs` handles model download, caching, quantisation, and batching.
  Model is downloaded to `~/.cache/lore/models/` on first use.
- Same model is used for both build-time (chunking + indexing) and query-time
  embedding, ensuring consistency.

**Consequences:**
- First `lore install` triggers a one-time ~130MB model download.
- Model is shared across all packages — downloaded once.
- Output dimension (384) is baked into the `node_embeddings` virtual table schema.
- Changing the model requires rebuilding all packages.

---

## ADR-006: Structural-first chunking with semantic refinement

**Status:** Accepted

**Context:**
Documentation must be split into chunks for indexing. Options: fixed-size token windows,
pure semantic chunking (LangChain SemanticChunker style), heading-boundary chunking
(neuledge/context style), or a hybrid.

**Decision:**
Structural-first with semantic refinement on large sections only.

**Rationale:**
- Heading boundaries are almost always correct semantic boundaries — they represent
  the author's intentional topic divisions.
- Pure semantic chunking ignores these boundaries and may split where headings
  already provide a better signal.
- Fixed-size windows are vocabulary-agnostic and produce poor results for technical docs.
- Semantic refinement is only applied to sections over 800 tokens, which is typically
  a small fraction of sections. This limits the compute cost while handling edge cases
  (very long sections covering multiple subtopics).
- Code blocks are a hard constraint — they are always atomic regardless of any other
  rule. A split code block is worse than an oversized chunk.
- Detecting the primary heading level per document (rather than assuming H2) handles
  API reference docs where H3 is the true primary unit.

**Consequences:**
- The chunker must detect the primary heading level before processing each document.
- Semantic refinement requires the embedding model to be available at build time.
- Code blocks may produce chunks that exceed the nominal token limit — this is accepted.
- Section titles from parent headings must be included in the chunk's breadcrumb.

---

## ADR-007: Contextual embeddings (breadcrumb prepended before embedding)

**Status:** Accepted

**Context:**
A chunk with only its own content loses positional meaning. "cacheLife() accepts a profile
name" without context could be from any library. Embedding with context improves retrieval.

**Decision:**
Before embedding any chunk, prepend its full breadcrumb path:
`"{doc_title} > {h1} > {h2} > {section_title}\n\n{chunk_content}"`

**Rationale:**
- Anthropic's contextual retrieval research showed prepending document context to chunks
  before embedding improved retrieval quality by ~35% on tested benchmarks.
- The cost is negligible: the breadcrumb is computed from the node tree already built,
  and the embedding call is only slightly longer.
- Query embeddings do NOT get a breadcrumb — they are questions, not document sections.
  The asymmetry is intentional and correct.
- This requires no changes to the schema or search pipeline — it is purely a build-time
  transformation of the text before the embedding call.

**Consequences:**
- The `Embedder` must have access to the full ancestor chain for each node.
- Stored embeddings represent "breadcrumb + content", not just "content".
- Query embeddings and chunk embeddings are in slightly different semantic spaces, which
  is the standard asymmetric retrieval setup used by `bge` models.

---

## ADR-008: Reciprocal Rank Fusion for hybrid search

**Status:** Accepted

**Context:**
Hybrid search produces two ranked lists (BM25 and vector). These must be merged into a
single ranked list. Options: weighted linear combination of scores, CombMNZ, Reciprocal
Rank Fusion (RRF).

**Decision:**
Use Reciprocal Rank Fusion with k=60.

**Rationale:**
- BM25 scores and vector cosine scores are on incompatible scales. Weighted linear
  combination requires tuning the weights, which is dataset-specific and fragile.
- RRF uses only the rank position, not the raw score: `score(d) = Σ 1/(k + rank_i)`.
  No calibration needed.
- k=60 is the standard value from the original RRF paper and empirically robust.
- RRF consistently outperforms weighted combination in retrieval benchmarks when the
  two score distributions are not normalised.
- Implementation is trivial: two maps of (node_id → rank), merge, sort by RRF score.

**Consequences:**
- The absolute BM25 and vector scores are discarded after ranking — only position matters.
- k=60 is a constant but can be made configurable if needed.
- RRF naturally handles the case where a document appears in only one list
  (it gets a single term in the sum).

---

## ADR-009: Maximal Marginal Relevance for result diversity

**Status:** Accepted

**Context:**
After RRF merge, the top results may be highly redundant — three chunks covering the same
concept from the same section. This wastes the token budget.

**Decision:**
Apply MMR after RRF: `score(d) = λ·relevance(d,q) − (1−λ)·max_sim(d, selected)` with λ=0.7.

**Rationale:**
- MMR explicitly trades off relevance against redundancy. λ=0.7 gives 70% weight to
  relevance and 30% to diversity — a conservative setting that removes obvious duplicates
  without sacrificing quality.
- The similarity computation reuses the already-computed embeddings — no additional
  model calls.
- Without MMR, the token budget can be consumed by near-identical chunks, leaving no
  room for genuinely different relevant sections.
- λ is exposed as a configurable parameter in `SearchConfig` for users who want more
  or less diversity.

**Consequences:**
- MMR requires chunk embeddings to be loaded into memory for the deduplication pass.
  For typical result sets (≤20 candidates) this is negligible.
- MMR runs after RRF — it refines, not replaces, the hybrid ranking.

---

## ADR-010: rmcp for the MCP server

**Status:** Accepted

**Context:**
We need to implement an MCP server in Rust. Options: `rmcp` (official Rust MCP SDK from
modelcontextprotocol), building directly on `jsonrpc-core`, or implementing the protocol
manually.

**Decision:**
Use `rmcp`.

**Rationale:**
- `rmcp` is the official Rust SDK maintained by the MCP organisation — protocol compliance
  is guaranteed.
- Handles both stdio and HTTP transports via a single abstraction.
- Tool definitions use Rust types with derive macros — less boilerplate than manual JSON.
- Implementing MCP over raw jsonrpc is ~500 lines of protocol handling we don't need to own.

**Consequences:**
- We depend on `rmcp`'s release cadence for MCP protocol updates.
- Tool parameter types must be expressible as `rmcp` schema types.
- The dynamic enum for the `library` parameter (built from installed packages) requires
  using `rmcp`'s runtime schema generation APIs.

---

## ADR-011: Cargo workspace with six crates

**Status:** Accepted

**Context:**
The project could be a single crate or split into multiple crates. Splitting increases
initial complexity but provides compile-time isolation and reusability.

**Decision:**
Cargo workspace with six crates: `lore-core`, `lore-build`, `lore-search`, `lore-mcp`,
`lore-registry`, `lore-cli`.

**Rationale:**
- `lore-build` and `lore-search` are completely independent — a change to chunking logic
  does not require recompiling the MCP server.
- `lore-core` types are shared but isolated — schema changes are caught at compile time
  across all consumers.
- The registry infrastructure (`lore-registry`) can be used as a library by CI tooling
  independently of the CLI.
- Incremental compilation is significantly faster in a workspace when changes are
  localised to one crate.

**Consequences:**
- Six `Cargo.toml` files to maintain.
- Dependency versions should be unified via workspace-level `[workspace.dependencies]`.
- The `lore-cli` crate is thin — it wires together the other crates with minimal logic.

---

## ADR-012: tiktoken-rs for token counting

**Status:** Accepted

**Context:**
Token counts are used to: enforce chunk size limits during chunking, enforce the token
budget in search results, and generate the manifest within the 500-token target.
Options: `content.len() / 4` (current neuledge/context approach), `tiktoken-rs`,
`tokenizers` (HuggingFace).

**Decision:**
Use `tiktoken-rs` with the `cl100k_base` tokenizer.

**Rationale:**
- `length / 4` is a rough heuristic. It is consistently wrong for code (which tokenises
  differently from prose) and for non-ASCII content.
- Accurate token counts matter for the chunk size limits and the 500-token manifest target.
- `cl100k_base` is the tokenizer used by modern Claude and GPT models — counts will be
  accurate for the models that consume lore's output.
- `tiktoken-rs` is a Rust port of OpenAI's tiktoken — fast, no Python dependency.
- `tokenizers` (HuggingFace) is heavier and designed for training workflows.

**Consequences:**
- `tiktoken-rs` adds a build dependency and increases compile time slightly.
- Token counts are model-specific. If a user's model uses a different tokenizer, counts
  may be slightly off — acceptable for our use case.
- Token counting is called frequently during the build pipeline — `tiktoken-rs` must be
  initialised once and reused (the `Tokenizer` struct is cheap to call but expensive to
  construct).
