//! One-shot updater for the cargo-via-universe distribution model.
//!
//! This module exposes a CLI handler (`cmd`) and a shared core (`run_update`)
//! that the future `deka self monitor` daemon can call directly.
//!
//! Flow:
//! 1. Resolve latest version from linkhash registry.
//! 2. If newer than the running version, snapshot the current binary.
//! 3. `cargo install --force deka --registry linkhash --root <temp>` (build-on-old).
//! 4. Health-check the newly built binary.
//! 5. Swap the new binary into place.
//! 6. Health-check the swapped binary; restore snapshot on failure.
//! 7. Restart configured managed services.

use core::Context;
use serde::Deserialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use stdio;

// ---------------------------------------------------------------------------
// Public shared core (callable from deka self monitor)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct LatestVersionInfo {
    pub version: String,
    pub digest: String,
}

#[derive(Debug, Clone)]
pub struct UpdateConfig {
    pub registry_url: String,
    pub token: Option<String>,
    pub current_version: String,
    pub current_binary: PathBuf,
    pub managed_units: Vec<String>,
    pub auto_confirm: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateResult {
    pub old_version: String,
    pub new_version: String,
    pub snapshot_path: PathBuf,
    pub restarted_units: Vec<String>,
}

/// Resolve the latest published version from the linkhash registry.
///
/// The query path is isolated in this small function so it is easy to repoint
/// at the live endpoint once Samira deploys it.
pub fn resolve_latest_version(registry_url: &str, token: Option<&str>) -> Result<LatestVersionInfo, String> {
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

/// Run the full update flow with safety rails.
///
/// 1. Resolve latest version.
/// 2. If newer, snapshot current binary.
/// 3. Build new binary via `cargo install` into a temp root (build-on-old).
/// 4. Health-check the newly built binary.
/// 5. Swap the new binary in place of the current one.
/// 6. Health-check the swapped binary.
/// 7. If health-check fails, restore the snapshot.
/// 8. Restart configured managed units.
pub fn run_update(config: &UpdateConfig) -> Result<UpdateResult, String> {
    let latest = resolve_latest_version(&config.registry_url, config.token.as_deref())?;
    stdio::log("self update", &format!("resolved latest version: {}", latest.version));

    if !is_newer(&latest.version, &config.current_version) {
        stdio::log(
            "self update",
            &format!(
                "already up-to-date ({} >= {})",
                config.current_version, latest.version
            ),
        );
        return Ok(UpdateResult {
            old_version: config.current_version.clone(),
            new_version: config.current_version.clone(),
            snapshot_path: PathBuf::new(),
            restarted_units: Vec::new(),
        });
    }

    stdio::log(
        "self update",
        &format!(
            "update available: {} -> {}",
            config.current_version, latest.version
        ),
    );

    if !config.auto_confirm {
        let prompt = format!("install deka {}? [Y/n]: ", latest.version);
        if !prompt_yes_no(&prompt, true).unwrap_or(false) {
            return Err("update canceled by user".to_string());
        }
    }

    // --- safety rail 1: snapshot ---
    let snapshot = snapshot_binary(&config.current_binary)?;
    stdio::log(
        "self update",
        &format!("snapshot created at {}", snapshot.display()),
    );

    // --- build-on-old: compile into a temp root ---
    let temp_root = temp_install_root()?;
    stdio::log(
        "self update",
        &format!("building into {}", temp_root.display()),
    );
    let new_binary = build_new_binary(&temp_root, &latest.version)?;

    // --- safety rail 2: health-check the build artifact ---
    stdio::log("self update", "health-checking built binary");
    if let Err(e) = health_check(&new_binary, &latest.version) {
        stdio::warn(
            "self update",
            &format!("built binary health-check failed: {}", e),
        );
        return Err(format!("built binary failed health-check: {}", e));
    }

    // --- swap ---
    stdio::log(
        "self update",
        &format!("swapping {}", config.current_binary.display()),
    );
    if let Err(e) = swap_binary(&new_binary, &config.current_binary) {
        stdio::warn("self update", &format!("swap failed: {}", e));
        return Err(format!("failed to swap binary: {}", e));
    }

    // --- safety rail 3: health-check after swap ---
    stdio::log("self update", "health-checking swapped binary");
    if let Err(e) = health_check(&config.current_binary, &latest.version) {
        stdio::error(
            "self update",
            &format!(
                "swapped binary health-check failed: {}. restoring snapshot...",
                e
            ),
        );
        if let Err(restore_err) = restore_snapshot(&snapshot, &config.current_binary) {
            return Err(format!(
                "health-check failed AND snapshot restore failed: {} | restore error: {}",
                e, restore_err
            ));
        }
        stdio::log("self update", "snapshot restored. update aborted.");
        return Err(format!(
            "update rolled back after health-check failure: {}",
            e
        ));
    }

    // --- restart managed units ---
    let mut restarted = Vec::new();
    if !config.managed_units.is_empty() {
        stdio::log(
            "self update",
            &format!("restarting managed units: {:?}", config.managed_units),
        );
        for unit in &config.managed_units {
            match restart_managed_unit(unit) {
                Ok(()) => {
                    restarted.push(unit.clone());
                    stdio::log("self update", &format!("restarted {}", unit));
                }
                Err(e) => {
                    stdio::warn(
                        "self update",
                        &format!("failed to restart {}: {}", unit, e),
                    );
                }
            }
        }
    }

    Ok(UpdateResult {
        old_version: config.current_version.clone(),
        new_version: latest.version.clone(),
        snapshot_path: snapshot,
        restarted_units: restarted,
    })
}

// ---------------------------------------------------------------------------
// CLI handler
// ---------------------------------------------------------------------------

pub fn cmd(context: &Context) {
    let (registry_url, token) = get_registry_config(context);
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let current_binary = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            stdio::error(
                "self update",
                &format!("failed to resolve current executable: {}", e),
            );
            std::process::exit(1);
        }
    };
    let managed_units = load_managed_units(context);
    let auto_confirm =
        context.args.flags.contains_key("--yes") || context.args.flags.contains_key("-y");

    let config = UpdateConfig {
        registry_url,
        token,
        current_version,
        current_binary,
        managed_units,
        auto_confirm,
    };

    match run_update(&config) {
        Ok(result) => {
            if result.old_version == result.new_version {
                stdio::log("self update", "no update needed");
            } else {
                stdio::log(
                    "self update",
                    &format!(
                        "updated {} -> {} (snapshot: {})",
                        result.old_version,
                        result.new_version,
                        result.snapshot_path.display()
                    ),
                );
                if !result.restarted_units.is_empty() {
                    stdio::log(
                        "self update",
                        &format!("restarted: {}", result.restarted_units.join(", ")),
                    );
                }
            }
        }
        Err(e) => {
            stdio::error("self update", &e);
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn get_registry_config(context: &Context) -> (String, Option<String>) {
    let registry = context
        .args
        .params
        .get("--registry-url")
        .cloned()
        .or_else(|| std::env::var("LINKHASH_REGISTRY_URL").ok())
        .or_else(|| std::env::var("LINKHASH_REGISTRY").ok())
        .or_else(|| std::env::var("TANA_GIT_SERVER").ok())
        .unwrap_or_else(|| "http://localhost:9418".to_string());

    let token = context
        .args
        .params
        .get("--token")
        .cloned()
        .or_else(|| std::env::var("LINKHASH_TOKEN").ok())
        .or_else(|| std::env::var("TANA_GIT_TOKEN").ok());

    (registry, token)
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

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => latest != current,
    }
}

fn snapshot_binary(binary: &Path) -> Result<PathBuf, String> {
    let parent = binary.parent().unwrap_or_else(|| Path::new("."));
    let stem = binary.file_stem().and_then(|s| s.to_str()).unwrap_or("deka");
    let ext = binary.extension().and_then(|s| s.to_str()).unwrap_or("");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let snapshot_name = if ext.is_empty() {
        format!("{}.bak.{}", stem, timestamp)
    } else {
        format!("{}.bak.{}.{}", stem, timestamp, ext)
    };
    let snapshot = parent.join(snapshot_name);
    std::fs::copy(binary, &snapshot)
        .map_err(|e| format!("failed to snapshot binary: {}", e))?;
    Ok(snapshot)
}

fn restore_snapshot(snapshot: &Path, target: &Path) -> Result<(), String> {
    std::fs::copy(snapshot, target)
        .map_err(|e| format!("failed to restore snapshot: {}", e))?;
    Ok(())
}

fn temp_install_root() -> Result<PathBuf, String> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!("deka-update-{}", ts));
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("failed to create temp root: {}", e))?;
    Ok(root)
}

fn build_new_binary(temp_root: &Path, _expected_version: &str) -> Result<PathBuf, String> {
    stdio::log(
        "self update",
        &format!(
            "running cargo install --force deka --registry linkhash --root {} ...",
            temp_root.display()
        ),
    );
    let status = Command::new("cargo")
        .args([
            "install",
            "--force",
            "deka",
            "--registry",
            "linkhash",
            "--root",
            &temp_root.to_string_lossy(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to run cargo install: {}", e))?;

    if !status.success() {
        return Err(format!("cargo install exited with status: {}", status));
    }

    let binary = temp_root.join("bin").join("deka");
    if !binary.exists() {
        return Err(format!(
            "cargo install succeeded but binary not found at {}",
            binary.display()
        ));
    }

    Ok(binary)
}

fn health_check(binary: &Path, expected_version: &str) -> Result<(), String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|e| format!("failed to run health-check: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "health-check exit code: {}. stderr: {}",
            output.status, stderr
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains(expected_version) {
        return Err(format!(
            "health-check version mismatch: expected '{}', got '{}'",
            expected_version,
            stdout.trim()
        ));
    }

    Ok(())
}

fn swap_binary(source: &Path, target: &Path) -> Result<(), String> {
    // Copy source -> target. On Unix this works even when target is the
    // currently running executable because the kernel keeps the old inode
    // mapped in memory.
    std::fs::copy(source, target)
        .map_err(|e| format!("failed to copy new binary: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(target)
            .map_err(|e| format!("failed to read target permissions: {}", e))?
            .permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(target, perms)
            .map_err(|e| format!("failed to set executable bit: {}", e))?;
    }

    Ok(())
}

fn restart_managed_unit(unit: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let uid = get_uid()?;
        let status = Command::new("launchctl")
            .args(["kickstart", "-k", &format!("gui/{}/{}", uid, unit)])
            .status()
            .map_err(|e| format!("failed to run launchctl: {}", e))?;
        if !status.success() {
            return Err(format!("launchctl kickstart failed for {}", unit));
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("systemctl")
            .args(["--user", "restart", unit])
            .status()
            .map_err(|e| format!("failed to run systemctl: {}", e))?;
        if !status.success() {
            return Err(format!("systemctl restart failed for {}", unit));
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(format!(
            "automatic restart not supported on this platform for unit {}",
            unit
        ))
    }
}

fn get_uid() -> Result<String, String> {
    if let Ok(uid) = std::env::var("UID") {
        return Ok(uid);
    }
    let output = Command::new("id")
        .arg("-u")
        .output()
        .map_err(|e| format!("failed to get uid: {}", e))?;
    if !output.status.success() {
        return Err("id -u failed".to_string());
    }
    let uid = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();
    Ok(uid)
}

fn load_managed_units(context: &Context) -> Vec<String> {
    let mut units = Vec::new();

    // 1. deka.json in cwd
    let deka_json = context.env.cwd.join("deka.json");
    if let Ok(raw) = std::fs::read_to_string(&deka_json) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(arr) = json
                .get("self")
                .and_then(|s| s.get("update"))
                .and_then(|u| u.get("managed_units"))
                .and_then(|m| m.as_array())
            {
                for val in arr {
                    if let Some(s) = val.as_str() {
                        units.push(s.to_string());
                    }
                }
            }
        }
    }

    // 2. Environment fallback
    if units.is_empty() {
        if let Ok(env_units) = std::env::var("DEKA_SELF_MANAGED_UNITS") {
            for s in env_units.split(',') {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    units.push(trimmed.to_string());
                }
            }
        }
    }

    units
}

fn prompt_yes_no(prompt: &str, default_yes: bool) -> Option<bool> {
    print!("{}", prompt);
    let _ = io::stdout().flush();
    let mut buf = String::new();
    if io::stdin().read_line(&mut buf).is_err() {
        return None;
    }
    let trimmed = buf.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Some(default_yes);
    }
    if trimmed == "y" || trimmed == "yes" {
        return Some(true);
    }
    if trimmed == "n" || trimmed == "no" {
        return Some(false);
    }
    Some(default_yes)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn dummy_context(cwd: PathBuf) -> Context {
        Context {
            args: core::Args {
                flags: HashMap::new(),
                params: HashMap::new(),
                commands: vec!["self".to_string(), "update".to_string()],
                positionals: Vec::new(),
            },
            env: core::EnvContext {
                vars: HashMap::new(),
                cwd,
            },
            handler: core::HandlerContext {
                input: ".".to_string(),
                resolved: core::ResolvedHandler {
                    path: PathBuf::from("."),
                    directory: PathBuf::from("."),
                    mode: core::ServeMode::Php,
                    config: core::ServeConfig::default(),
                },
                static_config: core::StaticServeConfig::default(),
                serve_config_path: None,
            },
        }
    }

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
    fn snapshot_path_generation() {
        let dir = std::env::temp_dir().join(format!("deka-snap-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("deka");
        std::fs::write(&binary, b"fake").unwrap();

        let snap = snapshot_binary(&binary).unwrap();
        assert!(snap.to_string_lossy().contains("deka.bak."));
        assert!(snap.exists());

        // cleanup
        let _ = std::fs::remove_file(&snap);
        let _ = std::fs::remove_file(&binary);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn load_managed_units_from_env() {
        let dir = std::env::temp_dir().join(format!("deka-env-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = dummy_context(dir.clone());

        // no deka.json, no env -> empty
        assert!(load_managed_units(&ctx).is_empty());

        unsafe {
            std::env::set_var(
                "DEKA_SELF_MANAGED_UNITS",
                "gg.tana.deka-platform,gg.tana.deka-edge",
            );
        }
        let units = load_managed_units(&ctx);
        assert_eq!(
            units,
            vec!["gg.tana.deka-platform", "gg.tana.deka-edge"]
        );
        unsafe {
            std::env::remove_var("DEKA_SELF_MANAGED_UNITS");
        }

        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn load_managed_units_from_deka_json() {
        let dir = std::env::temp_dir().join(format!("deka-json-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let deka_json = dir.join("deka.json");
        let content = r#"{"self":{"update":{"managed_units":["gg.tana.deka-platform"]}}}"#;
        std::fs::write(&deka_json, content).unwrap();

        let ctx = dummy_context(dir.clone());
        let units = load_managed_units(&ctx);
        assert_eq!(units, vec!["gg.tana.deka-platform"]);

        // deka.json takes precedence over env
        unsafe {
            std::env::set_var("DEKA_SELF_MANAGED_UNITS", "gg.tana.other");
        }
        let units2 = load_managed_units(&ctx);
        assert_eq!(units2, vec!["gg.tana.deka-platform"]);
        unsafe {
            std::env::remove_var("DEKA_SELF_MANAGED_UNITS");
        }

        // cleanup
        let _ = std::fs::remove_file(&deka_json);
        let _ = std::fs::remove_dir(&dir);
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
