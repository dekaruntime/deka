//! deka.gg registry + R2 tarball CDN endpoints for stdlib installs (deka#797).
//!
//! `DEKA_PM_REGISTRY_URL` / `DEKA_PM_STDLIB_CDN` exist solely so tests and CI
//! can serve the deka.gg registry + CDN shape from a local fixture server
//! (deka#797 grant-table tests run the real installer hermetically). They are
//! not a configuration channel — production installs always use deka.gg
//! (deka#801).

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
