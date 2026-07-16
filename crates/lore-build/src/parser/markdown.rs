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
        matches!(path.extension().and_then(|e| e.to_str()), Some("md" | "mdx" | "qmd" | "rmd"))
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
    let close = after_open.find("\n---\n").or_else(|| after_open.find("\n---\r\n"));

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
    /// Inside a list (bullet or ordered). Tight lists emit `Text` events with
    /// no enclosing `Paragraph`, so list text needs its own capture context or
    /// every bullet is silently dropped.
    List,
}

struct ParseState {
    ctx: Context,
    heading_level: u8,
    heading_text: String,
    paragraph_text: String,
    code_lang: Option<String>,
    code_text: String,
    table_text: String,
    /// Accumulated text for the list currently being parsed.
    list_text: String,
    /// Nesting depth of the current list; the buffer is flushed when it hits 0.
    list_depth: u32,
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
            list_text: String::new(),
            list_depth: 0,
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

    /// Flush accumulated list text to `node` as a [`ContentBlock::Paragraph`].
    fn flush_list(&mut self, node: &mut HeadingNode) {
        self.ctx = Context::None;
        let text = std::mem::take(&mut self.list_text);
        let text = strip_jsx(text.trim());
        if !text.is_empty() {
            node.blocks.push(ContentBlock::Paragraph(text));
        }
    }

    /// Flush whichever text buffer (paragraph or list) is currently active.
    fn flush_text(&mut self, node: &mut HeadingNode) {
        match self.ctx {
            Context::Paragraph => self.flush_paragraph(node),
            Context::List => self.flush_list(node),
            _ => self.ctx = Context::None,
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
    // The root is held separately and never pushed onto `stack`, so the
    // "current node" is always `stack.last_mut().unwrap_or(&mut root)` — no
    // `unwrap()` can ever fail, satisfying the no-unwrap policy.
    let mut root = HeadingNode::root();
    let mut stack: Vec<HeadingNode> = Vec::new();
    let mut s = ParseState::new();

    for event in parser {
        match event {
            // ── Headings ───────────────────────────────────────────────────
            Event::Start(Tag::Heading { level, .. }) => {
                s.flush_text(current(&mut stack, &mut root));
                s.ctx = Context::Heading;
                s.heading_level = heading_level_to_u8(level);
                s.heading_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let title = strip_jsx(s.heading_text.trim());
                let new_node =
                    HeadingNode { level: s.heading_level, title, ..HeadingNode::default() };
                // Pop completed siblings/ancestors until we find a proper parent.
                while stack.last().is_some_and(|n| n.level >= s.heading_level) {
                    let Some(completed) = stack.pop() else { break };
                    current(&mut stack, &mut root).children.push(completed);
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
                s.flush_paragraph(current(&mut stack, &mut root));
            }

            // ── Lists (tight or loose) ─────────────────────────────────────
            Event::Start(Tag::List(_)) => {
                if s.list_depth == 0 {
                    s.flush_text(current(&mut stack, &mut root));
                    s.ctx = Context::List;
                    s.list_text.clear();
                }
                s.list_depth = s.list_depth.saturating_add(1);
            }
            Event::Start(Tag::Item) if s.ctx == Context::List => {
                if !s.list_text.is_empty() {
                    s.list_text.push('\n');
                }
            }
            Event::End(TagEnd::List(_)) => {
                s.list_depth = s.list_depth.saturating_sub(1);
                if s.list_depth == 0 {
                    s.flush_list(current(&mut stack, &mut root));
                }
            }

            // ── Code blocks ────────────────────────────────────────────────
            Event::Start(Tag::CodeBlock(kind)) => {
                s.flush_text(current(&mut stack, &mut root));
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
                let content = std::mem::take(&mut s.code_text);
                if !content.trim().is_empty() {
                    current(&mut stack, &mut root)
                        .blocks
                        .push(ContentBlock::Code { lang: s.code_lang.take(), content });
                }
                // Resume list accumulation if the code block was nested in a list.
                s.ctx = if s.list_depth > 0 { Context::List } else { Context::None };
            }

            // ── Tables ─────────────────────────────────────────────────────
            Event::Start(Tag::Table(_)) => {
                s.flush_text(current(&mut stack, &mut root));
                s.ctx = Context::Table;
                s.table_text.clear();
            }
            Event::End(TagEnd::Table) => {
                let content = std::mem::take(&mut s.table_text);
                if !content.trim().is_empty() {
                    current(&mut stack, &mut root).blocks.push(ContentBlock::Table(content));
                }
                s.ctx = if s.list_depth > 0 { Context::List } else { Context::None };
            }

            // ── Text ───────────────────────────────────────────────────────
            // `InlineHtml`/`Html` are captured too: pulldown-cmark parses `<T>`,
            // `<String>` (and MDX tags) as HTML events, so dropping them would
            // corrupt generic-type mentions. `strip_jsx` then removes only
            // genuine component tags from the assembled text.
            Event::Text(text) | Event::Code(text) | Event::InlineHtml(text) | Event::Html(text) => {
                let t = text.as_ref();
                match s.ctx {
                    Context::Heading => s.heading_text.push_str(t),
                    Context::Code => s.code_text.push_str(t),
                    Context::Table => {
                        s.table_text.push_str(t);
                        s.table_text.push(' ');
                    }
                    Context::Paragraph => s.paragraph_text.push_str(t),
                    Context::List => s.list_text.push_str(t),
                    Context::None => {}
                }
            }
            Event::SoftBreak | Event::HardBreak => match s.ctx {
                Context::Paragraph => s.paragraph_text.push('\n'),
                Context::List => s.list_text.push(' '),
                _ => {}
            },

            _ => {}
        }
    }

    s.flush_text(current(&mut stack, &mut root));
    while let Some(completed) = stack.pop() {
        current(&mut stack, &mut root).children.push(completed);
    }
    root
}

/// The heading node currently being filled: the deepest open heading on
/// `stack`, or `root` when no heading is open. Never panics.
fn current<'a>(stack: &'a mut [HeadingNode], root: &'a mut HeadingNode) -> &'a mut HeadingNode {
    stack.last_mut().unwrap_or(root)
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

/// Remove MDX JSX component tags like `<AppOnly>`, `</AppOnly>`,
/// `<Callout type="info">`, while preserving generic-type syntax that merely
/// *looks* like a tag.
///
/// A `<…>` run is only stripped when all of the following hold:
/// * the `<` is not immediately preceded by an identifier character — so
///   `Vec<String>`, `Option<T>`, and `HashMap<K, V>` are left intact;
/// * the tag name (after an optional `/`) starts with an uppercase letter and
///   is **at least two characters** long — so bare generic parameters like
///   `<T>`, `<E>`, `<K>` are left intact;
/// * a closing `>` exists.
///
/// MDX component names are `PascalCase` and ≥ 2 characters, so this keeps the
/// intended behaviour while no longer corrupting Rust/TypeScript generics that
/// pervade the docs this tool indexes.
fn strip_jsx(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' {
            let start = i + 1;
            let is_closing = start < len && bytes[start] == b'/';
            let name_start = if is_closing { start + 1 } else { start };
            // The identifier-guard (skip `Vec<…>` generics) applies only to
            // opening tags — a closing tag such as `Note</Callout>` legitimately
            // abuts preceding text and must still be stripped.
            let generic_open = !is_closing && preceded_by_identifier(bytes, i);
            // Tag name must start uppercase and be ≥ 2 ASCII-alphanumeric chars.
            let name_len =
                bytes[name_start..].iter().take_while(|b| b.is_ascii_alphanumeric()).count();
            let looks_like_tag = !generic_open
                && name_start < len
                && bytes[name_start].is_ascii_uppercase()
                && name_len >= 2;
            if looks_like_tag {
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

/// Returns `true` if the byte before position `i` is an identifier character
/// (ASCII alphanumeric or `_`), i.e. the `<` at `i` is a generic like `Vec<…>`
/// rather than the start of a tag.
fn preceded_by_identifier(bytes: &[u8], i: usize) -> bool {
    i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_')
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
        let md = "---\ntitle: My Great Doc\nauthor: Alice\n---\n\n# Heading\n\nContent.\n";
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
        let md = "## Parent\n\nParent content.\n\n### Child\n\nChild content.\n";
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

    #[test]
    fn test_tight_list_content_is_captured() {
        // Regression: tight (single-line) bullet items previously produced no
        // Paragraph events and were silently dropped from the index.
        let md = "## Features\n\n- fast startup times\n- zero configuration\n- async support\n\nTrailing paragraph.\n";
        let doc = parse(md);
        let section = &doc.root.children[0];
        let combined: String =
            section.blocks.iter().map(super::ContentBlock::text).collect::<Vec<_>>().join("\n");
        assert!(combined.contains("fast startup times"), "bullet 1 lost: {combined:?}");
        assert!(combined.contains("zero configuration"), "bullet 2 lost: {combined:?}");
        assert!(combined.contains("async support"), "bullet 3 lost: {combined:?}");
        assert!(combined.contains("Trailing paragraph."));
    }

    #[test]
    fn test_generics_not_stripped_as_jsx() {
        // Regression: `strip_jsx` used to delete `<T>`, `<String>` runs,
        // corrupting generic-type mentions ubiquitous in Rust/TS docs.
        let md = "## API\n\nUse Option<T> and Vec<String> where <T> is a generic parameter.\n";
        let doc = parse(md);
        let combined: String =
            doc.root.children[0].blocks.iter().map(super::ContentBlock::text).collect();
        assert!(combined.contains("Option<T>"), "Option<T> corrupted: {combined:?}");
        assert!(combined.contains("Vec<String>"), "Vec<String> corrupted: {combined:?}");
        assert!(combined.contains("<T>"), "bare <T> corrupted: {combined:?}");
    }

    #[test]
    fn test_mdx_component_still_stripped() {
        // The genuine MDX-tag case must still be stripped.
        assert_eq!(strip_jsx("<AppOnly>Inside.</AppOnly>"), "Inside.");
        assert_eq!(strip_jsx("<Callout type=\"info\">Note</Callout>"), "Note");
    }
}
