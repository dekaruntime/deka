use anyhow::{Context, Result, bail};
use deka_host::integrity::PackageIntegrity;
use reqwest::Url;
use serde_json::Value;

/// Fetch the immutable package digest published with an exact release.
///
/// The digest is deliberately read independently of the download response:
/// the release metadata is the consumer's recorded source of truth. Missing
/// metadata is an error, never a signal to compute and accept a new lock hash.
pub fn fetch_package_digest(
    registry: &str,
    token: Option<&str>,
    name: &str,
    version: &str,
) -> Result<String> {
    let (scope, package) = name
        .strip_prefix('@')
        .and_then(|name| name.split_once('/'))
        .ok_or_else(|| anyhow::anyhow!("invalid scoped package name: {name}"))?;
    let url = format!(
        "{}/api/scoped-packages/{}/{}/{}",
        registry.trim_end_matches('/'),
        scope,
        package,
        version
    );
    let url = Url::parse(&url).context("invalid package release URL")?;
    let client = reqwest::blocking::Client::new();
    let mut request = client.get(url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .context("package release metadata request failed")?;
    let status = response.status();
    let body: Value = response
        .json()
        .context("package release metadata was not valid JSON")?;
    if !status.is_success() {
        bail!("package release metadata request failed ({status})");
    }
    let source = body.get("release").unwrap_or(&body);
    let raw = source
        .get("digest")
        .or_else(|| source.get("package_digest"))
        .or_else(|| source.get("artifact_digest"))
        .or_else(|| source.get("sha256"))
        .or_else(|| source.get("integrity").and_then(|v| v.get("digest")))
        .or_else(|| source.get("integrity").and_then(|v| v.get("fsGraphHash")))
        .or_else(|| source.get("integrity").and_then(|v| v.get("fs_graph_hash")))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("package release metadata is missing its digest"))?;
    normalize_sha256_digest(raw)
}

/// Verify the registry digest against the downloaded package tree.
///
/// Linkhash's PHPX package transport downloads a tree rather than one tarball;
/// its published artifact digest is therefore the package fs-graph digest.
pub fn verify_package_digest(
    name: &str,
    advertised: &str,
    integrity: &PackageIntegrity,
) -> Result<()> {
    if advertised != integrity.fs_graph {
        bail!(
            "package digest mismatch for {name}: registry {}, downloaded {}",
            advertised,
            integrity.fs_graph
        );
    }
    Ok(())
}

fn normalize_sha256_digest(raw: &str) -> Result<String> {
    let digest = raw.strip_prefix("sha256:").unwrap_or(raw).trim();
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("malformed package digest: {raw}");
    }
    Ok(digest.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::normalize_sha256_digest;

    #[test]
    fn accepts_prefixed_sha256() {
        assert_eq!(
            normalize_sha256_digest(&format!("sha256:{}", "a".repeat(64))).unwrap(),
            "a".repeat(64)
        );
    }

    #[test]
    fn rejects_malformed_sha256() {
        assert!(normalize_sha256_digest("sha256:not-a-digest").is_err());
    }
}
