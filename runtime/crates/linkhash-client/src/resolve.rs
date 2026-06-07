//! Package version resolution against the linkhash registry.

use anyhow::{bail, Result};
use serde::Deserialize;

/// A resolved package with exact version and metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
    pub git_ref: Option<String>,
    pub repo: Option<String>,
}

/// Resolve a package to an exact version.
///
/// If `version_range` is "latest" or empty, fetches the latest version.
/// Otherwise, resolves the range against available versions.
pub(crate) fn resolve(
    http: &reqwest::blocking::Client,
    registry_url: &str,
    token: Option<&str>,
    name: &str,
    version_range: &str,
) -> Result<ResolvedPackage> {
    let (scope, pkg_name) = crate::parse_scoped_name(name)?;

    let range = version_range.trim();
    let url = if range.is_empty() || range == "latest" || range == "*" {
        format!(
            "{}/api/scoped-packages/{}/{}/latest",
            registry_url, scope, pkg_name
        )
    } else {
        // Strip semver range prefixes to get the clean version string
        let clean = range
            .trim_start_matches('^')
            .trim_start_matches('~')
            .trim_start_matches(">=");
        // If clean looks like an exact semver (digits.digits.digits), fetch directly.
        // Otherwise use the /resolve?range= endpoint for range queries.
        if is_exact_version(clean) {
            format!(
                "{}/api/scoped-packages/{}/{}/{}",
                registry_url, scope, pkg_name, clean
            )
        } else {
            format!(
                "{}/api/scoped-packages/{}/{}/resolve?range={}",
                registry_url, scope, pkg_name, clean
            )
        }
    };

    let mut req = http.get(&url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }

    let response = req
        .send()
        .map_err(|e| anyhow::anyhow!("resolve request failed for {}: {}", name, e))?;

    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .map_err(|e| anyhow::anyhow!("failed to parse resolve response for {}: {}", name, e))?;

    if !status.is_success() {
        let err = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("not found");
        bail!("failed to resolve {}: {} ({})", name, err, status);
    }

    // The API may return the package info at the top level or nested under "release"
    let source = body.get("release").unwrap_or(&body);

    let version = source
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("resolve response missing version for {}", name))?
        .to_string();

    Ok(ResolvedPackage {
        name: name.to_string(),
        version,
        git_ref: source
            .get("git_ref")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        repo: source
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

/// List all available versions for a package.
pub(crate) fn list_versions(
    http: &reqwest::blocking::Client,
    registry_url: &str,
    token: Option<&str>,
    name: &str,
) -> Result<Vec<String>> {
    let (scope, pkg_name) = crate::parse_scoped_name(name)?;
    let url = format!(
        "{}/api/scoped-packages/{}/{}/versions",
        registry_url, scope, pkg_name
    );

    let mut req = http.get(&url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }

    let response = req
        .send()
        .map_err(|e| anyhow::anyhow!("list versions request failed for {}: {}", name, e))?;

    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .map_err(|e| anyhow::anyhow!("failed to parse versions response for {}: {}", name, e))?;

    if !status.is_success() {
        let err = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("not found");
        bail!("failed to list versions for {}: {} ({})", name, err, status);
    }

    let versions = body
        .get("versions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Ok(versions)
}

/// Returns true if `v` looks like an exact semver (e.g. "1.2.3", "0.1.0").
/// Does not accept range prefixes (^, ~, >=).
fn is_exact_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}
