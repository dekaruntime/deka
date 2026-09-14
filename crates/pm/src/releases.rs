//! Release-metadata check for `deka --update` (deka#976).
//!
//! This module answers exactly one question honestly: "is a newer deka
//! published, and if so where do I get it?" It does not download or replace
//! the running binary — see PUBLISH.md for the manual install path, and the
//! standing direction (tana-cli-core) for the future shared self-update
//! utility that would perform the swap.
//!
//! The release manifest is the same public, unauthenticated R2-backed JSON
//! document the release workflow already promotes to `latest.json`
//! (`.github/workflows/release.yml`, "Promote latest release manifest").
//! Nothing here needs a token or a new network dependency: `pm` already
//! talks to `deka.gg` over `reqwest::blocking` for stdlib installs
//! (see `registry.rs`), and this follows the identical pattern.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashMap;

const RELEASES_LATEST_URL: &str = "https://releases.deka.gg/latest.json";

/// Test-injection env var, in the same sanctioned category as
/// `DEKA_PM_REGISTRY_URL` / `DEKA_PM_STDLIB_CDN` (see `registry.rs`): it
/// lets tests point at a local fixture server. Production always resolves
/// `RELEASES_LATEST_URL` directly; no product code treats this as config.
const RELEASES_URL_ENV: &str = "DEKA_PM_RELEASES_URL";

#[derive(Debug, Clone, Deserialize)]
pub struct BinaryInfo {
    pub name: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LatestRelease {
    pub version: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub tag: String,
    pub binaries: HashMap<String, BinaryInfo>,
}

pub fn releases_url() -> String {
    std::env::var(RELEASES_URL_ENV)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| RELEASES_LATEST_URL.to_string())
}

/// The platform key the release manifest uses for the binary this process
/// was built for, e.g. `"linux-x64"`. `None` if this target has no
/// published binary (the manifest only ever lists linux-x64/darwin-x64/
/// darwin-arm64 today).
pub fn platform_key() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x64"),
        ("macos", "x86_64") => Some("darwin-x64"),
        ("macos", "aarch64") => Some("darwin-arm64"),
        _ => None,
    }
}

/// Fetch the published release manifest, resolving the URL from
/// [`releases_url`] (production default, or the test-injection override).
pub fn fetch_latest_release() -> Result<LatestRelease> {
    fetch_latest_release_from(&releases_url())
}

/// Fetch and parse a release manifest from an explicit URL. This is the
/// seam tests inject a local fixture server into, so the current-version
/// and newer-version-available cases can be exercised without touching the
/// live network or faking `fetch_latest_release` itself.
pub fn fetch_latest_release_from(url: &str) -> Result<LatestRelease> {
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("failed to reach release manifest at {}", url))?;
    let status = response.status();
    if !status.is_success() {
        bail!("release manifest request to {} failed: {}", url, status);
    }
    response
        .json()
        .with_context(|| format!("failed to parse release manifest from {}", url))
}

/// `true` if `latest` is a newer version than `current`. Falls back to a
/// plain string inequality when either version fails to parse as semver, so
/// a malformed manifest still reports "different" rather than silently
/// claiming the running binary is current.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(latest),
    ) {
        (Ok(current), Ok(latest)) => latest > current,
        _ => current != latest,
    }
}

/// The direct download URL for this platform's binary in `release`, if the
/// manifest published one. Built from the same versioned-release URL
/// pattern documented in PUBLISH.md (`https://releases.deka.gg/<VERSION>/...`),
/// derived from wherever the manifest itself was fetched so test fixtures
/// stay self-contained.
pub fn download_url(manifest_url: &str, release: &LatestRelease) -> Option<String> {
    let key = platform_key()?;
    let binary = release.binaries.get(key)?;
    let base = manifest_url.trim_end_matches("/latest.json");
    Some(format!("{}/{}/{}", base, release.version, binary.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::get};
    use serde_json::json;

    fn spawn_manifest_server(body: serde_json::Value) -> (String, tokio::runtime::Runtime) {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let address = rt.block_on(async {
            let app = Router::new().route("/latest.json", get(move || async move { Json(body) }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind fixture listener");
            let address = listener.local_addr().expect("fixture address");
            tokio::spawn(async move {
                axum::serve(listener, app).await.expect("fixture server");
            });
            address
        });
        (format!("http://{}/latest.json", address), rt)
    }

    fn sample_manifest(version: &str) -> serde_json::Value {
        let key = platform_key().expect("test host has a published platform key");
        json!({
            "version": version,
            "tag": format!("v{}", version),
            "binaries": {
                key: { "name": format!("deka-{}", key), "sha256": "deadbeef" }
            }
        })
    }

    #[test]
    fn reports_up_to_date_when_manifest_matches_running_version() {
        let (url, _rt) = spawn_manifest_server(sample_manifest("0.53.0"));
        let release = fetch_latest_release_from(&url).expect("fetch manifest");
        assert_eq!(release.version, "0.53.0");
        assert!(!is_newer("0.53.0", &release.version));
    }

    #[test]
    fn reports_newer_version_and_builds_a_download_url() {
        let (url, _rt) = spawn_manifest_server(sample_manifest("9.9.9"));
        let release = fetch_latest_release_from(&url).expect("fetch manifest");
        assert!(is_newer("0.53.0", &release.version));
        let download = download_url(&url, &release).expect("download url for current platform");
        let key = platform_key().unwrap();
        let expected_base = url.trim_end_matches("/latest.json");
        assert_eq!(download, format!("{}/9.9.9/deka-{}", expected_base, key));
    }

    #[test]
    fn older_running_version_than_manifest_is_not_newer_in_reverse() {
        assert!(!is_newer("1.2.0", "1.1.0"));
        assert!(is_newer("1.1.0", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2.0"));
    }

    #[test]
    fn non_semver_versions_fall_back_to_string_inequality() {
        assert!(is_newer("dev", "0.53.0"));
        assert!(!is_newer("0.53.0", "0.53.0"));
    }
}
