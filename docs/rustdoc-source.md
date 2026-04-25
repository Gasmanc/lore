# Plan: rustdoc JSON source + parser

**Status:** draft — needs fleshing out before implementation.

## Motivation

Many widely-used Rust crates (`notify`, `gix`, `serde_yml`, plus the long tail
of crates that don't maintain a separate prose docs site) keep all their
user-facing documentation as `///` and `//!` doc comments inside the `.rs`
sources. Lore's current pipeline ingests markdown files from a git repo, so
indexing those crates today means indexing only the README.

Adding rustdoc as a first-class source unlocks **every crate on crates.io**
with no per-package docs-repo hunting. That is a structural improvement, not
a one-off fix for three crates.

## High-level design

Two new pieces, mirroring the existing parser/source split:

1. **`crates/lore-build/src/source/rustdoc.rs`** — fetches a crate and runs
   nightly rustdoc against it to produce a JSON dump.
2. **`crates/lore-build/src/parser/rustdoc.rs`** — walks the JSON tree and
   emits markdown-shaped `Document`s for the existing chunker/embedder.

A new YAML spec form:

```yaml
name: notify
registry: cargo
version: "8.0.0"
description: "Cross-platform filesystem notification library"
source:
  type: rustdoc
  crate: notify
```

## Source: `rustdoc.rs`

### Inputs

- `crate` — name of the crate on crates.io
- `version` — pinned in the YAML; the source resolves the matching tarball
- (optional) `features` — feature flags to enable for the doc build

### Steps

1. Create a temp `cargo new --lib` scratch project.
2. Add the target crate as a path-less dependency at the requested version.
3. Run:
   ```
   cargo +nightly rustdoc -Zunstable-options \
     --output-format json \
     -p <crate>
   ```
4. Locate the generated `target/doc/<crate>.json`.
5. Return the JSON path as the "prepared" source for the parser.

### Open questions

- **Toolchain pinning.** rustdoc JSON is `format_version`-stamped but the
  schema still drifts between nightlies. Pin to a known-good nightly via
  `rust-toolchain.toml` in the scratch project, and document the upgrade
  process when bumping it.
- **Build failures.** Some crates fail to build under nightly (proc-macro
  ABI, sysroot quirks, native deps). Fall back to a clear error message
  rather than indexing nothing.
- **Feature selection.** Default features only? Or `--all-features`?
  All-features can pull in conflicting deps; default-only can omit important
  surface area. Likely: default features, with an opt-in `features:` list
  in the YAML.

## Parser: `rustdoc.rs`

### JSON shape (rustc-types crate)

The dump is roughly:

```jsonc
{
  "root": "0:0:0",
  "crate_version": "8.0.0",
  "format_version": 35,
  "index": {
    "0:0:0": {
      "id": "0:0:0",
      "name": "notify",
      "kind": "module",
      "docs": "Cross-platform filesystem notifications…",
      "inner": { "module": { "items": [...] } }
    },
    "0:0:1": { "kind": "struct", "docs": "…", "inner": { ... } },
    ...
  },
  "paths": { "0:0:0": { "path": ["notify"], "kind": "module" }, ... }
}
```

### Walk strategy

- Start at `root`, recurse through `module.items`.
- For each public item: emit a `Document` whose path is the dotted module
  path + item name (`notify::event::EventKind::Create`).
- Synthesise a markdown body per item:
  - `# <signature>` — rendered from the item kind (fn signature, struct
    fields, enum variants, trait methods, impl blocks).
  - The item's `docs` field, verbatim (already markdown).
  - For traits/structs/enums: a "## Methods" / "## Variants" subsection
    listing children with their own short doc snippets.
- Skip private items. Skip `#[doc(hidden)]` items.
- Skip impl blocks that re-document trait methods (they appear under the
  trait already) — but keep crate-defined inherent impls.

### Chunking

One chunk per item, grouped by module so MMR/RRF still surfaces neighbours
under the same module path. Reuse the existing `chunker::Chunker` with the
synthesised markdown.

## CI changes

`build-registry.yml` needs:
- `rustup toolchain install nightly --component rust-docs-json` before
  building any `type: rustdoc` spec.
- A new branch in the spec dispatcher's `case` statement for `rustdoc`.

## Validation

- Add `notify`, `gix`, `serde_yml` as rustdoc-sourced specs and confirm:
  - Build succeeds in CI.
  - `lore search` returns sensible results for queries like
    "watch a directory recursively" or "deserialize a yaml mapping".
- Re-run `lore-bench` to confirm overall retrieval quality is unchanged on
  the existing markdown-sourced corpus.

## Effort estimate

2–3 focused days end-to-end. The risk concentration is in the parser's
markdown synthesis — getting signatures + nested impls to render readably
will take iteration.

## Out of scope (for now)

- Cross-crate links (rustdoc emits item ids; resolving them across packages
  needs a separate index).
- Source-level snippets (rustdoc JSON does not include source code).
- Re-exports — handle the `import` kind to surface re-exported items where
  users would expect to find them, but defer fancy disambiguation.
