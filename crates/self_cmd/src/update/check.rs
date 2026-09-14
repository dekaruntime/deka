// ---------------------------------------------------------------------------
// Update-check for `deka self update` (deka#976 / rfd#61 / deka#990)
// ---------------------------------------------------------------------------
//
// deka#990 course correction: `deka --update` was a global flag duplicating
// this subcommand, so it is gone -- `cmd` below is now what `deka self
// update` actually runs (wired from the facade `update::cmd` re-export,
// which `self_cmd::lib.rs`'s `self update` SubcommandSpec points at).
//
// This is deliberately independent of `run_update` in the sibling
// `pipeline` module: `run_update` resolves against a linkhash registry URL
// (default `http://localhost:9418`) that is the retired self-hosted
// registry and is not reachable in the current distribution model (see
// CLAUDE.md, "Issue Tracking" and "Distribution"). Before deka#990, running
// the correctly-spelled `deka self update` hit exactly that dead registry
// and failed with a connection error -- `cmd`/`check_latest` do not call
// `run_update`, do not touch `resolve_latest_version`, and perform no
// download or binary swap; they only read the public, unauthenticated
// release manifest that the release workflow already publishes to R2, and
// report the result.

use deka_cli_core::Context;
use stdio;

/// `deka self update`'s handler. Ignores `context` -- the check takes no
/// flags today (no `--registry-url`/`--token`/`--config`, unlike the dead
/// `pipeline::cmd` this replaced).
pub fn cmd(_context: &Context) {
    check_latest();
}

/// Checks `https://releases.deka.gg/latest.json` (via `pm::releases`) and
/// prints plainly whether the running binary is current, or the newer
/// version plus a direct download URL. Never downloads or replaces
/// anything.
pub fn check_latest() {
    let current_version = env!("CARGO_PKG_VERSION");
    let manifest_url = pm::releases::releases_url();
    match compose_update_status(current_version, &manifest_url) {
        Ok(lines) => {
            for line in lines {
                stdio::raw(&line);
            }
        }
        Err(err) => {
            stdio::error(
                "self update",
                &format!("could not check for updates: {}", err),
            );
        }
    }
}

/// The testable core of `check_latest`: given the running version and a
/// manifest URL, returns the lines it would print. Kept separate from
/// `check_latest` so tests inject a local fixture URL directly at this
/// seam instead of going through process env or stdout capture.
fn compose_update_status(current_version: &str, manifest_url: &str) -> Result<Vec<String>, String> {
    let latest =
        pm::releases::fetch_latest_release_from(manifest_url).map_err(|err| err.to_string())?;
    if pm::releases::is_newer(current_version, &latest.version) {
        let mut lines = vec![format!(
            "a newer deka is available: {} -> {}",
            current_version, latest.version
        )];
        lines.push(match pm::releases::download_url(manifest_url, &latest) {
            Some(url) => format!("download: {}", url),
            None => "no prebuilt binary is published for this platform; see https://releases.deka.gg/latest.json".to_string(),
        });
        Ok(lines)
    } else {
        Ok(vec![format!("deka {} is up to date", current_version)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // check_latest wiring (deka#976 / rfd#61): pm::releases itself is
    // covered directly in crates/pm/src/releases.rs; these exercise that
    // self_cmd's own composition (compose_update_status) reaches it and
    // produces the right lines. Injected at the same kind of seam as
    // pm::releases's own tests — an explicit URL, not env vars or stdout
    // capture (stdio::begin_capture/end_capture are wasm32-only).

    fn spawn_manifest_server(body: serde_json::Value) -> (String, tokio::runtime::Runtime) {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let address = rt.block_on(async {
            let app = axum::Router::new().route(
                "/latest.json",
                axum::routing::get(move || async move { axum::Json(body) }),
            );
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

    #[test]
    fn compose_update_status_reports_up_to_date() {
        let key = pm::releases::platform_key().expect("test host has a published platform key");
        let current = env!("CARGO_PKG_VERSION");
        let body = serde_json::json!({
            "version": current,
            "tag": format!("v{}", current),
            "binaries": { key: { "name": format!("deka-{}", key), "sha256": "deadbeef" } }
        });
        let (url, _rt) = spawn_manifest_server(body);
        let lines = compose_update_status(current, &url).expect("compose status");
        assert_eq!(lines, vec![format!("deka {} is up to date", current)]);
    }

    #[test]
    fn compose_update_status_reports_newer_version_and_download_url() {
        let key = pm::releases::platform_key().expect("test host has a published platform key");
        let current = env!("CARGO_PKG_VERSION");
        let body = serde_json::json!({
            "version": "99.0.0",
            "tag": "v99.0.0",
            "binaries": { key: { "name": format!("deka-{}", key), "sha256": "deadbeef" } }
        });
        let (url, _rt) = spawn_manifest_server(body);
        let lines = compose_update_status(current, &url).expect("compose status");
        assert_eq!(
            lines[0],
            format!("a newer deka is available: {} -> 99.0.0", current)
        );
        let expected_base = url.trim_end_matches("/latest.json");
        assert_eq!(
            lines[1],
            format!("download: {}/99.0.0/deka-{}", expected_base, key)
        );
    }

    #[test]
    fn compose_update_status_surfaces_unreachable_manifest_as_an_error() {
        let current = env!("CARGO_PKG_VERSION");
        let err = compose_update_status(current, "http://127.0.0.1:0/latest.json")
            .expect_err("unreachable manifest should error, not silently succeed");
        assert!(!err.is_empty());
    }
}
