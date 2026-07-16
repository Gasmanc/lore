//! API-surface extraction and version diffing.
//!
//! Compares two indexed packages of the same library at different versions and
//! reports which API items were added, removed, or had their signature changed
//! — a breaking-change signal for coding agents ("what changed between axum 0.7
//! and 0.8?").
//!
//! The comparable surface is the set of **code-block signatures** keyed by their
//! heading breadcrumb. This is most precise on `build-rustdoc` packages, where
//! every item is a heading (its full path) with a one-line signature; for prose
//! docs it diffs the documented code examples.

use std::collections::BTreeMap;

use lore_core::{Db, LoreError, NodeKind};

/// A package's public API surface: item key → one-line signature.
///
/// The key is the item's heading breadcrumb (e.g. `tokio::sync::Mutex`); the
/// value is the first line of the associated code block.
#[derive(Debug, Default, Clone)]
pub struct ApiSurface {
    items: BTreeMap<String, String>,
}

/// The result of diffing two [`ApiSurface`]s.
#[derive(Debug, Default)]
pub struct ApiDiff {
    /// Keys present only in the newer version.
    pub added: Vec<String>,
    /// Keys present only in the older version.
    pub removed: Vec<String>,
    /// Keys in both whose signature changed: `(key, old_sig, new_sig)`.
    pub changed: Vec<(String, String, String)>,
}

/// Extract the API surface of an indexed package.
///
/// # Errors
///
/// Returns [`LoreError`] if the database cannot be read.
pub async fn api_surface(db: &Db) -> Result<ApiSurface, LoreError> {
    let code_blocks = db.get_nodes_by_kind(NodeKind::CodeBlock).await?;
    if code_blocks.is_empty() {
        return Ok(ApiSurface::default());
    }

    let ids: Vec<i64> = code_blocks.iter().map(|n| n.id).collect();
    let paths = db.get_heading_paths_for_nodes(ids).await?;
    let path_by_id: BTreeMap<i64, Vec<String>> = paths.into_iter().collect();

    let mut items = BTreeMap::new();
    for node in &code_blocks {
        let Some(content) = node.content.as_deref() else { continue };
        let Some(sig) = content.lines().map(str::trim).find(|l| !l.is_empty()) else {
            continue;
        };
        let key = path_by_id
            .get(&node.id)
            .filter(|p| !p.is_empty())
            .map_or_else(|| sig.to_owned(), |p| p.join("::"));
        // First signature under a heading wins (rustdoc packages have exactly one).
        items.entry(key).or_insert_with(|| sig.to_owned());
    }
    Ok(ApiSurface { items })
}

/// Diff two API surfaces (old → new).
#[must_use]
pub fn diff_api(old: &ApiSurface, new: &ApiSurface) -> ApiDiff {
    let mut diff = ApiDiff::default();
    for (key, new_sig) in &new.items {
        match old.items.get(key) {
            None => diff.added.push(key.clone()),
            Some(old_sig) if old_sig != new_sig => {
                diff.changed.push((key.clone(), old_sig.clone(), new_sig.clone()));
            }
            Some(_) => {}
        }
    }
    for key in old.items.keys() {
        if !new.items.contains_key(key) {
            diff.removed.push(key.clone());
        }
    }
    diff.added.sort();
    diff.removed.sort();
    diff.changed.sort();
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(pairs: &[(&str, &str)]) -> ApiSurface {
        ApiSurface {
            items: pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
        }
    }

    #[test]
    fn detects_added_removed_changed() {
        let old = surface(&[
            ("axum::routing::get", "pub fn get(handler: H) -> Route"),
            ("axum::Server", "pub struct Server"),
            ("axum::routing::delete", "pub fn delete(handler: H) -> Route"),
        ]);
        let new = surface(&[
            ("axum::routing::get", "pub fn get(handler: H) -> MethodRouter"), // changed
            ("axum::serve", "pub fn serve(listener: L, service: S)"),         // added
            ("axum::routing::delete", "pub fn delete(handler: H) -> Route"),  // unchanged
        ]);
        let d = diff_api(&old, &new);
        assert_eq!(d.added, vec!["axum::serve"]);
        assert_eq!(d.removed, vec!["axum::Server"]);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].0, "axum::routing::get");
    }

    #[test]
    fn identical_surfaces_have_no_diff() {
        let s = surface(&[("a::b", "fn b()")]);
        let d = diff_api(&s, &s);
        assert!(d.added.is_empty() && d.removed.is_empty() && d.changed.is_empty());
    }
}
