//! Package download via git clone with auth support.

use anyhow::{Result, bail};
use std::path::Path;
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
    match download_via_api(http, registry_url, token, &scope, &pkg_name, version, target_dir) {
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

    let response = req.send()
        .map_err(|e| anyhow::anyhow!("tree request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        bail!("tree API returned {}", status);
    }

    let body: serde_json::Value = response.json()
        .map_err(|e| anyhow::anyhow!("failed to parse tree response: {}", e))?;

    let files = body.get("files")
        .or_else(|| body.get("tree"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("tree response missing files array"))?;

    // Step 2: Create target directory
    std::fs::create_dir_all(target_dir)
        .map_err(|e| anyhow::anyhow!("failed to create target dir: {}", e))?;

    // Step 3: Download each file
    for file_entry in files {
        let path = file_entry.get("path")
            .or_else(|| file_entry.get("name"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("file entry missing path"))?;

        // Skip directories and .git
        if path.starts_with(".git") {
            continue;
        }
        let is_dir = file_entry.get("type")
            .and_then(|v| v.as_str())
            .map(|t| t == "tree" || t == "dir")
            .unwrap_or(false);
        if is_dir {
            std::fs::create_dir_all(target_dir.join(path))?;
            continue;
        }

        let blob_url = format!(
            "{}/api/scoped-packages/{}/{}/{}/blob?path={}",
            registry_url, scope, pkg_name, version, path
        );

        let mut blob_req = http.get(&blob_url);
        if let Some(t) = token {
            blob_req = blob_req.bearer_auth(t);
        }

        let blob_response = blob_req.send()
            .map_err(|e| anyhow::anyhow!("blob request failed for {}: {}", path, e))?;

        if !blob_response.status().is_success() {
            bail!("blob API returned {} for {}", blob_response.status(), path);
        }

        let content = blob_response.bytes()
            .map_err(|e| anyhow::anyhow!("failed to read blob for {}: {}", path, e))?;

        let file_path = target_dir.join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file_path, &content)?;
    }

    Ok(())
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
    let temp_dir = std::env::temp_dir().join(format!("linkhash-dl-{}-{}-{}", scope, pkg_name, version));
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

    let status = cmd.status()
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
