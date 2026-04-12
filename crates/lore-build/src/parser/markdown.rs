//! Markdown parser using `pulldown-cmark`.
//!
//! Handles `.md`, `.mdx`, `.qmd`, and `.rmd` files.  Extracts YAML
//! frontmatter for the document title, strips MDX JSX tags, and skips
//! `ToC` sections.

use std::path::Path;

use lore_core::LoreError;
use pulldown_cmark::{
    CodeBlockKind, Event, HeadingLevel, Options, Parser as CmarkParser, Tag, TagEnd,
};

use super::{ContentBlock, HeadingNode, ParsedDoc, Parser};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Title keywords that identify a table-of-contents heading (case-insensitive).
const TOC_TITLES: &[&str] =
    &["table of contents", "contents", "toc", "on this page", "in this article"];

// ── Public parser struct ──────────────────────────────────────────────────────

/// Parses Markdown (and MDX/Quarto/R Markdown) files.
pub struct MarkdownParser;

impl Parser for MarkdownParser {
    fn can_parse(&self, path: &Path) -> bool {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md" | "mdx" | "qmd" | "rmd")
        )
    }

    fn parse(&self, content: &str, _path: &Path) -> Result<ParsedDoc, LoreError> {
        Ok(parse_markdown(content))
    }
}

// ── Core parsing logic (also called by HtmlParser) ───────────────────────────

/// Parse a Markdown string into a [`ParsedDoc`].
///
/// This is `pub(crate)` so `HtmlParser` can reuse it after HTML→Markdown
/// conversion.
pub(crate) fn parse_markdown(content: &str) -> ParsedDoc {
    let (title_from_fm, md_content) = extract_frontmatter(content);
    let root = build_tree(md_content);
    let root = strip_toc(root);
    ParsedDoc { title: title_from_fm, root }
}

// ── Frontmatter ───────────────────────────────────────────────────────────────

/// If `content` begins with a `---\n` block, parse YAML key-values and return
/// the `title` field (if any) plus the content with the frontmatter removed.
fn extract_frontmatter(content: &str) -> (Option<String>, &str) {
    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        return (None, content);
    }

    let after_open = &content[4..];
    let close = after_open
        .find("\n---\n")
        .or_else(|| after_open.find("\n---\r\n"));

    let Some(close_pos) = close else {
        return (None, content);
    };

    let yaml_block = &after_open[..close_pos];
    let advance = if after_open[close_pos..].starts_with("\n---\r\n") { 6 } else { 5 };
    let rest = content.get(4 + close_pos + advance..).unwrap_or("");

    let title = yaml_block.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix("title:")?;
        let val = rest.trim().trim_matches('"').trim_matches('\'').to_owned();
        if val.is_empty() { None } else { Some(val) }
    });

    (title, rest)
}

// ── Parse state ───────────────────────────────────────────────────────────────

/// Tracks which block element the parser is currently inside.
#[derive(Default, PartialEq, Eq)]
enum Context {
    /// Between block elements.
    #[default]
    None,
    /// Inside a heading tag.
    Heading,
    /// Inside a paragraph tag.
    Paragraph,
    /// Inside a fenced/indented code block.
    Code,
    /// Inside a table.
    Table,
}

struct ParseState {
    ctx: Context,
    heading_level: u8,
    heading_text: String,
    paragraph_text: String,
    code_lang: Option<String>,
    code_text: String,
    table_text: String,
}

impl ParseState {
    const fn new() -> Self {
        Self {
            ctx: Context::None,
            heading_level: 0,
            heading_text: String::new(),
            paragraph_text: String::new(),
            code_lang: None,
            code_text: String::new(),
            table_text: String::new(),
        }
    }

    /// Flush any buffered paragraph text to `node` as a [`ContentBlock::Paragraph`].
    fn flush_paragraph(&mut self, node: &mut HeadingNode) {
        self.ctx = Context::None;
        let text = std::mem::take(&mut self.paragraph_text);
        let text = strip_jsx(text.trim());
        if !text.is_empty() {
            node.blocks.push(ContentBlock::Paragraph(text));
        }
    }
}

// ── Tree builder ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)] // one match arm per pulldown-cmark event type
fn build_tree(content: &str) -> HeadingNode {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = CmarkParser::new_ext(content, opts);
    let mut stack: Vec<HeadingNode> = vec![HeadingNode::root()];
    let mut s = ParseState::new();

    for event in parser {
        match event {
            // ── Headings ───────────────────────────────────────────────────
            Event::Start(Tag::Heading { level, .. }) => {
                s.flush_paragraph(stack.last_mut().unwrap());
                s.ctx = Context::Heading;
                s.heading_level = heading_level_to_u8(level);
                s.heading_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let title = strip_jsx(s.heading_text.trim());
                let new_node =
                    HeadingNode { level: s.heading_level, title, ..HeadingNode::default() };
                // Pop stack until we find a proper parent (lower level).
                while stack.len() > 1
                    && stack.last().is_some_and(|n| n.level >= s.heading_level)
                {
                    let completed = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(completed);
                }
                stack.push(new_node);
                s.ctx = Context::None;
                s.heading_text.clear();
            }

            // ── Paragraphs ─────────────────────────────────────────────────
            Event::Start(Tag::Paragraph) if s.ctx == Context::None => {
                s.ctx = Context::Paragraph;
                s.paragraph_text.clear();
            }
            Event::End(TagEnd::Paragraph) if s.ctx == Context::Paragraph => {
                s.flush_paragraph(stack.last_mut().unwrap());
            }

            // ── Code blocks ────────────────────────────────────────────────
            Event::Start(Tag::CodeBlock(kind)) => {
                s.flush_paragraph(stack.last_mut().unwrap());
                s.ctx = Context::Code;
                s.code_lang = match &kind {
                    CodeBlockKind::Fenced(lang) => {
                        let l = lang.trim().to_owned();
                        if l.is_empty() { None } else { Some(l) }
                    }
                    CodeBlockKind::Indented => None,
                };
                s.code_text.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                s.ctx = Context::None;
                let content = std::mem::take(&mut s.code_text);
                if !content.trim().is_empty() {
                    stack.last_mut().unwrap().blocks.push(ContentBlock::Code {
                        lang: s.code_lang.take(),
                        content,
                    });
                }
            }

            // ── Tables ─────────────────────────────────────────────────────
            Event::Start(Tag::Table(_)) => {
                s.flush_paragraph(stack.last_mut().unwrap());
                s.ctx = Context::Table;
                s.table_text.clear();
            }
            Event::End(TagEnd::Table) => {
                s.ctx = Context::None;
                let content = std::mem::take(&mut s.table_text);
                if !content.trim().is_empty() {
                    stack.last_mut().unwrap().blocks.push(ContentBlock::Table(content));
                }
            }

            // ── Text ───────────────────────────────────────────────────────
            Event::Text(text) | Event::Code(text) => {
                let t = text.as_ref();
                match s.ctx {
                    Context::Heading => s.heading_text.push_str(t),
                    Context::Code => s.code_text.push_str(t),
                    Context::Table => {
                        s.table_text.push_str(t);
                        s.table_text.push(' ');
                    }
                    Context::Paragraph => s.paragraph_text.push_str(t),
                    Context::None => {}
                }
            }
            Event::SoftBreak | Event::HardBreak if s.ctx == Context::Paragraph => {
                s.paragraph_text.push('\n');
            }

            _ => {}
        }
    }

    s.flush_paragraph(stack.last_mut().unwrap());
    while stack.len() > 1 {
        let completed = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(completed);
    }
    stack.pop().unwrap()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Remove MDX JSX tags like `<AppOnly>`, `</AppOnly>`, `<Callout type="info">`.
/// Only uppercase-starting tags are stripped (to avoid stripping HTML entities).
fn strip_jsx(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' {
            let start = i + 1;
            let name_start =
                if start < len && bytes[start] == b'/' { start + 1 } else { start };
            if name_start < len && bytes[name_start].is_ascii_uppercase() {
                if let Some(rel) = bytes[i..].iter().position(|&b| b == b'>') {
                    i += rel + 1;
                    continue;
                }
            }
        }
        // Copy the full UTF-8 character starting at byte `i`.
        let ch_end = text[i..].chars().next().map_or(i + 1, |c| i + c.len_utf8());
        out.push_str(&text[i..ch_end]);
        i = ch_end;
    }

    out
}

/// Walk the tree and remove `ToC` heading nodes.
fn strip_toc(mut root: HeadingNode) -> HeadingNode {
    root.children.retain(|child| !is_toc_node(child));
    root.children = root.children.into_iter().map(strip_toc).collect();
    root
}

fn is_toc_node(node: &HeadingNode) -> bool {
    let lower = node.title.to_lowercase();
    if TOC_TITLES.contains(&lower.as_str()) {
        return true;
    }
    let all_text: String = node
        .blocks
        .iter()
        .filter_map(|b| if let ContentBlock::Paragraph(s) = b { Some(s.as_str()) } else { None })
        .collect::<Vec<_>>()
        .join("\n");

    if all_text.is_empty() {
        return false;
    }

    let link_chars = count_markdown_link_chars(&all_text);
    // Precision loss is acceptable for a heuristic link-density check.
    #[allow(clippy::cast_precision_loss)]
    let density = link_chars as f64 / all_text.len() as f64;
    density > 0.6
}

/// Count characters that are part of Markdown link syntax `[text](url)`.
fn count_markdown_link_chars(text: &str) -> usize {
    let mut total = 0usize;
    let mut search_from = 0usize;

    while let Some(open) = text[search_from..].find('[') {
        let abs_open = search_from + open;
        let after_open = abs_open + 1;

        let Some(close_bracket_rel) = text[after_open..].find("](") else {
            break;
        };
        let abs_link_start = after_open + close_bracket_rel + 2;

        let Some(close_paren_rel) = text[abs_link_start..].find(')') else {
            break;
        };

        total += 1 + close_bracket_rel + 2 + close_paren_rel + 1;
        search_from = abs_link_start + close_paren_rel + 1;
    }

    total
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> ParsedDoc {
        parse_markdown(s)
    }

    #[test]
    fn test_basic_markdown() {
        let md = "# Doc\n\nIntro paragraph.\n\n## Section One\n\nFirst section content.\n\n## Section Two\n\nSecond section content.\n";
        let doc = parse(md);
        assert_eq!(doc.root.children.len(), 1);
        let h1 = &doc.root.children[0];
        assert_eq!(h1.title, "Doc");
        assert_eq!(h1.blocks.len(), 1);
        assert_eq!(h1.children.len(), 2);
        assert_eq!(h1.children[0].title, "Section One");
        assert_eq!(h1.children[1].title, "Section Two");
    }

    #[test]
    fn test_frontmatter() {
        let md =
            "---\ntitle: My Great Doc\nauthor: Alice\n---\n\n# Heading\n\nContent.\n";
        let doc = parse(md);
        assert_eq!(doc.title.as_deref(), Some("My Great Doc"));
        assert_eq!(doc.root.children.len(), 1);
        assert_eq!(doc.root.children[0].title, "Heading");
    }

    #[test]
    fn test_code_block() {
        let md = "# Doc\n\n```rust\nfn main() {}\n```\n";
        let doc = parse(md);
        let h1 = &doc.root.children[0];
        assert_eq!(h1.blocks.len(), 1);
        match &h1.blocks[0] {
            ContentBlock::Code { lang, content } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert!(content.contains("fn main"));
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn test_nested_headings() {
        let md =
            "## Parent\n\nParent content.\n\n### Child\n\nChild content.\n";
        let doc = parse(md);
        let h2 = &doc.root.children[0];
        assert_eq!(h2.title, "Parent");
        assert_eq!(h2.children.len(), 1);
        assert_eq!(h2.children[0].title, "Child");
    }

    #[test]
    fn test_mdx_tag_stripping() {
        let md = "## Section\n\n<AppOnly>Inside JSX.</AppOnly>\n\nRegular text.\n";
        let doc = parse(md);
        let h2 = &doc.root.children[0];
        let combined: String =
            h2.blocks.iter().map(super::ContentBlock::text).collect::<Vec<_>>().join(" ");
        assert!(combined.contains("Inside JSX."));
        assert!(!combined.contains("<AppOnly>"));
    }

    #[test]
    fn test_toc_skipped() {
        let md = "## Table of Contents\n\n- [Section One](#s1)\n- [Section Two](#s2)\n\n## Section One\n\nReal content.\n";
        let doc = parse(md);
        assert!(doc.root.children.iter().all(|n| n.title != "Table of Contents"));
        assert!(doc.root.children.iter().any(|n| n.title == "Section One"));
    }
}
