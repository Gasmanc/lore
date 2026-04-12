# Lore — Product Requirements Document

## Problem Statement

AI coding assistants (Claude Code, Cursor, Windsurf, Cline, GitHub Copilot) are trained on
documentation that becomes outdated as libraries evolve. When a developer asks about an API
introduced or changed after the model's training cutoff, the assistant confidently generates
wrong code. This is not a model intelligence problem — it is an information availability problem.

Existing solutions have significant limitations:

**Cloud services (Context7, Ref.Tools):**
- Rate limits and free tier restrictions interrupt workflows mid-session
- All queries and code context are sent to third-party servers
- Require internet connectivity
- Subscription costs for meaningful usage

**neuledge/context (closest prior art):**
- Chunking always splits at H2 headings regardless of document structure
- May split code examples mid-block
- BM25-only search fails on vocabulary mismatch
- No way to generate a compact index for CLAUDE.md to prime the agent
- All search parameters hardcoded with no user configurability
- Written in TypeScript/Node.js — higher memory footprint, slower startup

**Embedding docs in CLAUDE.md directly:**
- Vercel's evals show 100% pass rate vs 79% for tool-based retrieval
- But 8KB+ of docs per library embedded in every prompt wastes tokens and
  competes with actual conversation context
- Impractical for more than one or two libraries

---

## Solution

**Lore** is a local-first, offline documentation server for AI coding assistants. It:

1. Indexes library documentation into portable, self-contained SQLite databases
2. Serves them via MCP (Model Context Protocol) with hybrid BM25 + vector search
3. Generates compressed ~300 token manifests suitable for embedding in CLAUDE.md,
   giving the agent a reliable fingerpost to the full documentation without token waste

The result: the agent knows *what it doesn't know* (from the manifest in CLAUDE.md), and
retrieves full documentation on demand with near-perfect reliability.

---

## Target Users

**Primary:** Individual developers using AI coding assistants (Claude Code, Cursor, Windsurf,
Cline) who work with libraries that have APIs not well represented in model training data.
This includes: framework-specific features (Next.js app router, Astro, SvelteKit), recent
major versions, private/internal APIs, and niche libraries.

**Secondary:** Development teams who want to share a pre-built documentation index across
members so each developer does not need to rebuild it from source.

---

## Goals

1. Provide accurate, version-specific documentation to AI coding assistants with no internet
   dependency after initial package download
2. Achieve higher retrieval accuracy than BM25-only search via hybrid BM25 + vector search
3. Never return a split code example — code blocks are always returned intact
4. Respect document structure by detecting the primary heading level per document rather
   than assuming H2
5. Generate a compressed manifest for any installed package suitable for CLAUDE.md embedding
6. Run entirely on the developer's machine with no data leaving the local environment
7. Be fast enough that queries complete in under 100ms for typical packages
8. Produce a single self-contained `.db` file per package that can be shared with teammates

---

## Non-Goals

- Cloud hosting or multi-user access
- Real-time documentation updates (packages are snapshots at a version)
- Authentication or access control
- Web UI (CLI and MCP only)
- Support for non-English documentation (English only for initial release)
- Replacing general web search — lore is for library-specific documentation only

---

## Feature Requirements

### F-01: Package Installation

The user can install a pre-built documentation package from the community registry with a
single command. The package is downloaded as a `.db` file and validated before installation.
A failed or interrupted download does not leave a corrupt database.

Acceptance criteria:
- `lore install next@15` downloads, validates, and installs the package
- Progress is shown during download
- If download fails mid-stream, no corrupt file is written to the packages directory
- Installed package is immediately available in the MCP server without restart

### F-02: Hybrid Documentation Search

The MCP `get_docs` tool searches installed packages using a combination of BM25 full-text
search and vector similarity search, fused with Reciprocal Rank Fusion. Results are
deduplicated with Maximal Marginal Relevance and capped at a configurable token budget.

Acceptance criteria:
- Queries that have no term overlap with the answer (vocabulary mismatch) are still
  retrieved correctly via vector search
- Code blocks in results are always returned intact (never truncated mid-block)
- Results include the full breadcrumb path to each section
- Token budget, relevance threshold, and MMR lambda are configurable per-query
- Search completes in under 100ms for packages with up to 10,000 chunks

### F-03: Manifest Generation

`lore manifest <lib>` generates a compressed index of the package's contents: all heading
paths, API signatures extracted from code blocks, and key parameter names. The output is
under 500 tokens and is designed to be pasted directly into CLAUDE.md.

Acceptance criteria:
- Output is under 500 tokens for any standard library package
- Output includes all top-level API names (functions, classes, directives)
- Output is structured for human readability as well as agent consumption
- Output can be generated offline from an installed package with no network calls

### F-04: Custom Package Building

The user can build a documentation package from any supported source: local directory, git
repository URL, website with llms.txt, or pre-existing `.db` file.

Acceptance criteria:
- `lore build ./my-docs --name mylib --version 1.0.0` produces a valid `.db` file
- `lore build https://github.com/org/repo --name lib --version 2.0.0` shallow-clones
  the repository and indexes its documentation
- Code blocks are always kept intact regardless of size
- The primary heading level is detected per-document, not assumed to be H2

### F-05: MCP Server

Lore exposes an MCP server providing four tools: `get_docs`, `search_packages`,
`download_package`, and `get_manifest`. The server runs via stdio by default and optionally
via HTTP.

Acceptance criteria:
- MCP server is compatible with Claude Code, Cursor, Windsurf, and Cline
- `get_docs` library parameter is a closed enum of installed packages (no guessing)
- Installing a new package updates the enum without requiring a server restart
- HTTP transport supports multiple simultaneous clients

### F-06: Community Registry

A community-maintained registry of pre-built packages covering popular libraries. Package
definitions are YAML files in the `registry/` directory. Packages are built and published
via CI.

Acceptance criteria:
- Registry covers the same libraries as neuledge/context at a minimum
- New packages can be added by submitting a YAML definition
- Multiple versions of each package are available
- Registry search returns results sorted by version descending

### F-07: CLI Interface

All operations are available via a well-documented CLI with progress indicators, interactive
prompts where appropriate, and clear error messages.

Acceptance criteria:
- All subcommands documented via `--help`
- Progress bars shown for download and build operations
- Interactive version selection when version is not specified
- Error messages identify the cause and suggest corrective action

---

## Success Metrics

- Retrieval accuracy on vocabulary-mismatch queries: >90% (vs ~50% for BM25-only)
- Zero code blocks split across chunk boundaries
- Manifest output under 500 tokens for all standard packages
- Query latency under 100ms for packages under 10,000 chunks
- Binary size under 20MB (excluding the embedding model)
- First install (including model download) under 5 minutes on a standard connection
- Subsequent package installs under 30 seconds for typical packages (~10MB)
