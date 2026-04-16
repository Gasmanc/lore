//! `AsciiDoc` parser.
//!
//! Line-based parser for `.adoc` and `.asciidoc` files.  No external
//! `AsciiDoc` library is required.  Handles document titles (`= Title`),
//! section headings (`== H2`, `=== H3`, …), and source code blocks
//! (`[source,lang]` + `----`).

use std::path::Path;

use lore_core::LoreError;

use super::{ContentBlock, HeadingNode, ParsedDoc, Parser};

/// Parses `AsciiDoc` files.
pub struct AsciidocParser;

impl Parser for AsciidocParser {
    fn can_parse(&self, path: &Path) -> bool {
        matches!(path.extension().and_then(|e| e.to_str()), Some("adoc" | "asciidoc"))
    }

    fn parse(&self, content: &str, _path: &Path) -> Result<ParsedDoc, LoreError> {
        Ok(parse_asciidoc(content))
    }
}

// ── Core logic ────────────────────────────────────────────────────────────────

fn parse_asciidoc(content: &str) -> ParsedDoc {
    let mut title: Option<String> = None;
    let mut root = HeadingNode::root();
    let mut stack: Vec<HeadingNode> = vec![];
    let mut paragraph_buf = String::new();
    let mut in_code = false;
    let mut code_lang: Option<String> = None;
    let mut code_buf = String::new();
    let mut pending_lang: Option<String> = None;

    macro_rules! flush_paragraph {
        () => {
            let text = paragraph_buf.trim().to_owned();
            paragraph_buf.clear();
            if !text.is_empty() {
                current_node(&mut stack, &mut root).blocks.push(ContentBlock::Paragraph(text));
            }
        };
    }

    for line in content.lines() {
        // ── Inside a source block ────────────────────────────────────────────
        if in_code {
            if line == "----" {
                in_code = false;
                let code_content = std::mem::take(&mut code_buf);
                if !code_content.trim().is_empty() {
                    current_node(&mut stack, &mut root)
                        .blocks
                        .push(ContentBlock::Code { lang: code_lang.take(), content: code_content });
                }
            } else {
                code_buf.push_str(line);
                code_buf.push('\n');
            }
            continue;
        }

        // ── Document title: `= Title` (only as very first non-blank line) ───
        if title.is_none() && stack.is_empty() {
            if let Some(t) = line.strip_prefix("= ") {
                if !line.starts_with("==") {
                    title = Some(t.trim().to_owned());
                    continue;
                }
            }
        }

        // ── Source block attribute: `[source,lang]` ──────────────────────────
        if line.starts_with("[source") {
            pending_lang = parse_source_lang(line);
            continue;
        }

        // ── Code block delimiter: `----` ─────────────────────────────────────
        if line == "----" {
            flush_paragraph!();
            in_code = true;
            code_lang = pending_lang.take();
            code_buf.clear();
            continue;
        }

        // ── Headings: `== H2`, `=== H3`, etc. ───────────────────────────────
        if let Some(heading) = parse_heading(line) {
            flush_paragraph!();
            pending_lang = None;
            while !stack.is_empty() && stack.last().is_some_and(|n| n.level >= heading.level) {
                let completed = stack.pop().unwrap();
                current_node(&mut stack, &mut root).children.push(completed);
            }
            stack.push(heading);
            continue;
        }

        // ── Blank line → paragraph separator ────────────────────────────────
        if line.trim().is_empty() {
            flush_paragraph!();
            pending_lang = None;
            continue;
        }

        // ── Regular text ─────────────────────────────────────────────────────
        if !paragraph_buf.is_empty() {
            paragraph_buf.push('\n');
        }
        paragraph_buf.push_str(line);
    }

    flush_paragraph!();

    // Collapse stack.
    while let Some(completed) = stack.pop() {
        current_node(&mut stack, &mut root).children.push(completed);
    }

    ParsedDoc { title, root }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn current_node<'a>(
    stack: &'a mut [HeadingNode],
    root: &'a mut HeadingNode,
) -> &'a mut HeadingNode {
    stack.last_mut().unwrap_or(root)
}

/// Parse an `AsciiDoc` heading line into a [`HeadingNode`].
///
/// `AsciiDoc` uses `==` for H2, `===` for H3, `====` for H4, etc.
/// (`=` alone is the document title, handled separately.)
fn parse_heading(line: &str) -> Option<HeadingNode> {
    if !line.starts_with("==") {
        return None;
    }
    let sep = line.find(' ').unwrap_or(line.len());
    let (equals, rest) = line.split_at(sep);
    if !equals.chars().all(|c| c == '=') || equals.len() < 2 {
        return None;
    }
    let title = rest.trim().to_owned();
    if title.is_empty() {
        return None;
    }
    // `==` (2 chars) → level 2, `===` (3 chars) → level 3, etc.
    // Heading levels beyond 6 are unlikely but safe to allow.
    #[allow(clippy::cast_possible_truncation)]
    let level = equals.len() as u8;
    Some(HeadingNode { level, title, ..HeadingNode::default() })
}

/// Extract the language from a `[source,lang]` attribute line.
fn parse_source_lang(line: &str) -> Option<String> {
    let inner = line.trim_start_matches('[').trim_end_matches(']');
    let mut parts = inner.splitn(3, ',');
    parts.next(); // "source"
    let lang = parts.next()?.trim().to_owned();
    if lang.is_empty() { None } else { Some(lang) }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> ParsedDoc {
        parse_asciidoc(s)
    }

    #[test]
    fn test_asciidoc_basic() {
        let doc = parse(
            "= My Document\n\nIntro paragraph.\n\n== Overview\n\nFirst section.\n\n=== Details\n\nSub-section content.\n",
        );
        assert_eq!(doc.title.as_deref(), Some("My Document"));
        assert_eq!(doc.root.children.len(), 1);
        let h2 = &doc.root.children[0];
        assert_eq!(h2.title, "Overview");
        assert_eq!(h2.level, 2);
        assert_eq!(h2.children.len(), 1);
        assert_eq!(h2.children[0].title, "Details");
        assert_eq!(h2.children[0].level, 3);
    }

    #[test]
    fn test_asciidoc_source_block() {
        let doc = parse(
            "== Usage\n\nHere is some code:\n\n[source,java]\n----\npublic class Foo {}\n----\n\nMore text.\n",
        );
        let h2 = &doc.root.children[0];
        let code_block = h2.blocks.iter().find(|b| matches!(b, ContentBlock::Code { .. }));
        assert!(code_block.is_some(), "expected a code block");
        if let Some(ContentBlock::Code { lang, content }) = code_block {
            assert_eq!(lang.as_deref(), Some("java"));
            assert!(content.contains("class Foo"));
        }
    }

    #[test]
    fn test_asciidoc_no_title() {
        let doc = parse("== Section\n\nContent.\n");
        assert!(doc.title.is_none());
        assert_eq!(doc.root.children.len(), 1);
    }
}
