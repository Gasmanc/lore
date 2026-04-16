//! Parser trait and document AST types used throughout the build pipeline.
//!
//! The parser layer converts raw file content into a [`ParsedDoc`] — a tree of
//! [`HeadingNode`]s where each node owns a list of [`ContentBlock`]s (prose,
//! code, tables) plus child headings.  The [`ParserRegistry`] selects the
//! correct parser by file extension.

use std::path::Path;

use lore_core::LoreError;

pub mod asciidoc;
pub mod html;
pub mod markdown;
pub mod rst;

pub use asciidoc::AsciidocParser;
pub use html::HtmlParser;
pub use markdown::MarkdownParser;
pub use rst::RstParser;

// ── Content AST ──────────────────────────────────────────────────────────────

/// A single block of content attached to a heading node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentBlock {
    /// One or more prose paragraphs as plain text.
    Paragraph(String),
    /// A fenced code block with optional language annotation.
    Code {
        /// Programming language identifier, e.g. `"rust"`.
        lang: Option<String>,
        /// The verbatim code content.
        content: String,
    },
    /// A Markdown/AsciiDoc table rendered as a raw string.
    Table(String),
    /// Any other block content (admonitions, directives, etc.).
    Other(String),
}

impl ContentBlock {
    /// Returns the text content for token-counting purposes.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Paragraph(s) | Self::Table(s) | Self::Other(s) => s,
            Self::Code { content, .. } => content,
        }
    }
}

/// A heading node in the document AST.
///
/// Each node represents one heading (level 1–6) and owns all content blocks
/// that appear between that heading and the next heading of equal or higher
/// level, plus child headings (deeper levels).
#[derive(Debug, Clone, Default)]
pub struct HeadingNode {
    /// Heading level (1–6).  0 is used for the synthetic document root.
    pub level: u8,
    /// The text of the heading.  Empty string for the synthetic root.
    pub title: String,
    /// Content blocks directly beneath this heading.
    pub blocks: Vec<ContentBlock>,
    /// Child headings (level > `self.level`).
    pub children: Vec<Self>,
}

impl HeadingNode {
    /// Create a synthetic root node that acts as a container for the whole doc.
    #[must_use]
    pub fn root() -> Self {
        Self { level: 0, ..Self::default() }
    }

    /// Total number of content blocks in this node and all descendants.
    #[must_use]
    pub fn total_block_count(&self) -> usize {
        self.blocks.len() + self.children.iter().map(Self::total_block_count).sum::<usize>()
    }
}

/// The result of parsing a single document file.
#[derive(Debug, Clone)]
pub struct ParsedDoc {
    /// Document title, if one could be extracted (frontmatter, `<title>`, etc.).
    pub title: Option<String>,
    /// Synthetic root node; its `children` are the top-level headings.
    pub root: HeadingNode,
}

// ── Parser trait ─────────────────────────────────────────────────────────────

/// A format-specific document parser.
pub trait Parser: Send + Sync {
    /// Returns `true` if this parser handles the given file path.
    fn can_parse(&self, path: &Path) -> bool;

    /// Parse `content` into a [`ParsedDoc`].
    ///
    /// `path` is provided for error messages only; it is not read from disk.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::Parse`] on malformed input.
    fn parse(&self, content: &str, path: &Path) -> Result<ParsedDoc, LoreError>;
}

// ── ParserRegistry ────────────────────────────────────────────────────────────

/// Holds all available parsers and selects the correct one by file extension.
pub struct ParserRegistry {
    parsers: Vec<Box<dyn Parser>>,
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ParserRegistry {
    /// Construct a registry pre-loaded with all built-in parsers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parsers: vec![
                Box::new(MarkdownParser),
                Box::new(HtmlParser),
                Box::new(AsciidocParser),
                Box::new(RstParser),
            ],
        }
    }

    /// Parse `content` using the first parser that claims `path`.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::Parse`] if no parser handles the extension, or if
    /// the chosen parser returns an error.
    pub fn parse(&self, path: &Path, content: &str) -> Result<ParsedDoc, LoreError> {
        let parser = self
            .parsers
            .iter()
            .find(|p| p.can_parse(path))
            .ok_or_else(|| LoreError::Parse(format!("no parser for {}", path.display())))?;
        parser.parse(content, path)
    }
}

// ── detect_primary_heading_level ──────────────────────────────────────────────

/// Determine which heading level represents the primary topic unit.
///
/// Walks the heading tree, counts nodes at levels 2–4, and returns the
/// shallowest level where the average number of content blocks per node
/// exceeds `1.5`.  Defaults to `2` when the tree is flat or no level meets
/// the criterion.
#[must_use]
pub fn detect_primary_heading_level(root: &HeadingNode) -> u8 {
    // Collect (level → [block_count per node]) across the whole tree.
    let mut counts: [Vec<usize>; 5] = Default::default(); // index 0 unused; 1–4
    collect_counts(root, &mut counts);

    for level in 2u8..=4 {
        let nodes = &counts[level as usize];
        if nodes.is_empty() {
            continue;
        }
        // Precision loss is acceptable for a heuristic threshold comparison.
        #[allow(clippy::cast_precision_loss)]
        let avg = nodes.iter().sum::<usize>() as f64 / nodes.len() as f64;
        if avg > 1.5 {
            return level;
        }
    }
    2
}

fn collect_counts(node: &HeadingNode, counts: &mut [Vec<usize>; 5]) {
    for child in &node.children {
        let idx = child.level as usize;
        if (1..=4).contains(&idx) {
            counts[idx].push(child.blocks.len());
        }
        collect_counts(child, counts);
    }
}
