//! Integration tests for the DB + FTS search pipeline.
//!
//! These tests exercise the full path from node insertion through FTS indexing
//! to keyword search result retrieval.  No embedding model is required.

use lore_core::{Db, NewNode, NodeKind, Package, SearchConfig};
use tempfile::NamedTempFile;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Creates a temporary on-disk database that is deleted when the returned
/// handle is dropped.
async fn temp_db() -> (Db, NamedTempFile) {
    let f = NamedTempFile::with_suffix(".db").expect("tempfile");
    let db = Db::open(f.path()).await.expect("Db::open");
    (db, f)
}

/// Writes package metadata to `db` using the `meta` table convention.
async fn write_package_meta(db: &Db, pkg: &Package) {
    db.set_meta("name".into(), pkg.name.clone()).await.unwrap();
    db.set_meta("registry".into(), pkg.registry.clone()).await.unwrap();
    db.set_meta("version".into(), pkg.version.clone()).await.unwrap();
    if let Some(ref d) = pkg.description {
        db.set_meta("description".into(), d.clone()).await.unwrap();
    }
}

/// Returns a minimal [`Package`] suitable for writing package metadata.
fn test_package(name: &str) -> Package {
    Package {
        name: name.into(),
        registry: "cargo".into(),
        version: "0.1.0".into(),
        description: Some("Integration test package".into()),
        source_url: None,
        git_sha: None,
    }
}

// ── Schema + CRUD ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn open_creates_schema() {
    let (_db, _f) = temp_db().await;
    // If open() succeeds the schema was created.
}

#[tokio::test]
async fn insert_and_retrieve_doc() {
    let (db, _f) = temp_db().await;

    let id = db.insert_doc("docs/index.md".into(), Some("Index".into())).await.expect("insert_doc");

    let doc = db.get_doc(id).await.expect("get_doc");
    assert_eq!(doc.path, "docs/index.md");
    assert_eq!(doc.title.as_deref(), Some("Index"));
}

#[tokio::test]
async fn insert_and_retrieve_node() {
    let (db, _f) = temp_db().await;
    let doc_id = db.insert_doc("docs/api.md".into(), None).await.expect("insert_doc");

    let node_id = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: Some("Overview".into()),
            content: Some("The client connects to the API server.".into()),
            token_count: 8,
            lang: None,
        })
        .await
        .expect("insert_node");

    let node = db.get_node(node_id).await.expect("get_node");
    assert_eq!(node.kind, NodeKind::Chunk);
    assert_eq!(node.title.as_deref(), Some("Overview"));
}

#[tokio::test]
async fn heading_ancestry_chain() {
    let (db, _f) = temp_db().await;
    let doc_id = db.insert_doc("docs/guide.md".into(), None).await.unwrap();

    let h2_id = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Heading,
            level: Some(2),
            title: Some("Installation".into()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .unwrap();

    let h3_id = db
        .insert_node(NewNode {
            parent_id: Some(h2_id),
            doc_id,
            kind: NodeKind::Heading,
            level: Some(3),
            title: Some("macOS".into()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .unwrap();

    let chunk_id = db
        .insert_node(NewNode {
            parent_id: Some(h3_id),
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: None,
            content: Some("Install via Homebrew: brew install mylib".into()),
            token_count: 7,
            lang: None,
        })
        .await
        .unwrap();

    let ancestors = db.get_ancestors(chunk_id).await.unwrap();
    assert_eq!(ancestors.len(), 2, "chunk should have two heading ancestors");
    assert_eq!(ancestors[0].id, h2_id);
    assert_eq!(ancestors[1].id, h3_id);

    let path = db.get_heading_path(chunk_id).await.unwrap();
    assert_eq!(path, vec!["Installation".to_owned(), "macOS".to_owned()]);
}

// ── FTS Search ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fts_search_finds_inserted_content() {
    let (db, _f) = temp_db().await;
    let doc_id = db.insert_doc("guide.md".into(), Some("Guide".into())).await.unwrap();

    db.insert_node(NewNode {
        parent_id: None,
        doc_id,
        kind: NodeKind::Chunk,
        level: None,
        title: Some("Authentication".into()),
        content: Some("Pass a bearer token in the Authorization header.".into()),
        token_count: 9,
        lang: None,
    })
    .await
    .unwrap();

    db.insert_node(NewNode {
        parent_id: None,
        doc_id,
        kind: NodeKind::Chunk,
        level: None,
        title: Some("Rate Limiting".into()),
        content: Some("Requests are limited to 100 per minute per API key.".into()),
        token_count: 11,
        lang: None,
    })
    .await
    .unwrap();

    db.rebuild_fts().await.unwrap();

    let hits = db.fts_search("bearer token".into(), 10).await.unwrap();
    assert_eq!(hits.len(), 1, "only the authentication chunk matches");
    assert_eq!(hits[0].node.title.as_deref(), Some("Authentication"));
}

#[tokio::test]
async fn fts_search_returns_empty_for_no_match() {
    let (db, _f) = temp_db().await;
    let doc_id = db.insert_doc("doc.md".into(), None).await.unwrap();

    db.insert_node(NewNode {
        parent_id: None,
        doc_id,
        kind: NodeKind::Chunk,
        level: None,
        title: None,
        content: Some("Hello world.".into()),
        token_count: 2,
        lang: None,
    })
    .await
    .unwrap();

    db.rebuild_fts().await.unwrap();

    let hits = db.fts_search("xyzzy_nonexistent_term".into(), 10).await.unwrap();
    assert!(hits.is_empty(), "no match should return empty results");
}

#[tokio::test]
async fn search_sanitises_punctuation_in_query() {
    // lore_search::search strips punctuation (parens, colons, dots) before
    // calling FTS5.  A query like "user.authenticate()" should not error.
    let (db, _f) = temp_db().await;
    db.rebuild_fts().await.unwrap();

    let zero_embedding = vec![0.0f32; lore_build::embedder::EMBEDDING_DIMS];
    let config = lore_core::SearchConfig::default();
    // Punctuation is stripped; remaining tokens "user" and "authenticate" are valid.
    let result = lore_search::search(&db, "user.authenticate()", &zero_embedding, &config).await;
    assert!(result.is_ok(), "punctuation in query must not cause an error: {result:?}");
    assert!(result.unwrap().is_empty()); // empty DB → no results
}

// ── Meta table ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn meta_set_and_get() {
    let (db, _f) = temp_db().await;

    db.set_meta("manifest".into(), "Installs: install_pkg, remove_pkg".into()).await.unwrap();

    let value = db.get_meta("manifest".into()).await.unwrap();
    assert_eq!(value.as_deref(), Some("Installs: install_pkg, remove_pkg"));
}

#[tokio::test]
async fn meta_missing_key_returns_none() {
    let (db, _f) = temp_db().await;
    let value = db.get_meta("nonexistent".into()).await.unwrap();
    assert!(value.is_none());
}

#[tokio::test]
async fn meta_upsert_replaces_previous_value() {
    let (db, _f) = temp_db().await;
    db.set_meta("key".into(), "old".into()).await.unwrap();
    db.set_meta("key".into(), "new".into()).await.unwrap();
    let value = db.get_meta("key".into()).await.unwrap();
    assert_eq!(value.as_deref(), Some("new"));
}

// ── Package metadata ──────────────────────────────────────────────────────────

#[tokio::test]
async fn package_metadata_round_trip() {
    let (db, _f) = temp_db().await;
    let pkg = test_package("tokio");

    write_package_meta(&db, &pkg).await;

    let loaded = db.get_package_meta().await.unwrap();
    assert_eq!(loaded.name, "tokio");
    assert_eq!(loaded.version, "0.1.0");
    assert_eq!(loaded.registry, "cargo");
}

// ── Search pipeline (FTS only — no model required) ────────────────────────────

#[tokio::test]
async fn search_pipeline_fts_only() {
    // Insert content, rebuild FTS, then run lore_search::search with an
    // empty query embedding (disables vector search, relies on FTS only).
    let (db, _f) = temp_db().await;
    let doc_id = db.insert_doc("install.md".into(), Some("Install Guide".into())).await.unwrap();

    db.insert_node(NewNode {
        parent_id: None,
        doc_id,
        kind: NodeKind::Chunk,
        level: None,
        title: Some("Cargo Install".into()),
        content: Some("Install mylib with cargo install mylib-cli.".into()),
        token_count: 8,
        lang: None,
    })
    .await
    .unwrap();

    db.insert_node(NewNode {
        parent_id: None,
        doc_id,
        kind: NodeKind::Chunk,
        level: None,
        title: Some("Docker".into()),
        content: Some("Run the container with docker run mylib.".into()),
        token_count: 8,
        lang: None,
    })
    .await
    .unwrap();

    db.rebuild_fts().await.unwrap();

    // Zero-dim embedding (zeros, no semantic component) → FTS carries the result.
    let zero_embedding = vec![0.0f32; lore_build::embedder::EMBEDDING_DIMS];
    let config = SearchConfig::default();
    let results =
        lore_search::search(&db, "cargo install", &zero_embedding, &config).await.unwrap();

    assert!(!results.is_empty(), "FTS should find 'cargo install'");
    let titles: Vec<_> = results.iter().map(|r| r.node.title.as_deref()).collect();
    assert!(titles.contains(&Some("Cargo Install")), "cargo chunk should be in results");
}

// ── Search edge cases ─────────────────────────────────────────────────────────

#[tokio::test]
async fn search_empty_query_on_empty_db_returns_empty() {
    let (db, _f) = temp_db().await;
    db.rebuild_fts().await.unwrap();
    let zero_embedding = vec![0.0f32; lore_build::embedder::EMBEDDING_DIMS];
    let config = lore_core::SearchConfig::default();
    let results = lore_search::search(&db, "", &zero_embedding, &config).await;
    assert!(results.is_ok(), "empty query must not error: {results:?}");
    assert!(results.unwrap().is_empty());
}

#[tokio::test]
async fn search_all_special_chars_query_does_not_error() {
    let (db, _f) = temp_db().await;
    db.rebuild_fts().await.unwrap();
    let zero_embedding = vec![0.0f32; lore_build::embedder::EMBEDDING_DIMS];
    let config = lore_core::SearchConfig::default();
    let result =
        lore_search::search(&db, "!!@@##$$%%^^&&**()", &zero_embedding, &config).await;
    assert!(result.is_ok(), "all-special-chars query must not error: {result:?}");
    assert!(result.unwrap().is_empty());
}

#[tokio::test]
async fn search_very_long_query_does_not_error() {
    let (db, _f) = temp_db().await;
    db.rebuild_fts().await.unwrap();
    let long_query = "word ".repeat(200); // ~1 000 chars
    let zero_embedding = vec![0.0f32; lore_build::embedder::EMBEDDING_DIMS];
    let config = lore_core::SearchConfig::default();
    let result =
        lore_search::search(&db, long_query.trim(), &zero_embedding, &config).await;
    assert!(result.is_ok(), "long query must not error: {result:?}");
}

#[tokio::test]
async fn search_token_budget_zero_still_returns_one_result() {
    // budget::apply guarantees at least one result even when budget = 0.
    let (db, _f) = temp_db().await;
    let doc_id = db.insert_doc("doc.md".into(), None).await.unwrap();
    db.insert_node(NewNode {
        parent_id: None,
        doc_id,
        kind: NodeKind::Chunk,
        level: None,
        title: Some("Install".into()),
        content: Some("cargo install mylib".into()),
        token_count: 3,
        lang: None,
    })
    .await
    .unwrap();
    db.rebuild_fts().await.unwrap();

    let zero_embedding = vec![0.0f32; lore_build::embedder::EMBEDDING_DIMS];
    let config = lore_core::SearchConfig { token_budget: 0, ..Default::default() };
    let results =
        lore_search::search(&db, "cargo", &zero_embedding, &config).await.unwrap();
    assert_eq!(results.len(), 1, "budget=0 must still return exactly one result");
}

// ── Corruption / missing state ─────────────────────────────────────────────────

#[tokio::test]
async fn open_corrupt_db_file_returns_error() {
    let f = NamedTempFile::with_suffix(".db").expect("tempfile");
    std::fs::write(f.path(), b"this is not a sqlite database file").unwrap();
    let result = lore_core::Db::open(f.path()).await;
    assert!(result.is_err(), "opening a corrupt file must return an error");
}

// ── Concurrent access ─────────────────────────────────────────────────────────

#[tokio::test]
async fn concurrent_searches_do_not_deadlock() {
    let (db, _f) = temp_db().await;
    let doc_id = db.insert_doc("doc.md".into(), None).await.unwrap();
    for i in 0..5u32 {
        db.insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: Some(format!("Section {i}")),
            content: Some(format!("content about topic {i}")),
            token_count: 5,
            lang: None,
        })
        .await
        .unwrap();
    }
    db.rebuild_fts().await.unwrap();

    let zero_embedding = vec![0.0f32; lore_build::embedder::EMBEDDING_DIMS];
    let config = lore_core::SearchConfig::default();

    let (db_a, db_b) = (db.clone(), db.clone());
    let (emb_a, emb_b) = (zero_embedding.clone(), zero_embedding);
    let (cfg_a, cfg_b) = (config.clone(), config);

    let task_a =
        tokio::spawn(async move { lore_search::search(&db_a, "content", &emb_a, &cfg_a).await });
    let task_b =
        tokio::spawn(async move { lore_search::search(&db_b, "topic", &emb_b, &cfg_b).await });

    let (res_a, res_b) = tokio::join!(task_a, task_b);
    res_a.expect("task A panicked").expect("search A errored");
    res_b.expect("task B panicked").expect("search B errored");
}

// ── get_nodes_by_kind ─────────────────────────────────────────────────────────

#[tokio::test]
async fn get_nodes_by_kind_filters_correctly() {
    let (db, _f) = temp_db().await;
    let doc_id = db.insert_doc("mixed.md".into(), None).await.unwrap();

    db.insert_node(NewNode {
        parent_id: None,
        doc_id,
        kind: NodeKind::Heading,
        level: Some(2),
        title: Some("Overview".into()),
        content: None,
        token_count: 0,
        lang: None,
    })
    .await
    .unwrap();

    db.insert_node(NewNode {
        parent_id: None,
        doc_id,
        kind: NodeKind::Chunk,
        level: None,
        title: None,
        content: Some("Some prose content.".into()),
        token_count: 3,
        lang: None,
    })
    .await
    .unwrap();

    db.insert_node(NewNode {
        parent_id: None,
        doc_id,
        kind: NodeKind::CodeBlock,
        level: None,
        title: None,
        content: Some("fn main() {}".into()),
        token_count: 4,
        lang: Some("rust".into()),
    })
    .await
    .unwrap();

    let headings = db.get_nodes_by_kind(NodeKind::Heading).await.unwrap();
    let chunks = db.get_nodes_by_kind(NodeKind::Chunk).await.unwrap();
    let code = db.get_nodes_by_kind(NodeKind::CodeBlock).await.unwrap();

    assert_eq!(headings.len(), 1);
    assert_eq!(chunks.len(), 1);
    assert_eq!(code.len(), 1);
    assert_eq!(code[0].lang.as_deref(), Some("rust"));
}
