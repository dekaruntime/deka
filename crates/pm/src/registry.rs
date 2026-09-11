//! deka.gg registry + R2 tarball CDN endpoints for stdlib installs (deka#797).
//!
//! `DEKA_PM_REGISTRY_URL` / `DEKA_PM_STDLIB_CDN` exist solely so tests and CI
//! can serve the deka.gg registry + CDN shape from a local fixture server
//! (deka#797 grant-table tests run the real installer hermetically). They are
//! not a configuration channel — production installs always use deka.gg
//! (deka#801).

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

const DEKA_REGISTRY_URL: &str = "https://deka.gg";
const DEKA_STDLIB_CDN: &str = "https://pub-6d81db17678348abba85f93fde4b4400.r2.dev";

pub fn base_url() -> String {
    std::env::var("DEKA_PM_REGISTRY_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEKA_REGISTRY_URL.to_string())
}

pub fn stdlib_cdn_url() -> String {
    std::env::var("DEKA_PM_STDLIB_CDN")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEKA_STDLIB_CDN.to_string())
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegistryPackage {
    #[serde(default)]
    versions: Vec<String>,
}

pub(crate) fn fetch_package(name: &str) -> Result<(String, RegistryPackage)> {
    let package_name = name.strip_prefix("@deka/").ok_or_else(|| {
        anyhow!(
            "install_from_registry called with non-@deka package: {}",
            name
        )
    })?;
    let registry_url = format!("{}/api/registry/{}.json", base_url(), package_name);
    let response = reqwest::blocking::get(&registry_url)
        .with_context(|| format!("failed to contact deka.gg registry for {}", name))?;
    ensure_lookup_succeeded(name, response.status())?;
    let metadata = response
        .json()
        .with_context(|| format!("failed to parse deka.gg registry metadata for {}", name))?;
    Ok((package_name.to_string(), metadata))
}

fn ensure_lookup_succeeded(name: &str, status: reqwest::StatusCode) -> Result<()> {
    if status == reqwest::StatusCode::NOT_FOUND {
        bail!(
            "package {} not found in deka.gg registry (status {})",
            name,
            status
        );
    }
    if !status.is_success() {
        bail!("registry lookup failed for {}: status {}", name, status);
    }
    Ok(())
}

pub(crate) fn select_version(registry: &RegistryPackage, requested: &str) -> Result<String> {
    let requested = requested.trim();
    if requested != "latest" && requested != "*" && !requested.is_empty() {
        return Ok(requested.trim_start_matches('v').to_string());
    }
    latest_version(&registry.versions)
}

fn latest_version(versions: &[String]) -> Result<String> {
    let mut best: Option<(semver::Version, String)> = None;
    for raw in versions {
        let trimmed = raw.trim().trim_start_matches('v');
        if trimmed.is_empty() {
            continue;
        }
        let parsed = semver::Version::parse(trimmed)
            .with_context(|| format!("registry version `{raw}` is not semver"))?;
        match &best {
            Some((current, _)) if parsed <= *current => {}
            _ => best = Some((parsed, trimmed.to_string())),
        }
    }
    best.map(|(_, version)| version)
        .ok_or_else(|| anyhow!("registry listed no usable versions"))
}

#[cfg(test)]
mod tests {
    use super::{RegistryPackage, ensure_lookup_succeeded, select_version};

    #[test]
    fn lookup_status_distinguishes_missing_from_registry_faults() {
        let missing = ensure_lookup_succeeded("@deka/time", reqwest::StatusCode::NOT_FOUND)
            .expect_err("404 must remain a package failure");
        assert_eq!(
            missing.to_string(),
            "package @deka/time not found in deka.gg registry (status 404 Not Found)"
        );

        let fault = ensure_lookup_succeeded("@deka/time", reqwest::StatusCode::BAD_GATEWAY)
            .expect_err("5xx must remain a registry fault");
        assert_eq!(
            fault.to_string(),
            "registry lookup failed for @deka/time: status 502 Bad Gateway"
        );
    }

    #[test]
    fn select_version_uses_latest_usable_registry_version() {
        let registry = RegistryPackage {
            versions: vec!["0.1.0".into(), "0.1.1".into(), "0.2.0".into()],
        };
        assert_eq!(
            select_version(&registry, "latest").expect("latest"),
            "0.2.0"
        );
        assert_eq!(select_version(&registry, "0.1.1").expect("exact"), "0.1.1");
        assert_ne!(
            select_version(
                &RegistryPackage {
                    versions: vec!["0.1.1".into()],
                },
                "latest"
            )
            .expect("catalog latest"),
            "0.1.0"
        );
    }
}
