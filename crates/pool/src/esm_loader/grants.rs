//! RFD 27 host-bridge grant plumbing: manifest reads, lockfile digests, and
//! package-name derivation.
//!
//! Everything here answers "which published identity does this directory on
//! disk correspond to, and what did the user record about it?" The grant
//! *decisions* themselves live in `runtime_core::host_bridge`; this module
//! only feeds them the project-root manifest fields and lockfile-pinned
//! fsGraph digests they key off.

use std::collections::HashMap;
use std::path::Path;

/// Read `<root>/deka.json` and return its `(host.kinds, name)`. Both absent
/// manifest and absent fields yield empty values.
pub(crate) fn read_manifest_host_kinds(root: &Path) -> (Option<Vec<String>>, String) {
    let path = root.join("deka.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return (None, String::new());
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (None, String::new());
    };
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let kinds = value
        .get("host")
        .and_then(|host| host.get("kinds"))
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect::<Vec<String>>()
        });
    (kinds, name)
}

pub(crate) fn read_manifest_name(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("deka.json")).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// Read `<project_root>/deka.lock` once and extract each package's
/// lockfile-pinned `metadata.fsGraph.hash` digest. Defensive inline JSON
/// parse: anything missing or malformed yields no digest for that package
/// (which simply means no table lookup later, i.e. no grants).
///
/// Lock entries serialize as a 4-element array
/// `[descriptor, resolved, metadata, integrity]` (crates/pm/src/lock.rs);
/// the metadata object — including `fsGraph.hash` — lives at index 2. The
/// `{"metadata": ...}` object spelling is also accepted so hand-written and
/// future lock shapes keep working.
pub(crate) fn read_lock_digests(project_root: &Path) -> HashMap<String, String> {
    let mut digests = HashMap::new();
    let Ok(text) = std::fs::read_to_string(project_root.join("deka.lock")) else {
        return digests;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return digests;
    };
    let Some(packages) = value.get("packages").and_then(serde_json::Value::as_object) else {
        return digests;
    };
    for (name, entry) in packages {
        let metadata = entry
            .as_array()
            .and_then(|fields| fields.get(2))
            .or_else(|| entry.get("metadata"));
        let digest = metadata
            .and_then(|metadata| metadata.get("fsGraph"))
            .and_then(|fs_graph| fs_graph.get("hash"))
            .and_then(serde_json::Value::as_str);
        if let Some(digest) = digest {
            digests.insert(name.clone(), digest.to_string());
        }
    }
    digests
}

/// Package name from a dependency package root: the path relative to the
/// modules dir, with scoped names (`@deka/crypto`) kept as two segments.
pub(crate) fn dependency_package_name(package_root: &Path, project_root: &Path) -> String {
    for dir in ["ds_modules", "php_modules"] {
        let modules_dir = project_root.join(dir);
        if let Ok(rel) = package_root.strip_prefix(&modules_dir) {
            return rel.to_string_lossy().replace('\\', "/");
        }
    }
    package_root.to_string_lossy().replace('\\', "/")
}
