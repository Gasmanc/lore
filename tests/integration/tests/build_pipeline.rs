//! Integration tests for the full build pipeline.
//!
//! Tests that require the bge-small-en-v1.5 embedding model (~130 MB) are
//! marked `#[ignore]` so they are skipped in CI by default.  Run them locally
//! with:
//!
//! ```sh
//! cargo test -p lore-integration-tests -- --ignored
//! ```
//!
//! The first run will download the model; subsequent runs use the local cache.

use std::path::Path;

use lore_build::{
    ChunkConfig, MarkdownParser, StructuralChunker, TokenCounter, discover_files, parser::Parser,
};
use lore_core::{Db, NewNode, NodeKind, Package};
use tempfile::NamedTempFile;

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// Path to the bundled markdown fixture directory.
///
/// `CARGO_MANIFEST_DIR` points at `tests/integration/`.
fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

// ── Parse + chunk (no model) ──────────────────────────────────────────────────

#[test]
fn parse_and_chunk_fixture_getting_started() {
    let md = std::fs::read_to_string(fixtures_dir().join("getting-started.md"))
        .expect("fixture file must exist");

    let doc =
        MarkdownParser.parse(&md, Path::new("getting-started.md")).expect("parse must succeed");

    let counter = TokenCounter::new().expect("tokeniser init");
    let chunker = StructuralChunker::new(ChunkConfig::default(), counter);
    let tree = chunker.chunk(&doc, "getting-started.md", 2);

    assert!(!tree.is_empty(), "fixture must produce at least one chunk");

    let code_chunks: Vec<_> =
        tree.nodes.iter().filter(|(c, _)| c.kind == NodeKind::CodeBlock).collect();
    assert!(!code_chunks.is_empty(), "fixture has code blocks");

    let prose_chunks: Vec<_> =
        tree.nodes.iter().filter(|(c, _)| c.kind == NodeKind::Chunk).collect();
    assert!(!prose_chunks.is_empty(), "fixture has prose sections");
}

#[test]
fn parse_and_chunk_fixture_api_reference() {
    let md = std::fs::read_to_string(fixtures_dir().join("api-reference.md"))
        .expect("fixture file must exist");

    let doc = MarkdownParser.parse(&md, Path::new("api-reference.md")).expect("parse must succeed");

    let counter = TokenCounter::new().expect("tokeniser init");
    let chunker = StructuralChunker::new(ChunkConfig::default(), counter);
    let tree = chunker.chunk(&doc, "api-reference.md", 2);

    assert!(!tree.is_empty());

    // The api-reference fixture has `pub fn` code blocks — they should be
    // extracted as NodeKind::CodeBlock chunks.
    let code: Vec<_> = tree.nodes.iter().filter(|(c, _)| c.kind == NodeKind::CodeBlock).collect();
    assert!(!code.is_empty(), "API reference should extract code signatures");
}

#[test]
fn discover_finds_both_fixtures() {
    let dir = fixtures_dir();
    let files = discover_files(&dir, false).expect("discover_files must succeed");
    assert!(files.len() >= 2, "should find at least the two bundled fixtures");
    let names: Vec<_> = files.iter().filter_map(|p| p.file_name()).collect();
    assert!(
        names.iter().any(|n| *n == "getting-started.md"),
        "getting-started.md missing from discovered files"
    );
    assert!(
        names.iter().any(|n| *n == "api-reference.md"),
        "api-reference.md missing from discovered files"
    );
}

// ── DB write + FTS (no model) ─────────────────────────────────────────────────

#[tokio::test]
async fn parse_chunk_index_and_fts_search() {
    // Full pipeline without the embedder: parse → chunk → insert nodes → FTS.
    let f = NamedTempFile::with_suffix(".db").expect("tempfile");
    let db = Db::open(f.path()).await.expect("Db::open");

    let doc_id =
        db.insert_doc("getting-started.md".into(), Some("Getting Started".into())).await.unwrap();

    let md = std::fs::read_to_string(fixtures_dir().join("getting-started.md")).unwrap();
    let doc = MarkdownParser.parse(&md, Path::new("getting-started.md")).unwrap();
    let counter = TokenCounter::new().unwrap();
    let chunker = StructuralChunker::new(ChunkConfig::default(), counter);
    let tree = chunker.chunk(&doc, "getting-started.md", 2);

    // Insert every chunk into the DB (no embedding step).
    for (chunk, _parent) in tree.nodes {
        db.insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: chunk.kind,
            level: None,
            title: chunk.heading_path.last().cloned(),
            content: Some(chunk.text()),
            token_count: chunk.token_count,
            lang: None,
        })
        .await
        .unwrap();
    }

    db.rebuild_fts().await.unwrap();

    // "cargo" appears in the fixture's installation section.
    let hits = db.fts_search("cargo".into(), 10).await.unwrap();
    assert!(!hits.is_empty(), "FTS should find 'cargo' in the fixture content");
}

// ── Build edge cases (no model) ───────────────────────────────────────────────

#[test]
fn parse_and_chunk_empty_document_does_not_panic() {
    let doc = MarkdownParser.parse("", Path::new("empty.md")).expect("parse empty must succeed");
    let counter = TokenCounter::new().expect("tokeniser init");
    let chunker = StructuralChunker::new(ChunkConfig::default(), counter);
    let tree = chunker.chunk(&doc, "empty.md", 2);
    // An empty document should produce zero or minimal output — the key
    // invariant is that nothing panics or returns an error.
    let _ = tree;
}

#[test]
fn parse_and_chunk_headings_only_document_does_not_panic() {
    let md = "# Introduction\n\n## Getting Started\n\n### Installation\n\n## Configuration\n";
    let doc = MarkdownParser.parse(md, Path::new("headings.md")).expect("parse must succeed");
    let counter = TokenCounter::new().expect("tokeniser init");
    let chunker = StructuralChunker::new(ChunkConfig::default(), counter);
    let tree = chunker.chunk(&doc, "headings.md", 2);
    // Headings with no body may produce zero prose chunks — that is fine.
    let _ = tree;
}

#[test]
fn parse_and_chunk_unicode_content_does_not_panic() {
    // Mix of CJK, Arabic (RTL), emoji, and a Rust code block.
    let md = "# 日本語ドキュメント\n\nこのライブラリは非常に便利です。🚀\n\n\
              ## نمونه\n\nاین یک متن فارسی است\n\n\
              ```rust\nfn main() { println!(\"Hello, 世界!\"); }\n```\n";
    let doc = MarkdownParser.parse(md, Path::new("unicode.md")).expect("parse must succeed");
    let counter = TokenCounter::new().expect("tokeniser init");
    let chunker = StructuralChunker::new(ChunkConfig::default(), counter);
    let tree = chunker.chunk(&doc, "unicode.md", 2);
    assert!(!tree.is_empty(), "unicode document must produce at least one chunk");
    let code: Vec<_> =
        tree.nodes.iter().filter(|(c, _)| c.kind == NodeKind::CodeBlock).collect();
    assert!(!code.is_empty(), "rust code block in unicode doc must be extracted");
}

// ── Full build pipeline (requires embedding model) ────────────────────────────

/// End-to-end build of the fixture package using the real embedding model.
///
/// Skipped by default in CI.  Run with `cargo test -- --ignored` locally.
#[tokio::test]
#[ignore = "requires ~130 MB bge-small-en-v1.5 model download"]
async fn full_build_pipeline_produces_searchable_package() {
    use lore_build::builder::PackageBuilder;

    let cache =
        dirs_next::cache_dir().unwrap_or_else(std::env::temp_dir).join("lore").join("models");

    let builder = PackageBuilder::new(&cache).expect("builder init");
    let out = NamedTempFile::with_suffix(".db").expect("tempfile");

    let pkg = Package {
        name: "mylib".into(),
        registry: "cargo".into(),
        version: "1.0.0".into(),
        description: Some("Integration test fixture".into()),
        source_url: None,
        git_sha: None,
    };

    let stats =
        builder.build(&fixtures_dir(), pkg, out.path(), false).await.expect("build must succeed");

    assert!(stats.files_processed >= 2, "should process both fixture files");
    assert!(stats.chunk_count > 0, "should produce prose chunks");
    assert!(stats.code_block_count > 0, "should extract code blocks");

    // Verify the database is searchable.
    let db = Db::open(out.path()).await.expect("open built db");
    let hits = db.fts_search("bearer token".into(), 5).await.unwrap();
    assert!(!hits.is_empty(), "api-reference.md mentions bearer token");
}

/// Full search pipeline including vector search.
///
/// Skipped by default in CI.  Run with `cargo test -- --ignored` locally.
#[tokio::test]
#[ignore = "requires ~130 MB bge-small-en-v1.5 model download"]
async fn full_search_pipeline_returns_relevant_results() {
    use lore_build::builder::PackageBuilder;
    use lore_core::SearchConfig;

    let cache =
        dirs_next::cache_dir().unwrap_or_else(std::env::temp_dir).join("lore").join("models");

    let builder = PackageBuilder::new(&cache).expect("builder init");
    let out = NamedTempFile::with_suffix(".db").expect("tempfile");

    let pkg = Package {
        name: "mylib".into(),
        registry: "cargo".into(),
        version: "1.0.0".into(),
        description: None,
        source_url: None,
        git_sha: None,
    };

    builder.build(&fixtures_dir(), pkg, out.path(), false).await.expect("build");

    let db = Db::open(out.path()).await.unwrap();
    let embedder = builder.embedder();

    // Semantic query: "how do I authenticate" — should surface the bearer-token section.
    let q = "how do I authenticate";
    let q_embedding = embedder.embed(q).expect("embed query");
    let config = SearchConfig::default();
    let results = lore_search::search(&db, q, &q_embedding, &config).await.unwrap();

    assert!(!results.is_empty(), "search must return results");
    // At least one result should be from the API reference (authentication section).
    let has_auth = results.iter().any(|r| {
        r.node.content.as_deref().unwrap_or("").to_lowercase().contains("bearer")
            || r.node.title.as_deref().unwrap_or("").to_lowercase().contains("auth")
    });
    assert!(has_auth, "search should surface authentication content");
}

// ── Registry YAML integrity ───────────────────────────────────────────────────

#[test]
fn registry_yaml_files_parse_without_error() {
    // Walk the packages/ directory and ensure every YAML parses cleanly.
    let packages_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages");
    if !packages_dir.exists() {
        return; // packages/ not checked out — skip
    }

    let yaml_files = collect_yaml_files(&packages_dir);
    assert!(!yaml_files.is_empty(), "at least one YAML package spec must exist");

    for path in &yaml_files {
        let content =
            std::fs::read_to_string(path).unwrap_or_else(|_| panic!("read {}", path.display()));
        serde_yaml::from_str::<serde_yaml::Value>(&content)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

/// Recursively collect all `.yaml` / `.yml` files under `dir`.
fn collect_yaml_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut found = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(collect_yaml_files(&path));
        } else if path.extension().is_some_and(|e| e == "yaml" || e == "yml") {
            found.push(path);
        }
    }
    found
}
