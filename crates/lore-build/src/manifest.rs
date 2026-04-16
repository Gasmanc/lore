//! Manifest generation for Lore packages.
//!
//! This module provides two distinct outputs:
//!
//! 1. **JSON sidecar** ([`write_manifest`]) — a `<stem>.json` file placed next
//!    to a `.db` file containing [`PackageMetadata`].  Used by the registry
//!    infrastructure to populate `index.json`.
//!
//! 2. **API surface manifest** ([`generate_api_manifest`]) — a compressed
//!    `~500 token` text index of a package's public API surface, derived from
//!    the heading tree and code-block signatures extracted from the database.
//!    Stored in the `meta` table under the key `"manifest"` and returned by
//!    the `get_manifest` MCP tool.  Suitable for pasting into `CLAUDE.md` as
//!    a fingerpost.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lore_core::{Db, LoreError, Node, NodeKind, Package, PackageMetadata};
use serde_json;

use crate::builder::BuildStats;
use crate::tokens::TokenCounter;

// ── JSON sidecar ──────────────────────────────────────────────────────────────

/// Write a `<stem>.json` manifest alongside `db_path`.
///
/// The manifest contains the package identity, build statistics (chunk count,
/// file size), and a UTC build timestamp in ISO 8601 format.
///
/// Returns the path to the written manifest file.
///
/// # Errors
///
/// Returns [`LoreError::Io`] if the manifest cannot be written, or
/// [`LoreError::Schema`] if JSON serialisation fails.
pub fn write_manifest(
    db_path: &Path,
    package: &Package,
    stats: &BuildStats,
) -> Result<PathBuf, LoreError> {
    let size_bytes = std::fs::metadata(db_path).ok().map(|m| m.len());

    let metadata = PackageMetadata {
        package: package.clone(),
        size_bytes,
        chunk_count: Some(stats.chunk_count + stats.code_block_count),
        build_date: Some(utc_now_iso8601()),
    };

    let json =
        serde_json::to_string_pretty(&metadata).map_err(|e| LoreError::Schema(e.to_string()))?;

    let manifest_path = db_path.with_extension("json");
    std::fs::write(&manifest_path, json.as_bytes()).map_err(LoreError::Io)?;
    Ok(manifest_path)
}

// ── API surface manifest ──────────────────────────────────────────────────────

/// Generates and stores the compressed API surface manifest in the database.
///
/// Extracts all heading titles and API signatures from code blocks, formats
/// them into a `~500 token` index, and stores the result in the `meta` table
/// under the key `"manifest"`.
///
/// # Format
///
/// ```text
/// CACHING: cacheLife(profile), cacheTag(...tags), revalidateTag(tag)
/// ROUTING: forbidden(), unauthorized(), after(callback)
/// METADATA: generateMetadata({params}), generateStaticParams()
/// ```
///
/// # Errors
///
/// Returns [`LoreError`] if the database cannot be read or written.
pub async fn generate_api_manifest(db: &Db) -> Result<String, LoreError> {
    let counter = TokenCounter::new().map_err(|e| LoreError::Embed(e.to_string()))?;

    let headings = extract_headings(db).await?;
    let signatures = extract_signatures(db).await?;

    let manifest = build_manifest_text(&headings, &signatures, &counter);
    db.set_meta("manifest".to_owned(), manifest.clone()).await?;
    Ok(manifest)
}

// ── Internal extraction ───────────────────────────────────────────────────────

/// A heading entry extracted from the database.
struct HeadingEntry {
    title: String,
    level: u8,
}

/// An API signature extracted from a code block.
struct ApiSignature {
    /// The top-level section name (first heading ancestor title).
    section: String,
    /// The extracted signature line (e.g. `"cacheLife(profile)"`).
    signature: String,
}

/// Returns all heading nodes ordered by id.
async fn extract_headings(db: &Db) -> Result<Vec<HeadingEntry>, LoreError> {
    let nodes: Vec<Node> = db.get_nodes_by_kind(NodeKind::Heading).await?;
    let entries = nodes
        .into_iter()
        .filter_map(|n| {
            let title = n.title.filter(|t| !t.trim().is_empty())?;
            let level = n.level?;
            Some(HeadingEntry { title, level })
        })
        .collect();
    Ok(entries)
}

/// Returns all API signatures found in code blocks.
async fn extract_signatures(db: &Db) -> Result<Vec<ApiSignature>, LoreError> {
    let nodes: Vec<Node> = db.get_nodes_by_kind(NodeKind::CodeBlock).await?;

    let supported_langs =
        ["js", "ts", "javascript", "typescript", "python", "rust", "go", "java", "jsx", "tsx"];

    let mut signatures = Vec::new();

    for node in &nodes {
        let lang = node.lang.as_deref().unwrap_or("");
        if !supported_langs.contains(&lang) {
            continue;
        }

        let content = match node.content.as_deref() {
            Some(c) if !c.is_empty() => c,
            _ => continue,
        };

        // Derive the top-level section from the heading path breadcrumb.
        let section = db
            .get_heading_path(node.id)
            .await
            .ok()
            .and_then(|path| path.into_iter().next())
            .unwrap_or_default();

        for line in content.lines() {
            if let Some(sig) = extract_signature_from_line(line) {
                signatures.push(ApiSignature { section: section.clone(), signature: sig });
            }
        }
    }

    Ok(signatures)
}

/// Extracts a function/class/type signature from a single line of code.
///
/// Returns `Some(signature)` if the line matches a known definition pattern.
fn extract_signature_from_line(line: &str) -> Option<String> {
    let line = line.trim();

    // JavaScript / TypeScript patterns
    let js_patterns = [
        "export default async function ",
        "export default function ",
        "export default class ",
        "export async function ",
        "export function ",
        "export const ",
        "export let ",
        "export class ",
        "export type ",
        "export interface ",
        "async function ",
        "function ",
        "const ",
        "class ",
        "type ",
        "interface ",
    ];

    for prefix in &js_patterns {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(extract_compact_sig(rest));
        }
    }

    // Python patterns
    if let Some(rest) = line.strip_prefix("async def ").or_else(|| line.strip_prefix("def ")) {
        return Some(extract_compact_sig(rest));
    }
    if let Some(rest) = line.strip_prefix("class ") {
        return Some(extract_compact_sig(rest));
    }

    // Rust patterns
    let rust_prefixes = [
        "pub async fn ",
        "pub fn ",
        "async fn ",
        "fn ",
        "pub struct ",
        "pub enum ",
        "pub trait ",
        "struct ",
        "enum ",
        "trait ",
    ];
    for prefix in &rust_prefixes {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(extract_compact_sig(rest));
        }
    }

    // Go patterns
    if let Some(rest) = line.strip_prefix("func ") {
        return Some(extract_compact_sig(rest));
    }

    None
}

/// Trims a definition line to produce a compact signature (max 60 chars).
fn extract_compact_sig(rest: &str) -> String {
    // Take up to the first `{` or `;`.
    let end = rest.find(['{', ';']).unwrap_or(rest.len());
    let sig = rest[..end].trim().to_owned();
    if sig.len() > 60 { format!("{}…", &sig[..57]) } else { sig }
}

// ── Manifest formatting ───────────────────────────────────────────────────────

/// Builds the compressed manifest string from headings and signatures.
///
/// Groups signatures by their top-level section.  Sections without signatures
/// are omitted.  If the total token count exceeds 500, signature lists are
/// trimmed progressively until it fits.
fn build_manifest_text(
    headings: &[HeadingEntry],
    signatures: &[ApiSignature],
    counter: &TokenCounter,
) -> String {
    // Collect unique top-level section names (level ≤ 2), preserving order.
    let mut seen = std::collections::HashSet::new();
    let top_sections: Vec<String> =
        headings
            .iter()
            .filter(|h| h.level <= 2)
            .filter_map(|h| {
                if seen.insert(h.title.to_lowercase()) { Some(h.title.clone()) } else { None }
            })
            .collect();

    // Group signatures by section (case-insensitive).
    let mut section_sigs: HashMap<String, Vec<String>> = HashMap::new();
    for sig in signatures {
        let matched = top_sections
            .iter()
            .find(|s| s.to_lowercase() == sig.section.to_lowercase())
            .cloned()
            .unwrap_or_else(|| sig.section.clone());
        let entry = section_sigs.entry(matched).or_default();
        if !sig.signature.is_empty() && !entry.contains(&sig.signature) {
            entry.push(sig.signature.clone());
        }
    }

    // Build lines for sections that have signatures.
    let mut lines: Vec<(String, Vec<String>)> = top_sections
        .iter()
        .filter_map(|section| {
            let sigs = section_sigs.get(section)?;
            if sigs.is_empty() {
                return None;
            }
            Some((section.to_uppercase(), sigs.clone()))
        })
        .collect();

    // Trim until under 500 tokens.
    let target = 500_u32;

    loop {
        let result = lines
            .iter()
            .map(|(name, sigs)| format!("{name}: {}", sigs.join(", ")))
            .collect::<Vec<_>>()
            .join("\n");

        if counter.count(&result) <= target {
            return result;
        }

        // Remove one sig from the section with the most.
        let Some(max_pos) =
            lines.iter().enumerate().max_by_key(|(_, (_, sigs))| sigs.len()).map(|(i, _)| i)
        else {
            return lines
                .iter()
                .map(|(name, sigs)| format!("{name}: {}", sigs.join(", ")))
                .collect::<Vec<_>>()
                .join("\n");
        };

        let sigs = &mut lines[max_pos].1;
        if sigs.len() <= 1 {
            // Can't trim further; return what we have.
            break;
        }
        sigs.pop();
    }

    lines
        .iter()
        .map(|(name, sigs)| format!("{name}: {}", sigs.join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Private date helpers ──────────────────────────────────────────────────────

/// Returns the current UTC time as an ISO 8601 string (`YYYY-MM-DDTHH:MM:SSZ`).
fn utc_now_iso8601() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, mo, d, h, mi, s) = epoch_to_utc(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Convert Unix epoch seconds to `(year, month, day, hour, minute, second)`.
///
/// Uses the proleptic Gregorian calendar via the civil-date algorithm.
/// Algorithm: <https://howardhinnant.github.io/date_algorithms.html>
#[allow(clippy::many_single_char_names)]
const fn epoch_to_utc(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let sec = (secs % 60) as u32;
    let min = ((secs / 60) % 60) as u32;
    let hour = ((secs / 3600) % 24) as u32;
    let days = secs / 86400;

    #[allow(clippy::cast_possible_wrap)]
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    #[allow(clippy::cast_sign_loss)]
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    #[allow(clippy::cast_possible_wrap)]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    #[allow(clippy::cast_possible_truncation)]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    #[allow(clippy::cast_possible_truncation)]
    let mon = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let yr = (if mon <= 2 { y + 1 } else { y }) as u32;

    (yr, mon, day, hour, min, sec)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_zero_is_unix_epoch() {
        assert_eq!(epoch_to_utc(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn known_timestamp() {
        // 2025-01-15 11:34:56 UTC  →  1_736_940_896
        assert_eq!(epoch_to_utc(1_736_940_896), (2025, 1, 15, 11, 34, 56));
    }

    #[test]
    fn write_manifest_creates_json() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        std::fs::write(&db_path, b"fake db").unwrap();

        let package = Package {
            name: "test".into(),
            registry: "local".into(),
            version: "1.0.0".into(),
            description: Some("Test package".into()),
            source_url: None,
            git_sha: None,
        };
        let stats = BuildStats { chunk_count: 10, code_block_count: 3, ..Default::default() };

        let manifest_path = write_manifest(&db_path, &package, &stats).unwrap();
        assert!(manifest_path.exists());
        assert_eq!(manifest_path.extension().unwrap(), "json");

        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let meta: PackageMetadata = serde_json::from_str(&content).unwrap();
        assert_eq!(meta.package.name, "test");
        assert_eq!(meta.chunk_count, Some(13));
        assert!(meta.build_date.is_some());
    }

    #[test]
    fn extract_signature_from_js_function() {
        assert_eq!(
            extract_signature_from_line("export async function cacheLife(profile: string): void {"),
            Some("cacheLife(profile: string): void".into()),
        );
    }

    #[test]
    fn extract_signature_from_rust_fn() {
        assert_eq!(
            extract_signature_from_line("pub async fn serve(addr: &str) -> Result<(), Error> {"),
            Some("serve(addr: &str) -> Result<(), Error>".into()),
        );
    }

    #[test]
    fn extract_signature_from_python_def() {
        assert_eq!(
            extract_signature_from_line("async def get_user(user_id: int) -> User:"),
            Some("get_user(user_id: int) -> User:".into()),
        );
    }

    #[test]
    fn extract_signature_ignores_plain_statements() {
        assert_eq!(extract_signature_from_line("    return cacheLife(profile)"), None);
    }

    #[test]
    fn build_manifest_text_under_500_tokens() {
        let counter = TokenCounter::new().unwrap();
        let headings = vec![
            HeadingEntry { title: "Caching".into(), level: 1 },
            HeadingEntry { title: "Routing".into(), level: 2 },
        ];
        let signatures = vec![
            ApiSignature { section: "Caching".into(), signature: "cacheLife(profile)".into() },
            ApiSignature { section: "Caching".into(), signature: "cacheTag(...tags)".into() },
            ApiSignature { section: "Routing".into(), signature: "redirect(path)".into() },
        ];

        let result = build_manifest_text(&headings, &signatures, &counter);
        assert!(counter.count(&result) <= 500, "manifest exceeds 500 tokens: {result}");
        assert!(result.contains("CACHING"), "must contain CACHING section");
        assert!(result.contains("cacheLife"), "must contain cacheLife signature");
    }

    #[tokio::test]
    async fn generate_api_manifest_stores_in_meta() {
        use lore_core::{NewNode, NodeKind};
        let db = lore_core::Db::open_in_memory().await.unwrap();

        let doc_id = db.insert_doc("api.md".into(), Some("API".into())).await.unwrap();
        let heading_id = db
            .insert_node(NewNode {
                parent_id: None,
                doc_id,
                kind: NodeKind::Heading,
                level: Some(1),
                title: Some("Caching".into()),
                content: None,
                token_count: 0,
                lang: None,
            })
            .await
            .unwrap();
        db.insert_node(NewNode {
            parent_id: Some(heading_id),
            doc_id,
            kind: NodeKind::CodeBlock,
            level: None,
            title: None,
            content: Some("export async function cacheLife(profile: string) {}".into()),
            token_count: 10,
            lang: Some("ts".into()),
        })
        .await
        .unwrap();

        generate_api_manifest(&db).await.unwrap();

        let stored = db.get_meta("manifest".into()).await.unwrap();
        assert!(stored.is_some(), "manifest must be stored in meta table");
        let manifest = stored.unwrap();
        assert!(!manifest.is_empty(), "manifest must not be empty");
    }
}
