//! Integration tests for the Phase 3/4/5.1 pipeline:
//! token counting, structural chunking, semantic refinement,
//! contextual embedding, and file discovery.

use std::path::Path;

use lore_build::{
    ChunkConfig, ContentBlock, HeadingNode, MarkdownParser, ParsedDoc, ParserRegistry,
    StructuralChunker, TokenCounter, build_contextual_text,
    detect_primary_heading_level, discover_files, parser::Parser,
};
use lore_core::NodeKind;
use tempfile::tempdir;
use std::fs;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn counter() -> TokenCounter {
    TokenCounter::new().expect("cl100k_base must init")
}

fn chunker() -> StructuralChunker {
    StructuralChunker::new(ChunkConfig::default(), counter())
}

fn heading(level: u8, title: &str, blocks: Vec<ContentBlock>, children: Vec<HeadingNode>) -> HeadingNode {
    HeadingNode { level, title: title.into(), blocks, children }
}

fn para(s: &str) -> ContentBlock {
    ContentBlock::Paragraph(s.into())
}


fn doc(title: &str, children: Vec<HeadingNode>) -> ParsedDoc {
    ParsedDoc {
        title: Some(title.into()),
        root: HeadingNode { children, ..HeadingNode::root() },
    }
}

// ── Token counter ─────────────────────────────────────────────────────────────

#[test]
fn test_counter_counts_prose() {
    let count = counter().count("The quick brown fox jumps over the lazy dog.");
    assert!((5..=15).contains(&count), "expected ~10 tokens, got {count}");
}

#[test]
fn test_counter_empty() {
    assert_eq!(counter().count(""), 0);
}

// ── Contextual text ───────────────────────────────────────────────────────────

#[test]
fn test_contextual_text_with_path() {
    let path = vec!["Docs".into(), "API".into(), "fetch()".into()];
    let out = build_contextual_text(&path, "Fetches data.");
    assert_eq!(out, "Docs > API > fetch()\n\nFetches data.");
}

#[test]
fn test_contextual_text_empty_path() {
    assert_eq!(build_contextual_text(&[], "content"), "content");
}

// ── Structural chunker ────────────────────────────────────────────────────────

#[test]
fn test_pipeline_parse_and_chunk_markdown() {
    // Parse a real Markdown document, then chunk it.
    let md = r#"# My Library

Welcome to my library.

## Installation

Install with cargo:

```toml
my-lib = "1.0"
```

## Usage

Import and call `run()`:

```rust
use my_lib::run;
run();
```

## Configuration

Set the environment variable `MY_LIB_PORT=8080`.
"#;
    let doc = MarkdownParser.parse(md, Path::new("lib.md")).unwrap();
    let primary = detect_primary_heading_level(&doc.root);

    let tree = chunker().chunk(&doc, "lib.md", primary);

    // Should have prose and code chunks.
    let prose: Vec<_> =
        tree.nodes.iter().filter(|(c, _)| c.kind == NodeKind::Chunk).collect();
    let code: Vec<_> =
        tree.nodes.iter().filter(|(c, _)| c.kind == NodeKind::CodeBlock).collect();

    assert!(!prose.is_empty(), "expected at least one prose chunk");
    assert!(!code.is_empty(), "expected at least one code chunk");

    // Every chunk records the doc path.
    for (chunk, _) in &tree.nodes {
        assert_eq!(chunk.doc_path, "lib.md");
    }
}

#[test]
fn test_chunk_tree_parent_links_correct() {
    // H2 "Parent" > H3 "Child".  The Child's prose chunk should have the
    // Parent's prose chunk as its parent.
    let d = doc(
        "Nested",
        vec![heading(
            2,
            "Parent",
            vec![para("Parent content.")],
            vec![heading(3, "Child", vec![para("Child content.")], vec![])],
        )],
    );
    let tree = chunker().chunk(&d, "test.md", 2);

    // Two prose chunks: Parent (idx=0, parent=None) and Child (idx=1, parent=Some(0)).
    assert_eq!(tree.nodes.len(), 2);
    assert!(tree.nodes[0].1.is_none(), "Parent should have no parent");
    assert_eq!(tree.nodes[1].1, Some(0), "Child should have Parent as parent");
}

#[test]
fn test_chunk_registry_end_to_end() {
    // Use the full parser registry to parse each supported format.
    let registry = ParserRegistry::new();

    let test_cases = [
        ("doc.md", "## Overview\n\nContent.\n"),
        ("doc.rst", "Overview\n========\n\nContent.\n"),
        ("doc.adoc", "== Overview\n\nContent.\n"),
    ];

    for (name, content) in &test_cases {
        let path = Path::new(name);
        let parsed = registry.parse(path, content).expect(name);
        let tree = chunker().chunk(&parsed, name, 2);
        assert!(
            !tree.nodes.is_empty(),
            "expected at least one chunk for {name}"
        );
    }
}

// ── File discovery ────────────────────────────────────────────────────────────

#[test]
fn test_discover_nested_structure() {
    let dir = tempdir().unwrap();
    let subdir = dir.path().join("guides");
    fs::create_dir(&subdir).unwrap();
    fs::write(dir.path().join("index.md"), "# Index").unwrap();
    fs::write(subdir.join("getting-started.md"), "# Start").unwrap();
    fs::write(subdir.join("advanced.rst"), "Advanced\n========").unwrap();
    fs::write(subdir.join("notes.txt"), "plain text").unwrap();

    let files = discover_files(dir.path(), false).unwrap();
    assert_eq!(files.len(), 3, "should find .md and .rst but not .txt");
}

#[test]
fn test_discover_excludes_all_excluded_dirs() {
    let dir = tempdir().unwrap();
    for excluded in lore_build::discovery::EXCLUDED_DIRS {
        let sub = dir.path().join(excluded);
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("hidden.md"), "# Hidden").unwrap();
    }
    fs::write(dir.path().join("visible.md"), "# Visible").unwrap();
    let files = discover_files(dir.path(), false).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("visible.md"));
}
