//! Package download via git clone with auth support.

use anyhow::{bail, Result};
use reqwest::Url;
use std::error::Error;
use std::fmt;
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
        Err(api_err) => {
            if api_err.downcast_ref::<UnsafeRegistryPath>().is_some() {
                return Err(api_err);
            }
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

    let files = body
        .get("files")
        .or_else(|| body.get("tree"))
        .or_else(|| body.get("entries"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("tree response missing files array"))?;

    // Step 2: Create target directory
    std::fs::create_dir_all(target_dir)
        .map_err(|e| anyhow::anyhow!("failed to create target dir: {}", e))?;

    // Step 3: Download each file
    for file_entry in files {
        let path = file_entry
            .get("path")
            .or_else(|| file_entry.get("name"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("file entry missing path"))?;

        // Skip directories and .git
        if path.starts_with(".git") {
            continue;
        }
        let safe_path = safe_registry_path(path)?;
        let is_dir = file_entry
            .get("type")
            .or_else(|| file_entry.get("kind"))
            .and_then(|v| v.as_str())
            .map(|t| t == "tree" || t == "dir")
            .unwrap_or(false);
        if is_dir {
            std::fs::create_dir_all(confined_target_path(target_dir, &safe_path)?)?;
            continue;
        }

        let blob_url = Url::parse_with_params(
            &format!(
                "{}/api/scoped-packages/{}/{}/{}/blob",
                registry_url, scope, pkg_name, version
            ),
            [("path", path)],
        )
        .map_err(|e| anyhow::anyhow!("failed to build blob URL for {}: {}", path, e))?;

        let mut blob_req = http.get(blob_url);
        if let Some(t) = token {
            blob_req = blob_req.bearer_auth(t);
        }

        let blob_response = blob_req
            .send()
            .map_err(|e| anyhow::anyhow!("blob request failed for {}: {}", path, e))?;

        if !blob_response.status().is_success() {
            bail!("blob API returned {} for {}", blob_response.status(), path);
        }

        let content_bytes = blob_response
            .bytes()
            .map_err(|e| anyhow::anyhow!("failed to read blob for {}: {}", path, e))?;

        // Registry blob endpoint returns JSON { content, ... }; extract the raw content.
        // Fall back to raw bytes for non-JSON responses.
        let file_bytes: Vec<u8> = match serde_json::from_slice::<serde_json::Value>(&content_bytes)
        {
            Ok(v) if v.get("content").and_then(|c| c.as_str()).is_some() => v
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap()
                .as_bytes()
                .to_vec(),
            _ => content_bytes.to_vec(),
        };

        let file_path = confined_target_path(target_dir, &safe_path)?;
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file_path, &file_bytes)?;
    }

    Ok(())
}

fn safe_registry_path(path: &str) -> Result<PathBuf> {
    if path.is_empty() {
        return Err(UnsafeRegistryPath("registry path is empty".to_string()).into());
    }
    if path.contains('\\') {
        return Err(UnsafeRegistryPath(format!("registry path contains backslash: {path}")).into());
    }
    if path == "." || path.starts_with("./") || path.ends_with("/.") || path.contains("/./") {
        return Err(UnsafeRegistryPath(format!(
            "registry path contains current-dir component: {path}"
        ))
        .into());
    }

    let path = Path::new(path);
    if path.is_absolute() {
        return Err(
            UnsafeRegistryPath(format!("registry path is absolute: {}", path.display())).into(),
        );
    }

    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {
                return Err(UnsafeRegistryPath(format!(
                    "registry path contains current-dir component: {}",
                    path.display()
                ))
                .into())
            }
            Component::ParentDir => {
                return Err(UnsafeRegistryPath(format!(
                    "registry path contains parent-dir component: {}",
                    path.display()
                ))
                .into())
            }
            Component::RootDir => {
                return Err(UnsafeRegistryPath(format!(
                    "registry path contains root component: {}",
                    path.display()
                ))
                .into())
            }
            Component::Prefix(_) => {
                return Err(UnsafeRegistryPath(format!(
                    "registry path contains drive prefix: {}",
                    path.display()
                ))
                .into())
            }
        }
    }

    if safe.as_os_str().is_empty() {
        return Err(UnsafeRegistryPath(format!(
            "registry path has no file component: {}",
            path.display()
        ))
        .into());
    }

    Ok(safe)
}

#[derive(Debug)]
struct UnsafeRegistryPath(String);

impl fmt::Display for UnsafeRegistryPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unsafe registry path: {}", self.0)
    }
}

impl Error for UnsafeRegistryPath {}

fn confined_target_path(target_dir: &Path, safe_path: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(target_dir)
        .map_err(|e| anyhow::anyhow!("failed to create target dir: {}", e))?;
    let target_root = target_dir
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("failed to canonicalize target dir: {}", e))?;
    let file_path = target_root.join(safe_path);

    if !file_path.starts_with(&target_root) {
        bail!(
            "registry path escapes target directory: {}",
            safe_path.display()
        );
    }

    Ok(file_path)
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

    // Copy files from temp to target, excluding .git/.  Always remove the
    // clone, including when validation rejects an artifact entry.
    let copy_result = copy_package_files(&temp_dir, target_dir);
    let _ = std::fs::remove_dir_all(&temp_dir);
    copy_result
}

/// Copy all files from source to target, excluding .git/ directory.
fn copy_package_files(source: &Path, target: &Path) -> Result<()> {
    let package_root = source.canonicalize().map_err(|error| {
        anyhow::anyhow!(
            "failed to canonicalize package source {}: {}",
            source.display(),
            error
        )
    })?;
    copy_package_files_from(source, target, &package_root)
}

fn copy_package_files_from(source: &Path, target: &Path, package_root: &Path) -> Result<()> {
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
        let file_type = entry.file_type()?;

        // Keep the fallback's vendored-module boundary identical to the
        // staging validation in pm: reject this component at every depth,
        // case-insensitively, before symlink handling, recursion, or copying.
        if name_str.eq_ignore_ascii_case("php_modules") {
            bail!(
                "package artifact contains vendored php_modules at {}; packages must declare dependencies in deka.json",
                src_path.display()
            );
        }

        // Git preserves symlinks. Never dereference an artifact link: it can
        // otherwise copy bytes outside the cloned package or turn a vendored
        // php_modules tree into an ordinary directory before validation.
        if file_type.is_symlink() {
            bail!(
                "package artifact contains symlink {}; package artifacts may not contain symlinks",
                src_path.display()
            );
        }

        if file_type.is_dir() {
            let canonical = src_path.canonicalize().map_err(|error| {
                anyhow::anyhow!(
                    "failed to canonicalize package entry {}: {}",
                    src_path.display(),
                    error
                )
            })?;
            if !canonical.starts_with(package_root) {
                bail!(
                    "package artifact directory escapes package source: {}",
                    src_path.display()
                );
            }
            copy_package_files_from(&src_path, &dst_path, package_root)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Query, routing::get, Json, Router};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    #[test]
    fn safe_registry_path_rejects_traversal_and_absolute_paths() {
        for bad_path in [
            "../../x",
            "/tmp/pwned",
            "a/../../b",
            "..\\..\\x",
            "a\\..\\..\\b",
            "./x",
            "a/./b",
        ] {
            assert!(
                safe_registry_path(bad_path).is_err(),
                "expected {bad_path:?} to be rejected"
            );
        }

        assert_eq!(
            safe_registry_path("src/index.phpx").expect("safe path"),
            PathBuf::from("src").join("index.phpx")
        );
    }

    #[test]
    fn malicious_tree_entries_are_rejected_without_escape_or_blob_fetch() {
        for bad_path in [
            "../../x",
            "/tmp/pwned",
            "a/../../b",
            "..\\..\\x",
            "a\\..\\..\\b",
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let target_dir = temp.path().join("php_modules").join("@tana").join("store");
            let outside_file = temp.path().join("x");
            let (base_url, blob_hits, _server) = test_registry(bad_path);

            let error = download(
                &reqwest::blocking::Client::new(),
                &base_url,
                None,
                "@tana/store",
                "1.0.0",
                &target_dir,
            )
            .expect_err("malicious registry path should be rejected");

            assert!(
                error.to_string().contains("unsafe registry path"),
                "unexpected error for {bad_path:?}: {error:#}"
            );
            assert_eq!(
                blob_hits.load(Ordering::SeqCst),
                0,
                "blob endpoint should not be fetched for {bad_path:?}"
            );
            assert!(
                !outside_file.exists(),
                "download escaped target dir for {bad_path:?}"
            );
            assert!(
                !target_dir.join(bad_path).exists(),
                "malicious path was written inside target dir for {bad_path:?}"
            );
        }
    }

    #[test]
    fn blob_path_query_is_url_encoded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target_dir = temp.path().join("pkg");
        let requested_blob_path = Arc::new(Mutex::new(None));
        let registry_path = "dir/file name?.phpx";
        let (base_url, _blob_hits, _server) =
            test_registry_with_observed_blob_path(registry_path, Arc::clone(&requested_blob_path));

        download_via_api(
            &reqwest::blocking::Client::new(),
            &base_url,
            None,
            "tana",
            "store",
            "1.0.0",
            &target_dir,
        )
        .expect("download safe path with reserved URL characters");

        assert_eq!(
            requested_blob_path
                .lock()
                .expect("requested path lock")
                .as_deref(),
            Some(registry_path)
        );
        assert_eq!(
            std::fs::read_to_string(target_dir.join(registry_path)).expect("downloaded file"),
            "encoded"
        );
    }

    #[cfg(unix)]
    #[test]
    fn git_fallback_rejects_package_symlinks_before_copying_targets() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        // download() must attempt the tree API before using git. Point it at a
        // refused local port, then make a fake git clone preserve our fixture
        // links exactly as a real clone would.
        let _environment = fake_git_environment_lock()
            .lock()
            .expect("environment lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let fake_bin = temp.path().join("bin");
        std::fs::create_dir_all(&fake_bin).expect("fake git directory");
        let fake_git = fake_bin.join("git");
        std::fs::write(
            &fake_git,
            "#!/bin/sh\nfor arg do destination=\"$arg\"; done\nmkdir -p \"$destination\"\ncp -a \"$LINKHASH_FAKE_SOURCE\"/. \"$destination\"\n",
        )
        .expect("write fake git");
        let mut permissions = std::fs::metadata(&fake_git)
            .expect("fake git metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_git, permissions).expect("make fake git executable");

        let original_path = std::env::var_os("PATH");
        let mut path = std::ffi::OsString::from(&fake_bin);
        path.push(":");
        path.push(original_path.as_deref().unwrap_or_default());
        std::env::set_var("PATH", &path);

        for (label, target) in [
            (
                "innocuous link into php_modules",
                temp.path().join("php_modules"),
            ),
            (
                "link escaping package root",
                temp.path().join("installer-local-secret"),
            ),
        ] {
            let source = temp.path().join(label.replace(' ', "-"));
            std::fs::create_dir_all(&source).expect("package source");
            std::fs::write(source.join("index.phpx"), "export const safe = true;\n")
                .expect("package file");
            if label.contains("php_modules") {
                std::fs::create_dir_all(&target).expect("outside php_modules");
                std::fs::write(target.join("hidden.phpx"), "outside package\n")
                    .expect("outside php_modules file");
            } else {
                std::fs::write(&target, "installer-local secret\n").expect("outside secret");
            }
            symlink(&target, source.join("ordinary-looking-link")).expect("package symlink");
            std::env::set_var("LINKHASH_FAKE_SOURCE", &source);

            let destination = temp.path().join(format!("installed-{}", label.len()));
            let error = download(
                &reqwest::blocking::Client::new(),
                "http://127.0.0.1:1",
                None,
                "@scope/pkg",
                "1.0.0",
                &destination,
            )
            .expect_err(label);

            assert!(
                error.to_string().contains("contains symlink"),
                "{label}: {error:#}"
            );
            assert!(
                !destination.join("ordinary-looking-link").exists(),
                "{label}: symlink target was copied into the package"
            );
        }

        match original_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
        std::env::remove_var("LINKHASH_FAKE_SOURCE");
    }

    #[cfg(unix)]
    fn fake_git_environment_lock() -> &'static Mutex<()> {
        static LOCK: Mutex<()> = Mutex::new(());
        &LOCK
    }

    struct TestServer {
        shutdown: Option<tokio::sync::oneshot::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn test_registry(path: &str) -> (String, Arc<AtomicUsize>, TestServer) {
        test_registry_with_observed_blob_path(path, Arc::new(Mutex::new(None)))
    }

    fn test_registry_with_observed_blob_path(
        path: &str,
        observed_blob_path: Arc<Mutex<Option<String>>>,
    ) -> (String, Arc<AtomicUsize>, TestServer) {
        let tree_path = Arc::new(path.to_string());
        let blob_hits = Arc::new(AtomicUsize::new(0));

        let tree_route_path = Arc::clone(&tree_path);
        let blob_route_hits = Arc::clone(&blob_hits);
        let blob_observed_path = Arc::clone(&observed_blob_path);
        let app = Router::new()
            .route(
                "/api/scoped-packages/tana/store/1.0.0/tree",
                get(move || {
                    let tree_route_path = Arc::clone(&tree_route_path);
                    async move {
                        Json(json!({
                            "files": [
                                {
                                    "path": tree_route_path.as_str(),
                                    "type": "blob"
                                }
                            ]
                        }))
                    }
                }),
            )
            .route(
                "/api/scoped-packages/tana/store/1.0.0/blob",
                get(move |Query(params): Query<BTreeMap<String, String>>| {
                    let blob_route_hits = Arc::clone(&blob_route_hits);
                    let blob_observed_path = Arc::clone(&blob_observed_path);
                    async move {
                        blob_route_hits.fetch_add(1, Ordering::SeqCst);
                        let path = params.get("path").cloned();
                        *blob_observed_path.lock().expect("observed path lock") = path;
                        Json(json!({ "content": "encoded" }))
                    }
                }),
            );

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind test server");
        listener
            .set_nonblocking(true)
            .expect("set test server nonblocking");
        let addr = listener.local_addr().expect("test server local addr");
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("test runtime");
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .expect("test server");
            });
        });

        (
            format!("http://{addr}"),
            blob_hits,
            TestServer {
                shutdown: Some(shutdown_tx),
                thread: Some(thread),
            },
        )
    }
}
