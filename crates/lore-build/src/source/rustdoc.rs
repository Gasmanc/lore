//! [`RustdocSource`] — ingest a crate's `rustdoc --output-format json`.
//!
//! `rustdoc`'s JSON output is the most precise, version-exact description of a
//! Rust crate's public API: every item, its full path, signature, and doc
//! comment. This source converts that JSON into Markdown (one heading per item,
//! titled with the item's full path) and hands it to the standard pipeline, so
//! the indexed docs match the *locked* version of a dependency exactly.
//!
//! Two entry points:
//! - [`RustdocSource::from_json`] — parse an existing rustdoc JSON file.
//! - [`RustdocSource::from_crate`] — run `cargo +nightly rustdoc … --output-format
//!   json` for a crate in a project, then parse the result.
//!
//! The converter ([`rustdoc_json_to_markdown`]) is pure and unit-tested; the
//! cargo invocation is a thin wrapper around it.

use std::path::{Path, PathBuf};
use std::process::Command;

use lore_core::LoreError;
use serde_json::Value;
use tracing::info;

use super::{PreparedSource, Source};

/// How a [`RustdocSource`] obtains its rustdoc JSON.
pub enum RustdocInput {
    /// A path to an existing `*.json` produced by `rustdoc --output-format json`.
    Json(PathBuf),
    /// A crate to document by invoking cargo in `manifest_dir`.
    Crate {
        /// Crate name as cargo knows it (e.g. `tokio`).
        name: String,
        /// Directory containing the `Cargo.toml` where the crate is a member or
        /// dependency. Defaults to the current directory when empty.
        manifest_dir: PathBuf,
    },
}

/// A documentation source backed by rustdoc JSON.
pub struct RustdocSource {
    input: RustdocInput,
}

impl RustdocSource {
    /// Build from an existing rustdoc JSON file.
    pub const fn from_json(path: PathBuf) -> Self {
        Self { input: RustdocInput::Json(path) }
    }

    /// Build by running `cargo rustdoc` for `name` in `manifest_dir`.
    pub fn from_crate(name: impl Into<String>, manifest_dir: impl Into<PathBuf>) -> Self {
        Self { input: RustdocInput::Crate { name: name.into(), manifest_dir: manifest_dir.into() } }
    }
}

impl Source for RustdocSource {
    async fn prepare(&self) -> Result<PreparedSource, LoreError> {
        let json = match &self.input {
            RustdocInput::Json(path) => {
                std::fs::read_to_string(path).map_err(LoreError::Io)?
            }
            RustdocInput::Crate { name, manifest_dir } => generate_rustdoc_json(name, manifest_dir)?,
        };

        let markdown = rustdoc_json_to_markdown(&json)?;
        if markdown.trim().is_empty() {
            return Err(LoreError::Parse("rustdoc JSON produced no documentable items".into()));
        }

        let temp = tempfile::tempdir().map_err(LoreError::Io)?;
        let file = temp.path().join("api.md");
        std::fs::write(&file, markdown.as_bytes()).map_err(LoreError::Io)?;
        Ok(PreparedSource::from_temp(temp, None))
    }
}

// ── cargo rustdoc invocation ────────────────────────────────────────────────

/// Run `cargo +nightly rustdoc … --output-format json` for `crate_name` and
/// return the contents of the generated JSON file.
fn generate_rustdoc_json(crate_name: &str, manifest_dir: &Path) -> Result<String, LoreError> {
    let dir = if manifest_dir.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        manifest_dir.to_path_buf()
    };

    info!(crate_name, dir = %dir.display(), "running cargo rustdoc (json)");
    let output = Command::new("cargo")
        .current_dir(&dir)
        .args([
            "+nightly",
            "rustdoc",
            "-p",
            crate_name,
            "--lib",
            "--",
            "-Z",
            "unstable-options",
            "--output-format",
            "json",
        ])
        .output()
        .map_err(|e| {
            LoreError::Parse(format!(
                "failed to run `cargo +nightly rustdoc` (is the nightly toolchain installed?): {e}"
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(LoreError::Parse(format!(
            "cargo rustdoc failed for crate '{crate_name}': {}",
            stderr.lines().last().unwrap_or("unknown error")
        )));
    }

    // rustdoc writes to <target>/doc/<crate_snake>.json.
    let snake = crate_name.replace('-', "_");
    for candidate in [
        dir.join("target").join("doc").join(format!("{snake}.json")),
        dir.join("..").join("target").join("doc").join(format!("{snake}.json")),
    ] {
        if candidate.is_file() {
            return std::fs::read_to_string(&candidate).map_err(LoreError::Io);
        }
    }
    Err(LoreError::Parse(format!("could not locate rustdoc JSON for '{crate_name}' under target/doc")))
}

// ── JSON → Markdown converter ───────────────────────────────────────────────

/// Convert rustdoc JSON into Markdown, one `##` heading per public item titled
/// with the item's full path (e.g. `tokio::sync::Mutex`).
///
/// # Errors
///
/// Returns [`LoreError::Parse`] if the JSON is malformed.
pub fn rustdoc_json_to_markdown(json: &str) -> Result<String, LoreError> {
    use std::fmt::Write as _;

    let root: Value =
        serde_json::from_str(json).map_err(|e| LoreError::Parse(format!("invalid rustdoc JSON: {e}")))?;

    let index = root.get("index").and_then(Value::as_object).ok_or_else(|| {
        LoreError::Parse("rustdoc JSON missing `index`".into())
    })?;
    let paths = root.get("paths").and_then(Value::as_object);

    let crate_name = root
        .get("root")
        .and_then(|r| paths.and_then(|p| p.get(&r.to_string())))
        .and_then(|v| v.get("path"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("crate")
        .to_owned();

    let mut items: Vec<ApiItem> = Vec::new();
    for (id, item) in index {
        let Some(name) = item.get("name").and_then(Value::as_str) else { continue };
        if name.is_empty() {
            continue;
        }
        let Some((kind, inner)) = item
            .get("inner")
            .and_then(Value::as_object)
            .and_then(|o| o.iter().next())
        else {
            continue;
        };
        // Skip items that only make sense inside a parent, or have no own docs page.
        if matches!(kind.as_str(), "struct_field" | "variant" | "impl" | "module" | "use") {
            continue;
        }

        // Include an item if its path lives under the documented crate's
        // namespace — this covers both the crate's own items and re-exports
        // rustdoc inlined into it (facade crates) regardless of origin
        // `crate_id`. Fall back to `crate_id == 0` for items without a path.
        let path_segs = paths
            .and_then(|p| p.get(id))
            .and_then(|v| v.get("path"))
            .and_then(Value::as_array);
        let under_crate = path_segs
            .and_then(|segs| segs.first())
            .and_then(Value::as_str)
            == Some(crate_name.as_str());
        let is_local = item.get("crate_id").and_then(Value::as_i64) == Some(0);
        if !under_crate && !is_local {
            continue;
        }

        let path = path_segs
            .map(|segs| segs.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("::"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| name.to_owned());

        let docs = item.get("docs").and_then(Value::as_str).unwrap_or("").to_owned();
        let signature = render_signature(kind, name, inner);
        items.push(ApiItem { path, signature, docs });
    }

    items.sort_by(|a, b| a.path.cmp(&b.path));

    let mut out = format!("# {crate_name}\n\n");
    let crate_prefix = format!("{crate_name}::");
    for item in items {
        // The item path already includes the crate; strip that prefix so the
        // heading breadcrumb (crate H1 + item H2) doesn't double the crate name.
        let heading = item.path.strip_prefix(&crate_prefix).unwrap_or(&item.path);
        // Writing to a String is infallible; ignore the formatter Result.
        let _ = write!(out, "## {heading}\n\n```rust\n{}\n```\n\n", item.signature);
        if !item.docs.trim().is_empty() {
            out.push_str(item.docs.trim());
            out.push_str("\n\n");
        }
    }
    Ok(out)
}

struct ApiItem {
    path: String,
    signature: String,
    docs: String,
}

/// Render a one-line signature for an item of the given `kind`.
fn render_signature(kind: &str, name: &str, inner: &Value) -> String {
    match kind {
        "function" => render_fn(name, inner),
        "struct" => format!("pub struct {name}"),
        "enum" => format!("pub enum {name}"),
        "trait" => format!("pub trait {name}"),
        "trait_alias" => format!("pub trait {name} = …"),
        "constant" => format!("pub const {name}"),
        "static" => format!("pub static {name}"),
        "type_alias" | "typedef" => format!("pub type {name}"),
        "macro" | "proc_macro" => format!("{name}!"),
        "primitive" => format!("primitive {name}"),
        other => format!("{other} {name}"),
    }
}

/// Render a function signature from its `sig` (or legacy `decl`) object.
fn render_fn(name: &str, inner: &Value) -> String {
    let header = inner.get("header");
    let mut prefix = String::from("pub ");
    if header.and_then(|h| h.get("is_const")).and_then(Value::as_bool) == Some(true) {
        prefix.push_str("const ");
    }
    if header.and_then(|h| h.get("is_async")).and_then(Value::as_bool) == Some(true) {
        prefix.push_str("async ");
    }
    if header.and_then(|h| h.get("is_unsafe")).and_then(Value::as_bool) == Some(true) {
        prefix.push_str("unsafe ");
    }

    let sig = inner.get("sig").or_else(|| inner.get("decl"));
    let inputs = sig
        .and_then(|s| s.get("inputs"))
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(|pair| {
                    let arr = pair.as_array()?;
                    let n = arr.first()?.as_str()?;
                    let ty = render_type(arr.get(1)?, 0);
                    Some(if n == "self" { "self".to_owned() } else { format!("{n}: {ty}") })
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let output = sig
        .and_then(|s| s.get("output"))
        .filter(|o| !o.is_null())
        .map(|o| format!(" -> {}", render_type(o, 0)))
        .unwrap_or_default();

    format!("{prefix}fn {name}({inputs}){output}")
}

/// Render a rustdoc `Type` value to a readable string. `depth` bounds recursion
/// so a pathologically nested type can't blow the stack.
fn render_type(ty: &Value, depth: u8) -> String {
    if depth > 8 {
        return "_".to_owned();
    }
    let Some((tag, body)) = ty.as_object().and_then(|o| o.iter().next()) else {
        return "_".to_owned();
    };
    match tag.as_str() {
        "primitive" | "generic" => body.as_str().unwrap_or("_").to_owned(),
        "resolved_path" => {
            let path = body.get("path").and_then(Value::as_str).unwrap_or("_");
            let args = body
                .get("args")
                .and_then(|a| a.get("angle_bracketed"))
                .and_then(|a| a.get("args"))
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|a| a.get("type").map(|t| render_type(t, depth + 1)))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if args.is_empty() { path.to_owned() } else { format!("{path}<{}>", args.join(", ")) }
        }
        "borrowed_ref" => {
            let mutable = body.get("is_mutable").and_then(Value::as_bool) == Some(true);
            let inner = body.get("type").map_or_else(|| "_".to_owned(), |t| render_type(t, depth + 1));
            if mutable { format!("&mut {inner}") } else { format!("&{inner}") }
        }
        "raw_pointer" => {
            let mutable = body.get("is_mutable").and_then(Value::as_bool) == Some(true);
            let inner = body.get("type").map_or_else(|| "_".to_owned(), |t| render_type(t, depth + 1));
            format!("*{} {inner}", if mutable { "mut" } else { "const" })
        }
        "tuple" => {
            let parts = body
                .as_array()
                .map(|a| a.iter().map(|t| render_type(t, depth + 1)).collect::<Vec<_>>())
                .unwrap_or_default();
            format!("({})", parts.join(", "))
        }
        "slice" => format!("[{}]", render_type(body, depth + 1)),
        "array" => {
            let inner = body.get("type").map_or_else(|| "_".to_owned(), |t| render_type(t, depth + 1));
            let len = body.get("len").and_then(Value::as_str).unwrap_or("_");
            format!("[{inner}; {len}]")
        }
        "impl_trait" => "impl _".to_owned(),
        "dyn_trait" => "dyn _".to_owned(),
        _ => "_".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal rustdoc JSON (format_version 60 shape) with one function.
    const SAMPLE: &str = r#"{
        "root": 45,
        "format_version": 60,
        "paths": {
            "0": {"crate_id": 0, "path": ["rdtest", "add"], "kind": "function"},
            "3": {"crate_id": 0, "path": ["rdtest", "Point"], "kind": "struct"},
            "45": {"crate_id": 0, "path": ["rdtest"], "kind": "module"}
        },
        "index": {
            "0": {"crate_id": 0, "name": "add", "docs": "Adds two numbers.",
                  "inner": {"function": {"sig": {"inputs": [["a", {"primitive": "u32"}], ["b", {"primitive": "u32"}]], "output": {"primitive": "u32"}},
                                          "header": {"is_const": false, "is_async": false, "is_unsafe": false}}}},
            "3": {"crate_id": 0, "name": "Point", "docs": "A point.",
                  "inner": {"struct": {"kind": "plain", "impls": []}}},
            "9": {"crate_id": 2, "name": "Drop", "docs": "from core",
                  "inner": {"trait": {}}}
        }
    }"#;

    #[test]
    fn converts_function_and_struct() {
        let md = rustdoc_json_to_markdown(SAMPLE).unwrap();
        assert!(md.contains("# rdtest"), "{md}");
        // The crate prefix is stripped from item headings (it's the H1).
        assert!(md.contains("## add"), "{md}");
        assert!(md.contains("pub fn add(a: u32, b: u32) -> u32"), "{md}");
        assert!(md.contains("Adds two numbers."));
        assert!(md.contains("## Point"));
        assert!(md.contains("pub struct Point"));
    }

    #[test]
    fn excludes_foreign_crate_items() {
        let md = rustdoc_json_to_markdown(SAMPLE).unwrap();
        assert!(!md.contains("Drop"), "items from other crates must be excluded: {md}");
    }

    #[test]
    fn renders_nested_generic_and_ref_types() {
        let vec_t = serde_json::json!({
            "resolved_path": {"path": "Vec", "args": {"angle_bracketed": {"args": [{"type": {"generic": "T"}}]}}}
        });
        assert_eq!(render_type(&vec_t, 0), "Vec<T>");
        let ref_str = serde_json::json!({"borrowed_ref": {"is_mutable": false, "type": {"primitive": "str"}}});
        assert_eq!(render_type(&ref_str, 0), "&str");
    }

    #[test]
    fn malformed_json_errors() {
        assert!(rustdoc_json_to_markdown("not json").is_err());
    }
}
