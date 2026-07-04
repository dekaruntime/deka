//! Linkhash package registry client with built-in auth support.
//!
//! Handles resolve, download, preflight, and publish operations for
//! scoped PHPX packages against a linkhash-compatible registry.

mod download;
mod resolve;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

pub use resolve::ResolvedPackage;

/// A client for the linkhash package registry.
pub struct LinkhashClient {
    registry_url: String,
    token: Option<String>,
    http: reqwest::blocking::Client,
}

/// Request body for publish/preflight operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishRequest {
    pub name: String,
    pub version: String,
    pub repo: String,
    pub git_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<serde_json::Value>,
}

/// Result of a preflight check.
#[derive(Debug, Clone, Deserialize)]
pub struct PreflightResult {
    pub allowed: bool,
    pub required_bump: Option<String>,
    pub minimum_allowed_version: Option<String>,
    pub reasons: Option<Vec<String>>,
}

/// Result of a publish operation.
#[derive(Debug, Clone, Deserialize)]
pub struct PublishResult {
    pub package_name: String,
    pub version: String,
}

impl LinkhashClient {
    /// Create a new client. Token is optional for future public package support.
    pub fn new(registry_url: &str, token: Option<&str>) -> Self {
        Self {
            registry_url: registry_url.trim_end_matches('/').to_string(),
            token: token.map(|t| t.to_string()),
            http: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new()),
        }
    }

    /// Resolve a package version. Returns the exact version and metadata.
    pub fn resolve(&self, name: &str, version_range: &str) -> Result<ResolvedPackage> {
        resolve::resolve(
            &self.http,
            &self.registry_url,
            self.token.as_deref(),
            name,
            version_range,
        )
    }

    /// List all versions of a package.
    pub fn list_versions(&self, name: &str) -> Result<Vec<String>> {
        resolve::list_versions(&self.http, &self.registry_url, self.token.as_deref(), name)
    }

    /// Download a package to a target directory using the tree/blob API.
    pub fn download(&self, name: &str, version: &str, target_dir: &Path) -> Result<()> {
        download::download(
            &self.http,
            &self.registry_url,
            self.token.as_deref(),
            name,
            version,
            target_dir,
        )
    }

    /// Preflight a publish — dry-run validation.
    pub fn preflight(&self, req: &PublishRequest) -> Result<PreflightResult> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("auth token required for preflight"))?;

        let url = format!("{}/api/packages/preflight", self.registry_url);
        let response = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(req)
            .send()
            .map_err(|e| anyhow::anyhow!("preflight request failed: {}", e))?;

        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .map_err(|e| anyhow::anyhow!("failed to parse preflight response: {}", e))?;

        if !status.is_success() {
            let err = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            bail!("preflight failed ({}): {}", status, err);
        }

        if let Some(preflight) = body.get("preflight") {
            let result = PreflightResult {
                allowed: preflight
                    .get("allowed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                required_bump: preflight
                    .get("required_bump")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                minimum_allowed_version: preflight
                    .get("minimum_allowed_version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                reasons: preflight
                    .get("reasons")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    }),
            };
            Ok(result)
        } else {
            Ok(PreflightResult {
                allowed: true,
                required_bump: None,
                minimum_allowed_version: None,
                reasons: None,
            })
        }
    }

    /// Publish a package to the registry.
    pub fn publish(&self, req: &PublishRequest) -> Result<PublishResult> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("auth token required for publish"))?;

        let url = format!("{}/api/packages/publish", self.registry_url);
        let response = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(req)
            .send()
            .map_err(|e| anyhow::anyhow!("publish request failed: {}", e))?;

        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .map_err(|e| anyhow::anyhow!("failed to parse publish response: {}", e))?;

        if !status.is_success() {
            let err = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            bail!("publish failed ({}): {}", status, err);
        }

        let release = body
            .get("release")
            .ok_or_else(|| anyhow::anyhow!("publish response missing 'release' field"))?;

        Ok(PublishResult {
            package_name: release
                .get("package_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            version: release
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }
}

/// Parse a scoped package name like `@scope/name` into `("scope", "name")`.
pub fn parse_scoped_name(name: &str) -> Result<(String, String)> {
    if !name.starts_with('@') {
        bail!("package name must start with @: {}", name);
    }
    let without_at = &name[1..];
    // Strip version suffix if present (@scope/name@1.0.0)
    let base = if let Some(idx) = without_at.find('@') {
        &without_at[..idx]
    } else {
        without_at
    };
    let mut parts = base.split('/');
    let scope = parts.next().unwrap_or("").to_string();
    let pkg = parts.next().unwrap_or("").to_string();
    if scope.is_empty() || pkg.is_empty() || parts.next().is_some() {
        bail!(
            "invalid scoped package name: {} (expected @scope/name)",
            name
        );
    }
    Ok((scope, pkg))
}

/// Check if a package spec looks like a scoped PHPX package.
pub fn is_phpx_package(name: &str) -> bool {
    name.starts_with('@')
}
