//! Project dependency detection ("stack scoping").
//!
//! Reads the manifest(s) in a project directory — `Cargo.toml`, `package.json`,
//! `pyproject.toml` — and extracts the declared dependencies as
//! `(registry, name, version)` triples. Callers map these onto installed Lore
//! packages so an agent can search "the libraries this project actually uses"
//! without having to know exact package keys, and so version drift between the
//! project and the indexed docs can be surfaced.

use std::path::Path;

/// A dependency declared by a project manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// Lore registry this dependency belongs to (`cargo`, `npm`, `pypi`).
    pub registry: String,
    /// Dependency name as written in the manifest.
    pub name: String,
    /// Declared version requirement, if any (verbatim from the manifest).
    pub version: Option<String>,
}

/// Detect declared dependencies across every recognised manifest in `dir`.
///
/// Missing or malformed manifests are skipped silently — this is a
/// best-effort convenience, never a hard error.
#[must_use]
pub fn detect_dependencies(dir: &Path) -> Vec<Dependency> {
    let mut out = Vec::new();
    out.extend(cargo_deps(dir));
    out.extend(npm_deps(dir));
    out.extend(pypi_deps(dir));
    out.sort_by(|a, b| (a.registry.as_str(), a.name.as_str()).cmp(&(&b.registry, &b.name)));
    out.dedup();
    out
}

// ── Cargo.toml ──────────────────────────────────────────────────────────────

fn cargo_deps(dir: &Path) -> Vec<Dependency> {
    let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
        return vec![];
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return vec![];
    };
    let mut out = Vec::new();
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(deps) = value.get(table).and_then(toml::Value::as_table) {
            for (name, spec) in deps {
                out.push(Dependency {
                    registry: "cargo".to_owned(),
                    name: name.clone(),
                    version: cargo_version(spec),
                });
            }
        }
    }
    // Workspace-level dependency table (`[workspace.dependencies]`).
    if let Some(ws) = value
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        for (name, spec) in ws {
            out.push(Dependency {
                registry: "cargo".to_owned(),
                name: name.clone(),
                version: cargo_version(spec),
            });
        }
    }
    out
}

/// A cargo dependency spec is either a bare version string or a table with a
/// `version` key.
fn cargo_version(spec: &toml::Value) -> Option<String> {
    match spec {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Table(t) => t.get("version").and_then(toml::Value::as_str).map(str::to_owned),
        _ => None,
    }
}

// ── package.json ────────────────────────────────────────────────────────────

fn npm_deps(dir: &Path) -> Vec<Dependency> {
    let Ok(text) = std::fs::read_to_string(dir.join("package.json")) else {
        return vec![];
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return vec![];
    };
    let mut out = Vec::new();
    for key in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(obj) = value.get(key).and_then(serde_json::Value::as_object) {
            for (name, ver) in obj {
                out.push(Dependency {
                    registry: "npm".to_owned(),
                    name: name.clone(),
                    version: ver.as_str().map(str::to_owned),
                });
            }
        }
    }
    out
}

// ── pyproject.toml ──────────────────────────────────────────────────────────

fn pypi_deps(dir: &Path) -> Vec<Dependency> {
    let Ok(text) = std::fs::read_to_string(dir.join("pyproject.toml")) else {
        return vec![];
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return vec![];
    };
    let mut out = Vec::new();

    // PEP 621: [project].dependencies = ["requests>=2", ...]
    if let Some(list) =
        value.get("project").and_then(|p| p.get("dependencies")).and_then(toml::Value::as_array)
    {
        for item in list.iter().filter_map(toml::Value::as_str) {
            if let Some((name, version)) = parse_pep508(item) {
                let version = if version.is_empty() { None } else { Some(version) };
                out.push(Dependency { registry: "pypi".to_owned(), name, version });
            }
        }
    }
    // Poetry: [tool.poetry.dependencies] table.
    if let Some(table) = value
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        for (name, spec) in table {
            if name.eq_ignore_ascii_case("python") {
                continue;
            }
            out.push(Dependency {
                registry: "pypi".to_owned(),
                name: name.clone(),
                version: spec.as_str().map(str::to_owned),
            });
        }
    }
    out
}

/// Extract `(name, version)` from a PEP 508 requirement like
/// `requests>=2.0,<3` or `numpy[extra]==1.0`.
fn parse_pep508(req: &str) -> Option<(String, String)> {
    let req = req.trim();
    // The name runs until the first version operator, extra bracket, or space.
    let end = req.find(|c: char| "<>=!~[ ;(".contains(c)).unwrap_or(req.len());
    let name = req[..end].trim().to_owned();
    if name.is_empty() {
        return None;
    }
    let version = req[end..].trim().trim_start_matches(['[']).to_owned();
    Some((name, if version.is_empty() { String::new() } else { version }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn reads_cargo_dependencies() {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            "Cargo.toml",
            "[dependencies]\ntokio = \"1.44\"\nserde = { version = \"1.0\", features = [\"derive\"] }\n",
        );
        let deps = detect_dependencies(d.path());
        assert!(deps.iter().any(|x| x.registry == "cargo" && x.name == "tokio"));
        assert!(
            deps.iter()
                .any(|x| x.name == "serde" && x.version.as_deref() == Some("1.0"))
        );
    }

    #[test]
    fn reads_npm_dependencies() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "package.json", r#"{"dependencies":{"react":"19.0.0"}}"#);
        let deps = detect_dependencies(d.path());
        assert!(
            deps.iter()
                .any(|x| x.registry == "npm" && x.name == "react" && x.version.as_deref() == Some("19.0.0"))
        );
    }

    #[test]
    fn reads_pep621_dependencies() {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            "pyproject.toml",
            "[project]\nname = \"x\"\ndependencies = [\"requests>=2.0\", \"numpy==1.26\"]\n",
        );
        let deps = detect_dependencies(d.path());
        assert!(deps.iter().any(|x| x.registry == "pypi" && x.name == "requests"));
        assert!(deps.iter().any(|x| x.name == "numpy"));
    }

    #[test]
    fn parse_pep508_variants() {
        assert_eq!(parse_pep508("requests>=2.0").unwrap().0, "requests");
        assert_eq!(parse_pep508("numpy[extra]==1.0").unwrap().0, "numpy");
        assert_eq!(parse_pep508("flask").unwrap(), ("flask".to_owned(), String::new()));
    }

    #[test]
    fn missing_manifests_yield_empty() {
        let d = tempfile::tempdir().unwrap();
        assert!(detect_dependencies(d.path()).is_empty());
    }
}
