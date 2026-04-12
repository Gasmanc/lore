//! reStructuredText parser.
//!
//! Line-based parser for `.rst` files.  Detects headings by underline
//! patterns (first underline character seen = H1, second = H2, etc.) and
//! `.. code-block::` / `.. code::` directives.

use std::path::Path;

use lore_core::LoreError;

use super::{ContentBlock, HeadingNode, ParsedDoc, Parser};

/// Parses reStructuredText files.
pub struct RstParser;

impl Parser for RstParser {
    fn can_parse(&self, path: &Path) -> bool {
        matches!(path.extension().and_then(|e| e.to_str()), Some("rst"))
    }

    fn parse(&self, content: &str, _path: &Path) -> Result<ParsedDoc, LoreError> {
        Ok(parse_rst(content))
    }
}

// ── RST underline characters ──────────────────────────────────────────────────

const UNDERLINE_CHARS: &[char] = &['=', '-', '~', '^', '"', '#', '*', '+'];

fn is_underline_line(line: &str, min_len: usize) -> Option<char> {
    if line.is_empty() {
        return None;
    }
    let first = line.chars().next()?;
    if !UNDERLINE_CHARS.contains(&first) {
        return None;
    }
    if line.chars().all(|c| c == first) && line.len() >= min_len {
        Some(first)
    } else {
        None
    }
}

// ── Code directive enum ───────────────────────────────────────────────────────

/// The result of recognising a `.. code-block::` or `.. code::` directive.
enum CodeDirective {
    /// `.. code-block:: lang` or `.. code:: lang` with an explicit language.
    WithLang(String),
    /// `.. code-block::` or `.. code::` without a language specifier.
    Anonymous,
}

/// Parse `.. code-block:: lang` or `.. code:: lang` directive line.
fn parse_code_directive(line: &str) -> Option<CodeDirective> {
    let rest = line
        .strip_prefix(".. code-block::")
        .or_else(|| line.strip_prefix(".. code::"))?;
    let lang = rest.trim().to_owned();
    Some(if lang.is_empty() {
        CodeDirective::Anonymous
    } else {
        CodeDirective::WithLang(lang)
    })
}

// ── Core logic ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)] // inherent complexity of a line-based RST state machine
fn parse_rst(content: &str) -> ParsedDoc {
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len();

    // Maps underline chars (in order of first appearance) → heading level (1-based).
    let mut char_to_level: Vec<char> = vec![];

    let mut root = HeadingNode::root();
    let mut stack: Vec<HeadingNode> = vec![];
    let mut paragraph_buf = String::new();
    let mut in_code = false;
    let mut code_lang: Option<String> = None;
    let mut code_indent: usize = 0;
    let mut code_buf = String::new();
    let mut prev_heading_candidate: Option<&str> = None;

    macro_rules! flush_paragraph {
        () => {{
            let text = paragraph_buf.trim().to_owned();
            paragraph_buf.clear();
            if !text.is_empty() {
                current_node(&mut stack, &mut root)
                    .blocks
                    .push(ContentBlock::Paragraph(text));
            }
        }};
    }

    macro_rules! finish_code_block {
        () => {{
            in_code = false;
            let code_content = std::mem::take(&mut code_buf);
            if !code_content.trim().is_empty() {
                current_node(&mut stack, &mut root).blocks.push(ContentBlock::Code {
                    lang: code_lang.take(),
                    content: code_content,
                });
            }
        }};
    }

    let mut i = 0usize;
    while i < n {
        let line = lines[i];

        // ── Inside indented code block ───────────────────────────────────────
        if in_code {
            if line.trim().is_empty() {
                // Look ahead: if next non-blank line has less indent, block ends.
                let next_indent = lines[i + 1..]
                    .iter()
                    .find(|&&l| !l.trim().is_empty())
                    .map_or(0, |l| l.len() - l.trim_start().len());
                if next_indent < code_indent {
                    finish_code_block!();
                    // Fall through to blank-line handling below.
                } else {
                    code_buf.push('\n');
                    i += 1;
                    continue;
                }
            } else {
                let indent = line.len() - line.trim_start().len();
                if indent < code_indent {
                    finish_code_block!();
                    // Fall through to normal processing of this line.
                } else {
                    code_buf.push_str(line.trim_start_matches(' '));
                    code_buf.push('\n');
                    i += 1;
                    continue;
                }
            }
        }

        // ── Heading detection ────────────────────────────────────────────────
        if let Some(candidate) = prev_heading_candidate {
            let min_len = candidate.trim().len();
            if let Some(ul_char) = is_underline_line(line.trim(), min_len) {
                flush_paragraph!();
                let level =
                    char_to_level.iter().position(|&c| c == ul_char).map_or_else(
                        || {
                            char_to_level.push(ul_char);
                            char_to_level.len()
                        },
                        |pos| pos + 1,
                    );
                // Heading levels beyond 6 are uncommon but technically valid RST.
                #[allow(clippy::cast_possible_truncation)]
                let level = level as u8;
                let heading =
                    HeadingNode { level, title: candidate.trim().to_owned(), ..HeadingNode::default() };
                while !stack.is_empty()
                    && stack.last().is_some_and(|n| n.level >= level)
                {
                    let completed = stack.pop().unwrap();
                    current_node(&mut stack, &mut root).children.push(completed);
                }
                stack.push(heading);
                prev_heading_candidate = None;
                i += 1;
                continue;
            }
        }

        let trimmed = line.trim();

        // ── Code block directive ─────────────────────────────────────────────
        if let Some(directive) = parse_code_directive(trimmed) {
            flush_paragraph!();
            in_code = true;
            code_lang = match directive {
                CodeDirective::WithLang(lang) => Some(lang),
                CodeDirective::Anonymous => None,
            };
            code_buf.clear();
            code_indent = lines[i + 1..]
                .iter()
                .find(|&&l| !l.trim().is_empty())
                .map_or(4, |l| l.len() - l.trim_start().len());
            prev_heading_candidate = None;
            i += 1;
            continue;
        }

        // ── Anonymous code block: paragraph ending with `::` ─────────────────
        if trimmed.ends_with("::") && !trimmed.starts_with("..") {
            let para_text = trimmed.trim_end_matches(':').trim().to_owned();
            if !para_text.is_empty() {
                paragraph_buf.push_str(&para_text);
                paragraph_buf.push('\n');
            }
            flush_paragraph!();
            in_code = true;
            code_lang = None;
            code_buf.clear();
            code_indent = lines[i + 1..]
                .iter()
                .find(|&&l| !l.trim().is_empty())
                .map_or(4, |l| l.len() - l.trim_start().len());
            prev_heading_candidate = None;
            i += 1;
            continue;
        }

        // ── Blank line ───────────────────────────────────────────────────────
        if trimmed.is_empty() {
            flush_paragraph!();
            prev_heading_candidate = None;
            i += 1;
            continue;
        }

        // ── Regular text — may be a heading candidate ────────────────────────
        let next_line = lines.get(i + 1).copied().unwrap_or("");
        if is_underline_line(next_line.trim(), trimmed.len()).is_some() {
            prev_heading_candidate = Some(line);
        } else {
            prev_heading_candidate = None;
            if !paragraph_buf.is_empty() {
                paragraph_buf.push('\n');
            }
            paragraph_buf.push_str(trimmed);
        }

        i += 1;
    }

    // Flush trailing content.
    if in_code {
        // Don't set in_code = false here — we're done; avoid an unused-assignment lint.
        let code_content = std::mem::take(&mut code_buf);
        if !code_content.trim().is_empty() {
            current_node(&mut stack, &mut root).blocks.push(ContentBlock::Code {
                lang: code_lang,
                content: code_content,
            });
        }
    } else {
        flush_paragraph!();
    }

    while let Some(completed) = stack.pop() {
        current_node(&mut stack, &mut root).children.push(completed);
    }

    let title = root.children.iter().find(|n| n.level == 1).map(|n| n.title.clone());
    ParsedDoc { title, root }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn current_node<'a>(
    stack: &'a mut [HeadingNode],
    root: &'a mut HeadingNode,
) -> &'a mut HeadingNode {
    stack.last_mut().unwrap_or(root)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> ParsedDoc {
        parse_rst(s)
    }

    #[test]
    fn test_rst_headings() {
        let rst =
            "Overview\n========\n\nIntro text.\n\nInstallation\n------------\n\nInstall info.\n";
        let doc = parse(rst);
        // `=` → H1, `-` → H2 (child of H1 because level 2 > level 1).
        assert_eq!(doc.root.children.len(), 1);
        let h1 = &doc.root.children[0];
        assert_eq!(h1.title, "Overview");
        assert_eq!(h1.level, 1);
        assert_eq!(h1.children.len(), 1);
        assert_eq!(h1.children[0].title, "Installation");
        assert_eq!(h1.children[0].level, 2);
    }

    #[test]
    fn test_rst_nested_headings() {
        let rst = "Guide\n=====\n\nIntro.\n\nChapter One\n-----------\n\nText.\n\nSection A\n~~~~~~~~~\n\nDeep content.\n";
        let doc = parse(rst);
        let h1 = &doc.root.children[0];
        assert_eq!(h1.title, "Guide");
        assert_eq!(h1.children.len(), 1);
        let h2 = &h1.children[0];
        assert_eq!(h2.title, "Chapter One");
        assert_eq!(h2.children.len(), 1);
        assert_eq!(h2.children[0].title, "Section A");
    }

    #[test]
    fn test_rst_code_block() {
        let rst = "Usage\n=====\n\nExample:\n\n.. code-block:: python\n\n    def hello():\n        print(\"hi\")\n\nMore text.\n";
        let doc = parse(rst);
        let h1 = &doc.root.children[0];
        let code = h1.blocks.iter().find(|b| matches!(b, ContentBlock::Code { .. }));
        assert!(code.is_some(), "expected a code block");
        if let Some(ContentBlock::Code { lang, content }) = code {
            assert_eq!(lang.as_deref(), Some("python"));
            assert!(content.contains("hello"));
        }
    }
}
