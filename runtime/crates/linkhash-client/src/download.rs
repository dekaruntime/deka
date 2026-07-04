//! Package download via git clone with auth support.

use anyhow::{bail, Result};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// Download a package at a specific version to the target directory.
///
/// Uses `git clone --depth 1 --branch v{version}` with bearer auth
/// passed via GIT_HTTP_EXTRAHEADER for authenticated access.
/// Falls back to the tree/blob API if git clone is unavailable.
pub(crate) fn download(
    http: &reqwest::blocking::Client,
    registry_url: &str,
    token: Option<&str>,
    name: &str,
    version: &str,
    target_dir: &Path,
) -> Result<()> {
    let (scope, pkg_name) = crate::parse_scoped_name(name)?;

    // Try the tree/blob API first (works without git CLI)
    match download_via_api(
        http,
        registry_url,
        token,
        &scope,
        &pkg_name,
        version,
        target_dir,
    ) {
        Ok(()) => return Ok(()),
        Err(_api_err) => {
            // Fall back to git clone
            download_via_git(registry_url, token, &scope, &pkg_name, version, target_dir)?;
        }
    }

    Ok(())
}

/// Download via the tree/blob HTTP API.
fn download_via_api(
    http: &reqwest::blocking::Client,
    registry_url: &str,
    token: Option<&str>,
    scope: &str,
    pkg_name: &str,
    version: &str,
    target_dir: &Path,
) -> Result<()> {
    // Step 1: Get the file tree
    let tree_url = format!(
        "{}/api/scoped-packages/{}/{}/{}/tree",
        registry_url, scope, pkg_name, version
    );

    let mut req = http.get(&tree_url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }

    let response = req
        .send()
        .map_err(|e| anyhow::anyhow!("tree request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        bail!("tree API returned {}", status);
    }

    let body: serde_json::Value = response
        .json()
        .map_err(|e| anyhow::anyhow!("failed to parse tree response: {}", e))?;

    let files = registry_tree_entries(&body)?;

    // Step 2: Create target directory
    std::fs::create_dir_all(target_dir)
        .map_err(|e| anyhow::anyhow!("failed to create target dir: {}", e))?;

    // Step 3: Download each file
    for file_entry in files {
        let safe_path = safe_registry_path(&file_entry.path)?;

        // Skip directories and .git
        if matches!(
            safe_path.components().next(),
            Some(Component::Normal(part)) if part == ".git"
        ) {
            continue;
        }
        if file_entry.is_dir {
            std::fs::create_dir_all(target_dir.join(&safe_path))?;
            continue;
        }

        let blob_url = reqwest::Url::parse_with_params(
            &format!(
                "{}/api/scoped-packages/{}/{}/{}/blob",
                registry_url, scope, pkg_name, version
            ),
            &[("path", file_entry.path.as_str())],
        )
        .map_err(|e| anyhow::anyhow!("failed to build blob URL for {}: {}", file_entry.path, e))?;

        let mut blob_req = http.get(blob_url);
        if let Some(t) = token {
            blob_req = blob_req.bearer_auth(t);
        }

        let blob_response = blob_req
            .send()
            .map_err(|e| anyhow::anyhow!("blob request failed for {}: {}", file_entry.path, e))?;

        if !blob_response.status().is_success() {
            bail!(
                "blob API returned {} for {}",
                blob_response.status(),
                file_entry.path
            );
        }

        let content_bytes = blob_response
            .bytes()
            .map_err(|e| anyhow::anyhow!("failed to read blob for {}: {}", file_entry.path, e))?;

        let file_bytes = registry_blob_bytes(&content_bytes);

        let file_path = target_dir.join(&safe_path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file_path, &file_bytes)?;
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistryTreeEntry {
    path: String,
    is_dir: bool,
}

fn registry_tree_entries(body: &serde_json::Value) -> Result<Vec<RegistryTreeEntry>> {
    let files = body
        .get("files")
        .or_else(|| body.get("tree"))
        .or_else(|| body.get("entries"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("tree response missing files array"))?;

    files
        .iter()
        .map(|file_entry| {
            let path = file_entry
                .get("path")
                .or_else(|| file_entry.get("name"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("file entry missing path"))?
                .to_string();
            let is_dir = file_entry
                .get("type")
                .or_else(|| file_entry.get("kind"))
                .and_then(|v| v.as_str())
                .map(|t| t == "tree" || t == "dir")
                .unwrap_or(false);

            Ok(RegistryTreeEntry { path, is_dir })
        })
        .collect()
}

fn registry_blob_bytes(content_bytes: &[u8]) -> Vec<u8> {
    // Registry blob endpoint returns JSON { content, ... }; extract the raw content.
    // Fall back to raw bytes for non-JSON responses.
    match serde_json::from_slice::<serde_json::Value>(content_bytes) {
        Ok(v) if v.get("content").and_then(|c| c.as_str()).is_some() => v
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap()
            .as_bytes()
            .to_vec(),
        _ => content_bytes.to_vec(),
    }
}

fn safe_registry_path(path: &str) -> Result<PathBuf> {
    if path.trim().is_empty() {
        bail!("registry file path is empty");
    }

    let candidate = Path::new(path);
    if candidate.is_absolute() {
        bail!("registry file path escapes package root: {}", path);
    }

    let mut safe = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => {
                if part.is_empty() {
                    bail!("registry file path has an empty component: {}", path);
                }
                safe.push(part);
            }
            Component::CurDir | Component::ParentDir | Component::RootDir => {
                bail!("registry file path escapes package root: {}", path);
            }
            Component::Prefix(_) => {
                bail!("registry file path has a drive prefix: {}", path);
            }
        }
    }

    if safe.as_os_str().is_empty() {
        bail!("registry file path is empty");
    }

    Ok(safe)
}

/// Download via git clone with auth header.
fn download_via_git(
    registry_url: &str,
    token: Option<&str>,
    scope: &str,
    pkg_name: &str,
    version: &str,
    target_dir: &Path,
) -> Result<()> {
    let repo_url = format!("{}/{}/{}.git", registry_url, scope, pkg_name);
    let tag = format!("v{}", version);

    // Clone to a temp dir first, then copy files (excluding .git/)
    let temp_dir =
        std::env::temp_dir().join(format!("linkhash-dl-{}-{}-{}", scope, pkg_name, version));
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir)?;
    }

    let mut cmd = Command::new("git");
    cmd.args(["clone", "--depth", "1", "--branch", &tag, &repo_url]);
    cmd.arg(temp_dir.to_str().unwrap());

    if let Some(t) = token {
        cmd.env(
            "GIT_HTTP_EXTRAHEADER",
            format!("Authorization: Bearer {}", t),
        );
    }

    let status = cmd
        .status()
        .map_err(|e| anyhow::anyhow!("failed to run git clone: {}", e))?;

    if !status.success() {
        // Clean up temp dir on failure
        let _ = std::fs::remove_dir_all(&temp_dir);
        bail!("git clone failed for {}@{}", pkg_name, version);
    }

    // Copy files from temp to target, excluding .git/
    copy_package_files(&temp_dir, target_dir)?;

    // Clean up temp dir
    let _ = std::fs::remove_dir_all(&temp_dir);

    Ok(())
}

/// Copy all files from source to target, excluding .git/ directory.
fn copy_package_files(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir_all(target)?;

    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == ".git" {
            continue;
        }

        let src_path = entry.path();
        let dst_path = target.join(&name);

        if src_path.is_dir() {
            copy_package_files(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        registry_blob_bytes, registry_tree_entries, safe_registry_path, RegistryTreeEntry,
    };
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn registry_path_accepts_nested_relative_files() {
        assert_eq!(
            safe_registry_path("src/index.phpx").expect("path"),
            PathBuf::from("src").join("index.phpx")
        );
    }

    #[test]
    fn registry_path_rejects_parent_traversal() {
        let err = safe_registry_path("../../deka.lock").unwrap_err();
        assert!(err.to_string().contains("escapes package root"));
    }

    #[test]
    fn registry_path_rejects_absolute_paths() {
        let err = safe_registry_path("/tmp/pwned").unwrap_err();
        assert!(err.to_string().contains("escapes package root"));
    }

    #[test]
    fn registry_path_rejects_current_dir_components() {
        let err = safe_registry_path("./index.phpx").unwrap_err();
        assert!(err.to_string().contains("escapes package root"));
    }

    #[test]
    fn tree_response_contract_accepts_documented_entries_shape() {
        let body = json!({
            "package_name": "@tana/store",
            "version": "1.2.0",
            "git_ref": "v1.2.0",
            "entries": [
                {
                    "mode": "100644",
                    "kind": "blob",
                    "object": "6f1ed002ab5595859014ebf0951522d9",
                    "size": 2418,
                    "path": "src/index.phpx"
                },
                {
                    "kind": "tree",
                    "path": "src/views"
                }
            ]
        });

        let entries = registry_tree_entries(&body).expect("tree entries");

        assert_eq!(
            entries,
            vec![
                RegistryTreeEntry {
                    path: "src/index.phpx".to_string(),
                    is_dir: false,
                },
                RegistryTreeEntry {
                    path: "src/views".to_string(),
                    is_dir: true,
                },
            ]
        );
    }

    #[test]
    fn blob_response_contract_accepts_documented_content_shape() {
        let body = br#"{
            "package_name": "@tana/store",
            "version": "1.2.0",
            "path": "src/index.phpx",
            "git_ref": "v1.2.0",
            "content": "<?phpx\nexport function renderStore() { return 'ok'; }\n"
        }"#;

        let bytes = registry_blob_bytes(body);

        assert_eq!(
            String::from_utf8(bytes).expect("utf8"),
            "<?phpx\nexport function renderStore() { return 'ok'; }\n"
        );
    }
}
