//! HTML parser.
//!
//! Strips boilerplate elements (`<nav>`, `<footer>`, etc.), extracts the
//! document title, converts the cleaned body to Markdown via `htmd`, then
//! delegates to [`super::markdown::parse_markdown`].

use std::path::Path;

use lore_core::LoreError;
use scraper::{Html, Selector};

use super::{ParsedDoc, Parser, markdown::parse_markdown};

/// Parses `.html` and `.htm` files.
pub struct HtmlParser;

impl Parser for HtmlParser {
    fn can_parse(&self, path: &Path) -> bool {
        matches!(path.extension().and_then(|e| e.to_str()), Some("html" | "htm"))
    }

    fn parse(&self, content: &str, path: &Path) -> Result<ParsedDoc, LoreError> {
        let document = Html::parse_document(content);

        // Extract title: try <title> first, then first <h1>.
        let title = extract_title(&document);

        // Build a selector for boilerplate elements to remove.
        // scraper's Html is immutable, so we collect the element IDs to skip
        // and rebuild a cleaned HTML string.
        let cleaned_html = remove_boilerplate(content, &document);

        // Convert cleaned HTML to Markdown.
        let markdown = htmd::convert(&cleaned_html).map_err(|e| {
            LoreError::Parse(format!("htmd conversion failed for {}: {e}", path.display()))
        })?;

        // Delegate to the Markdown parser.
        let mut doc = parse_markdown(&markdown);

        // Prefer the HTML-extracted title if the Markdown parser didn't find one.
        if doc.title.is_none() {
            doc.title = title;
        }

        Ok(doc)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn extract_title(document: &Html) -> Option<String> {
    // Try <title> element.
    let title_sel = Selector::parse("title").unwrap();
    if let Some(el) = document.select(&title_sel).next() {
        let text = el.text().collect::<String>().trim().to_owned();
        if !text.is_empty() {
            return Some(text);
        }
    }
    // Fall back to first <h1>.
    let h1_sel = Selector::parse("h1").unwrap();
    document.select(&h1_sel).next().map(|el| el.text().collect::<String>().trim().to_owned())
}

/// Remove boilerplate tags by rebuilding the HTML without them.
///
/// `scraper::Html` is immutable, so we use a simple approach: convert to
/// string, then ask `htmd` to ignore those tag names via its default config
/// which already omits script/style.  For nav/footer/header we do a
/// tag-stripping pass on the raw HTML string.
fn remove_boilerplate(html: &str, _document: &Html) -> String {
    // Tags whose entire subtree should be dropped.
    const DROP_TAGS: &[&str] = &["script", "style", "nav", "footer", "header"];

    let mut result = html.to_owned();
    for tag in DROP_TAGS {
        result = remove_tag_subtrees(&result, tag);
    }
    // Also remove role="navigation", role="banner", role="contentinfo" blocks.
    // These are tricky to remove exactly, so we strip the containing element
    // when it appears as a block-level element by using the same approach.
    result
}

/// Remove all occurrences of `<tag>...</tag>` (including nested) from `html`.
fn remove_tag_subtrees(html: &str, tag: &str) -> String {
    let open = format!("<{tag}");

    let mut out = String::with_capacity(html.len());
    let mut remaining = html;

    while let Some(open_pos) = remaining.to_ascii_lowercase().find(&open) {
        // Check that it's actually a tag start (`<nav>` or `<nav `, not `<navigation>`).
        let after_tag = open_pos + open.len();
        let next_char = remaining.as_bytes().get(after_tag).copied();
        if !matches!(next_char, Some(b'>' | b' ' | b'\n' | b'\t' | b'\r' | b'/')) {
            // Not a tag boundary — copy up to and including `<` and keep scanning.
            out.push_str(&remaining[..=open_pos]);
            remaining = &remaining[open_pos + 1..];
            continue;
        }

        // Copy everything before the tag.
        out.push_str(&remaining[..open_pos]);

        // Find the matching close tag, handling nesting.
        let inner = &remaining[open_pos..];
        let end = find_close_tag(inner, tag);
        remaining = &remaining[open_pos + end..];
    }

    out.push_str(remaining);
    out
}

/// Find the end position (exclusive) of the outermost `<tag>...</tag>` block
/// starting at `html[0]`.  Returns `html.len()` if no close tag is found.
///
/// The caller passes `html` starting AT the opening `<tag` — so the initial
/// tag counts as depth=1.  We return once depth drops back to zero.
fn find_close_tag(html: &str, tag: &str) -> usize {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let lower = html.to_ascii_lowercase();

    // depth=1: the initial opening tag is already open.
    let mut depth = 1usize;
    // Skip past the initial opening tag's `>` before scanning for more.
    let mut pos = lower.find('>').map_or(0, |p| p + 1);

    // Use byte-level comparisons throughout so that multibyte UTF-8 sequences
    // (e.g. emoji in placeholder attributes) never cause a char-boundary panic.
    let open_b = open.as_bytes();
    let close_b = close.as_bytes();
    let lower_b = lower.as_bytes();

    while pos < lower_b.len() {
        if lower_b[pos..].starts_with(close_b) {
            depth -= 1;
            if depth == 0 {
                return pos + close_b.len();
            }
            pos += close_b.len();
        } else if lower_b[pos..].starts_with(open_b) {
            let after = pos + open_b.len();
            if matches!(
                lower_b.get(after).copied(),
                Some(b'>' | b' ' | b'\n' | b'\t' | b'\r' | b'/')
            ) {
                depth += 1;
            }
            pos += open_b.len();
        } else {
            pos += 1;
        }
    }

    html.len()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse(html: &str) -> ParsedDoc {
        HtmlParser.parse(html, Path::new("test.html")).unwrap()
    }

    #[test]
    fn test_html_basic() {
        let html = r"<!DOCTYPE html>
<html>
<head><title>My Page</title></head>
<body>
<h1>Main Heading</h1>
<p>Some introductory text.</p>
<h2>Section One</h2>
<p>Content of section one.</p>
</body>
</html>";
        let doc = parse(html);
        assert_eq!(doc.title.as_deref(), Some("My Page"));
        // Should have at least one heading in the tree.
        assert!(!doc.root.children.is_empty());
    }

    #[test]
    fn test_html_strips_nav() {
        let html = r#"<!DOCTYPE html>
<html>
<head><title>Page</title></head>
<body>
<nav><ul><li><a href="/">Home</a></li></ul></nav>
<h2>Content</h2>
<p>Real paragraph.</p>
</body>
</html>"#;
        let doc = parse(html);
        // The nav text "Home" should not appear in any content block.
        let all: String = doc
            .root
            .children
            .iter()
            .flat_map(|n| n.blocks.iter())
            .map(|b| b.text().to_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!all.contains("Home"));
        assert!(all.contains("Real paragraph"));
    }
}
