//! Manifest generation — writes a JSON sidecar alongside the built `.db` file.
//!
//! The manifest is a serialised [`PackageMetadata`] that records build-time
//! statistics and is used to populate the registry index.
//!
//! Call [`write_manifest`] after a successful [`crate::builder::PackageBuilder::build`]
//! to produce a `<stem>.json` file next to the `.db`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lore_core::{LoreError, Package, PackageMetadata};
use serde_json;

use crate::builder::BuildStats;

// ── Public API ────────────────────────────────────────────────────────────────

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

    let json = serde_json::to_string_pretty(&metadata)
        .map_err(|e| LoreError::Schema(e.to_string()))?;

    let manifest_path = db_path.with_extension("json");
    std::fs::write(&manifest_path, json.as_bytes()).map_err(LoreError::Io)?;
    Ok(manifest_path)
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Returns the current UTC time as an ISO 8601 string (`YYYY-MM-DDTHH:MM:SSZ`).
fn utc_now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let (y, mo, d, h, mi, s) = epoch_to_utc(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Convert Unix epoch seconds to `(year, month, day, hour, minute, second)`.
///
/// Uses the proleptic Gregorian calendar via the civil-date algorithm.
/// This avoids pulling in a date/time crate for a simple timestamp.
/// Algorithm: <https://howardhinnant.github.io/date_algorithms.html>
#[allow(clippy::many_single_char_names)]
const fn epoch_to_utc(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let sec  = (secs % 60) as u32;
    let min  = ((secs / 60) % 60) as u32;
    let hour = ((secs / 3600) % 24) as u32;
    let days = secs / 86400;

    // Civil date from day number (days since 1970-01-01).
    #[allow(clippy::cast_possible_wrap)]
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    #[allow(clippy::cast_sign_loss)]
    let doe = z.rem_euclid(146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    #[allow(clippy::cast_possible_wrap)]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp  = (5 * doy + 2) / 153; // [0, 11]
    #[allow(clippy::cast_possible_truncation)]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]; value is always ≤31
    #[allow(clippy::cast_possible_truncation)]
    let mon = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]; value is always ≤12
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
        // Verified: 20_103 days * 86400 + 41_696 s = 1_736_940_896
        assert_eq!(epoch_to_utc(1_736_940_896), (2025, 1, 15, 11, 34, 56));
    }

    #[test]
    fn write_manifest_creates_json(  ) {
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
        let stats = BuildStats {
            chunk_count: 10,
            code_block_count: 3,
            ..Default::default()
        };

        let manifest_path = write_manifest(&db_path, &package, &stats).unwrap();
        assert!(manifest_path.exists());
        assert_eq!(manifest_path.extension().unwrap(), "json");

        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let meta: PackageMetadata = serde_json::from_str(&content).unwrap();
        assert_eq!(meta.package.name, "test");
        assert_eq!(meta.chunk_count, Some(13));
        assert!(meta.build_date.is_some());
    }
}
