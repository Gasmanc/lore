//! Integration tests for [`lore_core::Db`].
//!
//! Each test opens a fresh in-memory database so they are fully isolated and
//! require no filesystem access.

use lore_core::{Db, LoreError, NewNode, NodeKind};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Opens a fresh in-memory database.  Panics on error.
async fn open_db() -> Db {
    Db::open_in_memory().await.expect("failed to open in-memory database")
}

// ---------------------------------------------------------------------------
// Schema / connectivity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_open_and_migrate_succeeds() {
    // If open_in_memory() returns Ok the migrations ran without errors.
    open_db().await;
}

#[tokio::test]
async fn test_schema_version_is_set() {
    let db = open_db().await;
    let version = db
        .get_meta("schema_version".to_owned())
        .await
        .expect("get_meta failed");

    let v: u32 = version
        .expect("schema_version key missing")
        .parse()
        .expect("schema_version is not an integer");

    assert_eq!(v, 4, "expected 4 migrations to have been applied");
}

#[tokio::test]
async fn test_nodes_fts_table_exists() {
    let db = open_db().await;
    // Inserting into the FTS table is the simplest way to confirm it exists.
    db.rebuild_fts().await.expect("rebuild_fts failed — nodes_fts likely missing");
}

#[tokio::test]
async fn test_node_embeddings_table_exists() {
    let db = open_db().await;
    // Inserting a dummy zero-vector confirms vec0 is available.
    let zeros: Vec<f32> = vec![0.0_f32; 384];
    // We need a real node to insert an embedding for.
    let doc_id = db
        .insert_doc("dummy.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let node_id = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: None,
            content: Some("dummy content".to_owned()),
            token_count: 2,
            lang: None,
        })
        .await
        .expect("insert_node failed");

    db.insert_embedding(node_id, zeros.clone())
        .await
        .expect("insert_embedding failed — node_embeddings table likely missing");

    let retrieved = db
        .get_embedding(node_id)
        .await
        .expect("get_embedding failed")
        .expect("embedding should be present");

    assert_eq!(retrieved.len(), 384, "embedding dimension mismatch");
    assert!(
        retrieved.iter().all(|&v| v == 0.0_f32),
        "embedding values should all be zero"
    );
}

// ---------------------------------------------------------------------------
// Meta table
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_set_and_get_meta_roundtrip() {
    let db = open_db().await;
    db.set_meta("name".to_owned(), "next".to_owned())
        .await
        .expect("set_meta failed");

    let value = db
        .get_meta("name".to_owned())
        .await
        .expect("get_meta failed");

    assert_eq!(value.as_deref(), Some("next"));
}

#[tokio::test]
async fn test_get_meta_missing_key_returns_none() {
    let db = open_db().await;
    let value = db
        .get_meta("nonexistent".to_owned())
        .await
        .expect("get_meta failed");
    assert!(value.is_none());
}

#[tokio::test]
async fn test_set_meta_overwrites_existing_value() {
    let db = open_db().await;
    db.set_meta("key".to_owned(), "first".to_owned())
        .await
        .expect("set_meta failed");
    db.set_meta("key".to_owned(), "second".to_owned())
        .await
        .expect("set_meta overwrite failed");

    let value = db
        .get_meta("key".to_owned())
        .await
        .expect("get_meta failed");
    assert_eq!(value.as_deref(), Some("second"));
}

// ---------------------------------------------------------------------------
// Docs table
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_insert_doc_returns_id() {
    let db = open_db().await;
    let id = db
        .insert_doc("docs/intro.md".to_owned(), Some("Introduction".to_owned()))
        .await
        .expect("insert_doc failed");
    assert!(id > 0, "expected a positive doc id");
}

#[tokio::test]
async fn test_insert_doc_idempotent() {
    let db = open_db().await;
    let id1 = db
        .insert_doc("docs/intro.md".to_owned(), None)
        .await
        .expect("first insert failed");
    let id2 = db
        .insert_doc("docs/intro.md".to_owned(), None)
        .await
        .expect("second insert failed");
    assert_eq!(id1, id2, "inserting the same path twice should return the same id");
}

#[tokio::test]
async fn test_get_doc_retrieves_inserted_record() {
    let db = open_db().await;
    let id = db
        .insert_doc("api/reference.md".to_owned(), Some("API Reference".to_owned()))
        .await
        .expect("insert_doc failed");

    let doc = db.get_doc(id).await.expect("get_doc failed");
    assert_eq!(doc.id, id);
    assert_eq!(doc.path, "api/reference.md");
    assert_eq!(doc.title.as_deref(), Some("API Reference"));
}

// ---------------------------------------------------------------------------
// Nodes table — insertion and retrieval
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_insert_and_get_heading_node() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/caching.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let node_id = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Heading,
            level: Some(2),
            title: Some("Caching".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert_node failed");

    let node = db.get_node(node_id).await.expect("get_node failed");
    assert_eq!(node.id, node_id);
    assert_eq!(node.kind, NodeKind::Heading);
    assert_eq!(node.level, Some(2));
    assert_eq!(node.title.as_deref(), Some("Caching"));
    assert!(node.parent_id.is_none());
}

#[tokio::test]
async fn test_insert_and_get_chunk_node() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/api.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let id = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: None,
            content: Some("This is a prose chunk.".to_owned()),
            token_count: 6,
            lang: None,
        })
        .await
        .expect("insert_node failed");

    let node = db.get_node(id).await.expect("get_node failed");
    assert_eq!(node.kind, NodeKind::Chunk);
    assert_eq!(node.content.as_deref(), Some("This is a prose chunk."));
    assert_eq!(node.token_count, 6);
}

#[tokio::test]
async fn test_insert_and_get_code_block_node() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/examples.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let id = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::CodeBlock,
            level: None,
            title: None,
            content: Some("fn main() {}".to_owned()),
            token_count: 4,
            lang: Some("rust".to_owned()),
        })
        .await
        .expect("insert_node failed");

    let node = db.get_node(id).await.expect("get_node failed");
    assert_eq!(node.kind, NodeKind::CodeBlock);
    assert_eq!(node.lang.as_deref(), Some("rust"));
}

// ---------------------------------------------------------------------------
// Nodes table — path enumeration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_root_node_path_is_its_own_id() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/root.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let root_id = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Heading,
            level: Some(1),
            title: Some("Root".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert_node failed");

    let root = db.get_node(root_id).await.expect("get_node failed");
    assert_eq!(root.path, root_id.to_string());
}

#[tokio::test]
async fn test_child_node_path_extends_parent_path() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/hierarchy.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let parent_id = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Heading,
            level: Some(1),
            title: Some("Parent".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert parent failed");

    let child_id = db
        .insert_node(NewNode {
            parent_id: Some(parent_id),
            doc_id,
            kind: NodeKind::Heading,
            level: Some(2),
            title: Some("Child".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert child failed");

    let child = db.get_node(child_id).await.expect("get_node failed");
    assert_eq!(
        child.path,
        format!("{parent_id}/{child_id}"),
        "child path should extend parent path"
    );
}

#[tokio::test]
async fn test_three_level_path_enumeration() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/deep.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let l1 = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Heading,
            level: Some(1),
            title: Some("H1".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert L1 failed");

    let l2 = db
        .insert_node(NewNode {
            parent_id: Some(l1),
            doc_id,
            kind: NodeKind::Heading,
            level: Some(2),
            title: Some("H2".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert L2 failed");

    let l3 = db
        .insert_node(NewNode {
            parent_id: Some(l2),
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: None,
            content: Some("Deep content".to_owned()),
            token_count: 2,
            lang: None,
        })
        .await
        .expect("insert L3 failed");

    let node = db.get_node(l3).await.expect("get_node failed");
    assert_eq!(node.path, format!("{l1}/{l2}/{l3}"));
}

// ---------------------------------------------------------------------------
// Nodes table — hierarchy queries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_children_returns_direct_children_only() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/children.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let root = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Heading,
            level: Some(1),
            title: Some("Root".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert root failed");

    let child_a = db
        .insert_node(NewNode {
            parent_id: Some(root),
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: None,
            content: Some("Child A".to_owned()),
            token_count: 2,
            lang: None,
        })
        .await
        .expect("insert child_a failed");

    let child_b = db
        .insert_node(NewNode {
            parent_id: Some(root),
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: None,
            content: Some("Child B".to_owned()),
            token_count: 2,
            lang: None,
        })
        .await
        .expect("insert child_b failed");

    // Grandchild — should NOT appear in root's children.
    db.insert_node(NewNode {
        parent_id: Some(child_a),
        doc_id,
        kind: NodeKind::CodeBlock,
        level: None,
        title: None,
        content: Some("code".to_owned()),
        token_count: 1,
        lang: None,
    })
    .await
    .expect("insert grandchild failed");

    let children = db.get_children(root).await.expect("get_children failed");
    let ids: Vec<i64> = children.iter().map(|n| n.id).collect();

    assert_eq!(ids.len(), 2, "root should have exactly 2 direct children");
    assert!(ids.contains(&child_a));
    assert!(ids.contains(&child_b));
}

#[tokio::test]
async fn test_get_ancestors_returns_correct_chain() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/ancestors.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let l1 = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Heading,
            level: Some(1),
            title: Some("Level 1".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert l1 failed");

    let l2 = db
        .insert_node(NewNode {
            parent_id: Some(l1),
            doc_id,
            kind: NodeKind::Heading,
            level: Some(2),
            title: Some("Level 2".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert l2 failed");

    let l3 = db
        .insert_node(NewNode {
            parent_id: Some(l2),
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: None,
            content: Some("Leaf content".to_owned()),
            token_count: 2,
            lang: None,
        })
        .await
        .expect("insert l3 failed");

    let ancestors = db.get_ancestors(l3).await.expect("get_ancestors failed");
    let ancestor_ids: Vec<i64> = ancestors.iter().map(|n| n.id).collect();

    assert_eq!(ancestor_ids, vec![l1, l2], "ancestors should be [l1, l2] in root-first order");
}

#[tokio::test]
async fn test_root_node_has_no_ancestors() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/root_only.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let root = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Heading,
            level: Some(1),
            title: Some("Only node".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert root failed");

    let ancestors = db.get_ancestors(root).await.expect("get_ancestors failed");
    assert!(ancestors.is_empty(), "root node should have no ancestors");
}

// ---------------------------------------------------------------------------
// Heading path (breadcrumb)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_heading_path_returns_heading_titles_only() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/headings.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let h1 = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Heading,
            level: Some(1),
            title: Some("Next.js Docs".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert h1 failed");

    let h2 = db
        .insert_node(NewNode {
            parent_id: Some(h1),
            doc_id,
            kind: NodeKind::Heading,
            level: Some(2),
            title: Some("Caching".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert h2 failed");

    let chunk = db
        .insert_node(NewNode {
            parent_id: Some(h2),
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: None,
            content: Some("cacheLife() controls TTL.".to_owned()),
            token_count: 5,
            lang: None,
        })
        .await
        .expect("insert chunk failed");

    let path = db
        .get_heading_path(chunk)
        .await
        .expect("get_heading_path failed");

    assert_eq!(path, vec!["Next.js Docs", "Caching"]);
}

#[tokio::test]
async fn test_get_heading_path_skips_non_heading_ancestors() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("docs/mixed.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let heading = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Heading,
            level: Some(1),
            title: Some("Overview".to_owned()),
            content: None,
            token_count: 0,
            lang: None,
        })
        .await
        .expect("insert heading failed");

    // Insert a Chunk as an intermediate ancestor (unusual but possible).
    let chunk_parent = db
        .insert_node(NewNode {
            parent_id: Some(heading),
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: None,
            content: Some("Some prose".to_owned()),
            token_count: 2,
            lang: None,
        })
        .await
        .expect("insert chunk_parent failed");

    let leaf = db
        .insert_node(NewNode {
            parent_id: Some(chunk_parent),
            doc_id,
            kind: NodeKind::CodeBlock,
            level: None,
            title: None,
            content: Some("let x = 1;".to_owned()),
            token_count: 4,
            lang: Some("js".to_owned()),
        })
        .await
        .expect("insert leaf failed");

    let path = db
        .get_heading_path(leaf)
        .await
        .expect("get_heading_path failed");

    // Only the Heading ancestor contributes to the breadcrumb.
    assert_eq!(path, vec!["Overview"]);
}

// ---------------------------------------------------------------------------
// Embeddings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_insert_and_retrieve_embedding_roundtrip() {
    let db = open_db().await;
    let doc_id = db
        .insert_doc("embed_test.md".to_owned(), None)
        .await
        .expect("insert_doc failed");

    let node_id = db
        .insert_node(NewNode {
            parent_id: None,
            doc_id,
            kind: NodeKind::Chunk,
            level: None,
            title: None,
            content: Some("test content".to_owned()),
            token_count: 2,
            lang: None,
        })
        .await
        .expect("insert_node failed");

    let original: Vec<f32> = (0..384).map(|i| i as f32 * 0.001_f32).collect();
    db.insert_embedding(node_id, original.clone())
        .await
        .expect("insert_embedding failed");

    let retrieved = db
        .get_embedding(node_id)
        .await
        .expect("get_embedding failed")
        .expect("embedding should be present");

    assert_eq!(retrieved.len(), original.len());
    for (a, b) in original.iter().zip(retrieved.iter()) {
        assert!(
            (a - b).abs() < f32::EPSILON,
            "embedding value mismatch: {a} vs {b}"
        );
    }
}

#[tokio::test]
async fn test_get_embedding_absent_node_returns_none() {
    let db = open_db().await;
    let result = db
        .get_embedding(999_999)
        .await
        .expect("get_embedding should not error for missing node");
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// Package metadata
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_package_meta_returns_correct_fields() {
    let db = open_db().await;
    db.set_meta("name".to_owned(), "next".to_owned())
        .await
        .expect("set name failed");
    db.set_meta("registry".to_owned(), "npm".to_owned())
        .await
        .expect("set registry failed");
    db.set_meta("version".to_owned(), "15.0.0".to_owned())
        .await
        .expect("set version failed");
    db.set_meta(
        "description".to_owned(),
        "The React Framework".to_owned(),
    )
    .await
    .expect("set description failed");

    let pkg = db.get_package_meta().await.expect("get_package_meta failed");
    assert_eq!(pkg.name, "next");
    assert_eq!(pkg.registry, "npm");
    assert_eq!(pkg.version, "15.0.0");
    assert_eq!(pkg.description.as_deref(), Some("The React Framework"));
    assert!(pkg.source_url.is_none());
    assert!(pkg.git_sha.is_none());
}

#[tokio::test]
async fn test_get_package_meta_fails_without_required_keys() {
    let db = open_db().await;
    // Only set `name`; `registry` and `version` are missing.
    db.set_meta("name".to_owned(), "orphan".to_owned())
        .await
        .expect("set_meta failed");

    let result = db.get_package_meta().await;
    assert!(
        matches!(result, Err(LoreError::Database(_))),
        "expected a database error when required meta keys are absent"
    );
}

// ---------------------------------------------------------------------------
// NodeKind helpers
// ---------------------------------------------------------------------------

#[test]
fn test_node_kind_as_str_roundtrip() {
    for kind in [NodeKind::Heading, NodeKind::Chunk, NodeKind::CodeBlock] {
        let s = kind.as_str();
        let parsed = NodeKind::try_from(s).expect("try_from failed");
        assert_eq!(parsed, kind);
    }
}

#[test]
fn test_node_kind_try_from_invalid_returns_error() {
    let result = NodeKind::try_from("unknown_kind");
    assert!(result.is_err());
}
