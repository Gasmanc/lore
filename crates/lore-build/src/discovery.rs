//! File discovery — walks a directory tree and returns documentation files.
//!
//! Skips well-known build artefact directories (`node_modules`, `target`,
//! etc.), non-documentation files (changelogs, licence files), and optionally
//! example/test directories.

use std::path::{Path, PathBuf};

use lore_core::LoreError;
use walkdir::WalkDir;

// ── Constants ─────────────────────────────────────────────────────────────────

/// File extensions that are treated as documentation sources.
pub const INCLUDED_EXTENSIONS: &[&str] =
    &["md", "mdx", "qmd", "rmd", "html", "htm", "adoc", "asciidoc", "rst"];

/// Directory names that are always skipped during traversal.
pub const EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "__pycache__",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".svelte-kit",
    "vendor",
];

/// File stem names (without extension) that are always excluded.
/// Comparison is case-insensitive.
pub const EXCLUDED_NAMES: &[&str] = &[
    "CHANGELOG",
    "CODE_OF_CONDUCT",
    "LICENSE",
    "LICENCE",
    "CONTRIBUTING",
    "AUTHORS",
    "CODEOWNERS",
];

/// Directory names skipped when `exclude_examples` is `true`.
pub const EXAMPLE_DIRS: &[&str] =
    &["examples", "example", "fixtures", "fixture", "test", "tests", "spec", "specs"];

// ── Public API ────────────────────────────────────────────────────────────────

/// Walk `root` recursively and return all documentation files.
///
/// Files are sorted lexicographically for deterministic output.
///
/// # Arguments
///
/// * `exclude_examples` — when `true`, also skip directories named
///   `examples`, `example`, `fixtures`, `fixture`, `test`, `tests`,
///   `spec`, `specs`.
///
/// # Errors
///
/// Returns [`LoreError::Io`] if any directory entry cannot be read.
pub fn discover_files(root: &Path, exclude_examples: bool) -> Result<Vec<PathBuf>, LoreError> {
    let mut files: Vec<PathBuf> = Vec::new();

    let walker = WalkDir::new(root).into_iter().filter_entry(|entry| {
        if entry.file_type().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                if EXCLUDED_DIRS.contains(&name)
                    || (exclude_examples && EXAMPLE_DIRS.contains(&name))
                {
                    return false;
                }
            }
        }
        true
    });

    for result in walker {
        let entry = result.map_err(|e| {
            let io = e
                .into_io_error()
                .unwrap_or_else(|| std::io::Error::other("walkdir error"));
            LoreError::Io(io)
        })?;

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !INCLUDED_EXTENSIONS.contains(&ext) {
            continue;
        }

        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if EXCLUDED_NAMES.iter().any(|&n| n.eq_ignore_ascii_case(stem)) {
            continue;
        }

        files.push(path.to_path_buf());
    }

    files.sort();
    Ok(files)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn setup() -> TempDir {
        tempfile::tempdir().expect("temp dir must create")
    }

    #[test]
    fn test_discovers_md_files() {
        let dir = setup();
        fs::write(dir.path().join("one.md"), "# One").unwrap();
        fs::write(dir.path().join("two.md"), "# Two").unwrap();
        fs::write(dir.path().join("three.md"), "# Three").unwrap();
        let files = discover_files(dir.path(), false).unwrap();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_discovers_all_supported_extensions() {
        let dir = setup();
        for ext in INCLUDED_EXTENSIONS {
            fs::write(dir.path().join(format!("doc.{ext}")), "content").unwrap();
        }
        let files = discover_files(dir.path(), false).unwrap();
        assert_eq!(files.len(), INCLUDED_EXTENSIONS.len());
    }

    #[test]
    fn test_excludes_node_modules() {
        let dir = setup();
        let nm = dir.path().join("node_modules");
        fs::create_dir(&nm).unwrap();
        fs::write(nm.join("docs.md"), "# Docs").unwrap();
        fs::write(dir.path().join("readme.md"), "# Readme").unwrap();
        let files = discover_files(dir.path(), false).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("readme.md"));
    }

    #[test]
    fn test_excludes_changelog() {
        let dir = setup();
        fs::write(dir.path().join("CHANGELOG.md"), "## v1.0.0").unwrap();
        fs::write(dir.path().join("CHANGELOG.rst"), "v1").unwrap();
        let files = discover_files(dir.path(), false).unwrap();
        assert_eq!(files.len(), 0, "CHANGELOG files must be excluded");
    }

    #[test]
    fn test_excludes_case_insensitive_names() {
        let dir = setup();
        // Mixed-case variants should all be excluded.
        fs::write(dir.path().join("license.md"), "MIT").unwrap();
        fs::write(dir.path().join("Licence.md"), "Apache").unwrap();
        fs::write(dir.path().join("guide.md"), "# Guide").unwrap();
        let files = discover_files(dir.path(), false).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("guide.md"));
    }

    #[test]
    fn test_excludes_example_dirs_when_requested() {
        let dir = setup();
        let examples = dir.path().join("examples");
        fs::create_dir(&examples).unwrap();
        fs::write(examples.join("demo.md"), "# Demo").unwrap();
        fs::write(dir.path().join("readme.md"), "# Readme").unwrap();

        // Without exclude_examples, the file IS found.
        let without = discover_files(dir.path(), false).unwrap();
        assert_eq!(without.len(), 2);

        // With exclude_examples, the file is skipped.
        let with_exclude = discover_files(dir.path(), true).unwrap();
        assert_eq!(with_exclude.len(), 1);
        assert!(with_exclude[0].ends_with("readme.md"));
    }

    #[test]
    fn test_skips_non_doc_extensions() {
        let dir = setup();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("data.json"), "{}").unwrap();
        fs::write(dir.path().join("doc.md"), "# Doc").unwrap();
        let files = discover_files(dir.path(), false).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn test_results_are_sorted() {
        let dir = setup();
        fs::write(dir.path().join("z.md"), "# Z").unwrap();
        fs::write(dir.path().join("a.md"), "# A").unwrap();
        fs::write(dir.path().join("m.md"), "# M").unwrap();
        let files = discover_files(dir.path(), false).unwrap();
        let names: Vec<_> =
            files.iter().map(|p| p.file_name().unwrap().to_str().unwrap()).collect();
        assert_eq!(names, ["a.md", "m.md", "z.md"]);
    }
}
