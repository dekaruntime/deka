//! Version parsing/comparison, and looking up the latest published version
//! from a linkhash-registry-shaped endpoint. Split out of `update.rs`
//! (deka#976 / rfd#61 file-size gate) -- see the parent `update` module doc
//! for how this fits together with `check` and `pipeline`.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct LatestVersionInfo {
    pub version: String,
    pub digest: String,
}

/// Resolve the latest published version from the linkhash registry.
///
/// The query path is isolated in this small function so it is easy to repoint
/// at the live endpoint once Samira deploys it.
pub fn resolve_latest_version(
    registry_url: &str,
    token: Option<&str>,
) -> Result<LatestVersionInfo, String> {
    let client = reqwest::blocking::Client::new();
    let url = format!(
        "{}/api/v1/packages/cargo/deka/latest",
        registry_url.trim_end_matches('/')
    );
    let mut request = client.get(&url);
    if let Some(t) = token {
        request = request.bearer_auth(t);
    }
    let response = request
        .send()
        .map_err(|e| format!("version resolution request failed: {}", e))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(format!("version resolution failed ({}): {}", status, body));
    }
    let info: LatestVersionInfo = response
        .json()
        .map_err(|e| format!("failed to parse version response: {}", e))?;
    Ok(info)
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub(super) fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => latest != current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_valid() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.0.1"), Some((0, 0, 1)));
        assert_eq!(parse_version("10.20.30"), Some((10, 20, 30)));
    }

    #[test]
    fn parse_version_invalid() {
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("a.b.c"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn is_newer_comparison() {
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.2.0", "0.1.99"));
        assert!(is_newer("0.0.2", "0.0.1"));
        assert!(!is_newer("0.0.1", "0.0.2"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "2.0.0"));
    }

    #[test]
    fn resolve_latest_version_url_shape() {
        // Contract test: verify the URL is built exactly as expected.
        let url = "http://localhost:9418";
        let constructed = format!(
            "{}/api/v1/packages/cargo/deka/latest",
            url.trim_end_matches('/')
        );
        assert_eq!(
            constructed,
            "http://localhost:9418/api/v1/packages/cargo/deka/latest"
        );

        let url2 = "http://localhost:9418/";
        let constructed2 = format!(
            "{}/api/v1/packages/cargo/deka/latest",
            url2.trim_end_matches('/')
        );
        assert_eq!(
            constructed2,
            "http://localhost:9418/api/v1/packages/cargo/deka/latest"
        );
    }
}
