# Lore — Project Summary

## Origin

This project emerged from a conversation exploring the problem of AI coding assistants hallucinating
when working with library APIs not present in their training data. The research path covered:

1. **neuledge/context** — a local-first MCP server that serves pre-indexed SQLite documentation
   packages to AI agents. Community registry of 100+ libraries. Offline, private, free.
2. **Vercel's agents.md research** — an eval showing that embedding docs directly into AGENTS.md
   (100% pass rate) significantly outperforms skill/tool-based retrieval (79% with explicit
   instructions, 53% with default). The gap exists because the agent must decide to invoke a tool,
   and fails to do so ~56% of the time without explicit instructions.
3. **The hybrid insight** — a compressed ~300 token index in CLAUDE.md tells the agent *what it
   doesn't know*, so it reliably invokes the tool. Full docs stay in the local index, retrieved
   on demand. Best of both worlds: no token waste, near-100% retrieval reliability.

---

## Analysis of neuledge/context

A deep technical read of the source revealed the following design:

### What it does well
- Single portable `.db` file per package (SQLite + FTS5)
- Porter stemming for BM25 ranking
- Field weights (section_title: 10×, content: 1×)
- Temp-file-then-move pattern for safe downloads
- Streaming downloads, no RAM spike
- Dynamic enum of installed packages for MCP tool parameters

### Weaknesses identified

**Chunking:**
- Always splits at H2 regardless of document structure (wrong for API reference docs where H3
  is the primary topic unit)
- May split inside code blocks (arbitrary line-boundary fallback)
- Chunks store only `section_title` — no ancestry breadcrumb
- Token estimation is `length / 4` (imprecise)
- No awareness of doc type (tutorial vs reference vs concept)

**Search:**
- BM25 only — vocabulary mismatch is a hard failure ("timeout" never matches "deadline")
- 0.5 relevance threshold and 2000 token budget are hardcoded constants
- No deduplication — three chunks saying the same thing all consume token budget
- No small-to-big retrieval (index and retrieve at the same granularity)

**Missing feature:**
- No way to generate a compressed index for CLAUDE.md fingerposting

---

## Design Decisions Made

### Database
Keep SQLite. The portability of a single `.db` file is the core value proposition. Change the
*schema* to a hierarchical nodes table using path enumeration, enabling tree traversal without
changing the storage engine. Add `sqlite-vec` for vector storage alongside FTS5 — still one file.

### Chunking
Structural-first: detect the primary heading level per document (not always H2). Headings are
hard boundaries. For sections over 800 tokens, apply paragraph-level semantic similarity to detect
topic shifts and split further. Code blocks are always atomic — never split regardless of size.

Before embedding each chunk, prepend the full breadcrumb path. This "contextual embedding" bakes
document position into the vector, improving retrieval quality significantly with no query-time cost.

### Search
Hybrid BM25 + vector search fused with Reciprocal Rank Fusion (RRF). RRF requires no weight
tuning — it's robust by design. After merge, apply Maximal Marginal Relevance (MMR) to remove
redundant results. All parameters (threshold, token budget, λ) are configurable, not hardcoded.

### Async boundary
`tokio-rusqlite` over raw `spawn_blocking`. The connection lives on a dedicated thread; closures
are sent via channel. Avoids `Connection` ownership/mutex problems entirely.

### Embeddings
`bge-small-en-v1.5` via `fastembed-rs`. 384 dimensions, ~130MB, downloads to
`~/.cache/lore/models/` on first use. Fast on CPU, accurate on technical text.

### New feature: manifest
`lore manifest <lib>` generates a ~300 token compressed index of everything a package contains:
heading paths, API signatures extracted from code blocks, key parameter names. Intended to be
pasted directly into CLAUDE.md as a fingerpost.

---

## Naming

The project is named **lore** — "a body of knowledge." CLI reads naturally:
`lore install react@18`, `lore get react hooks`, `lore manifest next`.

---

## Technology

Rust. Cargo workspace with six crates:
`lore-core`, `lore-build`, `lore-search`, `lore-mcp`, `lore-registry`, `lore-cli`.
