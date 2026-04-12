//! Integration tests for the lore-build parser module.

use lore_build::{
    AsciidocParser, ContentBlock, HeadingNode, HtmlParser, MarkdownParser, ParsedDoc,
    ParserRegistry, RstParser, detect_primary_heading_level,
    parser::Parser,
};
use std::path::Path;

// ── detect_primary_heading_level ─────────────────────────────────────────────

/// Build a HeadingNode tree that simulates an API reference: many H3 nodes
/// each containing code blocks and prose — the primary unit is H3.
fn api_reference_tree() -> HeadingNode {
    let mut root = HeadingNode::root();
    // One H2 module heading with lots of H3 function entries.
    let mut module = HeadingNode { level: 2, title: "Module".into(), ..HeadingNode::default() };
    for i in 0..8 {
        let mut func = HeadingNode {
            level: 3,
            title: format!("function_{i}"),
            ..HeadingNode::default()
        };
        func.blocks.push(ContentBlock::Paragraph("Description.".into()));
        func.blocks.push(ContentBlock::Code {
            lang: Some("rust".into()),
            content: format!("pub fn function_{i}() {{}}"),
        });
        module.children.push(func);
    }
    root.children.push(module);
    root
}

/// Build a tree simulating a tutorial: meaty H2 sections with several blocks each.
fn tutorial_tree() -> HeadingNode {
    let mut root = HeadingNode::root();
    for i in 0..4 {
        let mut section = HeadingNode {
            level: 2,
            title: format!("Chapter {i}"),
            ..HeadingNode::default()
        };
        // Each H2 has multiple content blocks.
        section.blocks.push(ContentBlock::Paragraph("Introduction text.".into()));
        section.blocks.push(ContentBlock::Paragraph("More details here.".into()));
        section.blocks.push(ContentBlock::Code {
            lang: Some("python".into()),
            content: format!("# example {i}"),
        });
        root.children.push(section);
    }
    root
}

#[test]
fn test_detect_primary_level_api_reference() {
    let root = api_reference_tree();
    // H3 nodes have avg 2 blocks > 1.5 threshold; H2 has avg 0 blocks.
    assert_eq!(detect_primary_heading_level(&root), 3);
}

#[test]
fn test_detect_primary_level_tutorial() {
    let root = tutorial_tree();
    // H2 nodes each have 3 blocks (avg 3.0 > 1.5).
    assert_eq!(detect_primary_heading_level(&root), 2);
}

#[test]
fn test_detect_primary_level_flat_doc() {
    // A document with no headings should default to 2.
    let root = HeadingNode::root();
    assert_eq!(detect_primary_heading_level(&root), 2);
}

// ── ParserRegistry ────────────────────────────────────────────────────────────

#[test]
fn test_registry_selects_markdown() {
    let registry = ParserRegistry::new();
    let doc = registry.parse(Path::new("README.md"), "## Hello\n\nWorld.\n").unwrap();
    assert!(!doc.root.children.is_empty());
}

#[test]
fn test_registry_selects_rst() {
    let registry = ParserRegistry::new();
    let doc = registry
        .parse(Path::new("docs.rst"), "Hello\n=====\n\nContent.\n")
        .unwrap();
    assert!(!doc.root.children.is_empty());
}

#[test]
fn test_registry_selects_adoc() {
    let registry = ParserRegistry::new();
    let doc = registry
        .parse(Path::new("guide.adoc"), "== Overview\n\nContent.\n")
        .unwrap();
    assert!(!doc.root.children.is_empty());
}

#[test]
fn test_registry_unknown_extension_errors() {
    let registry = ParserRegistry::new();
    let result = registry.parse(Path::new("file.xyz"), "content");
    assert!(result.is_err());
}

// ── MarkdownParser ────────────────────────────────────────────────────────────

#[test]
fn test_markdown_realistic_doc() {
    let md = r#"---
title: Tokio Runtime
---

# Tokio Runtime

The Tokio runtime provides async I/O and task scheduling.

## Creating a Runtime

Use the `Builder` to configure a multi-threaded runtime.

```rust
use tokio::runtime::Builder;

let rt = Builder::new_multi_thread()
    .worker_threads(4)
    .enable_all()
    .build()
    .unwrap();
```

## Shutting Down

Call `rt.shutdown_timeout()` to wait for all tasks to complete.
"#;
    let doc = MarkdownParser.parse(md, Path::new("doc.md")).unwrap();
    assert_eq!(doc.title.as_deref(), Some("Tokio Runtime"));
    let h1 = &doc.root.children[0];
    assert_eq!(h1.title, "Tokio Runtime");
    assert_eq!(h1.children.len(), 2);
    assert_eq!(h1.children[0].title, "Creating a Runtime");
    // Code block should be present.
    let has_code = h1.children[0]
        .blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Code { lang: Some(l), .. } if l == "rust"));
    assert!(has_code, "expected rust code block");
}

// ── AsciidocParser ────────────────────────────────────────────────────────────

#[test]
fn test_asciidoc_realistic_doc() {
    let adoc = r#"= Quarkus Guide

A brief intro paragraph.

== Configuration

Configure your application with:

[source,properties]
----
quarkus.http.port=8080
----

=== Environment Variables

You can also use environment variables.
"#;
    let doc = AsciidocParser.parse(adoc, Path::new("guide.adoc")).unwrap();
    assert_eq!(doc.title.as_deref(), Some("Quarkus Guide"));
    let h2 = doc.root.children.iter().find(|n| n.title == "Configuration");
    assert!(h2.is_some(), "expected Configuration section");
    let h2 = h2.unwrap();
    let code = h2.blocks.iter().find(|b| matches!(b, ContentBlock::Code { .. }));
    assert!(code.is_some());
    if let Some(ContentBlock::Code { lang, .. }) = code {
        assert_eq!(lang.as_deref(), Some("properties"));
    }
    assert_eq!(h2.children.len(), 1);
    assert_eq!(h2.children[0].title, "Environment Variables");
}

// ── RstParser ────────────────────────────────────────────────────────────────

#[test]
fn test_rst_realistic_doc() {
    let rst = r#"Welcome to Sphinx
=================

Sphinx is a documentation tool.

Getting Started
---------------

Install with pip:

.. code-block:: bash

    pip install sphinx

Then run ``sphinx-quickstart``.
"#;
    let doc = RstParser.parse(rst, Path::new("doc.rst")).unwrap();
    let h1 = doc.root.children.iter().find(|n| n.title == "Welcome to Sphinx");
    assert!(h1.is_some());
    let h1 = h1.unwrap();
    let h2 = h1.children.iter().find(|n| n.title == "Getting Started");
    assert!(h2.is_some());
    let h2 = h2.unwrap();
    let code = h2.blocks.iter().find(|b| matches!(b, ContentBlock::Code { .. }));
    assert!(code.is_some(), "expected code block in Getting Started");
    if let Some(ContentBlock::Code { lang, content }) = code {
        assert_eq!(lang.as_deref(), Some("bash"));
        assert!(content.contains("sphinx"));
    }
}
