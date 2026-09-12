use anyhow::Result;
use core::{CommandSpec, Context, FlagSpec, ParamSpec, Registry};
use pm::{InstallPayload, run_install};
use runtime_core::module_spec::canonical_php_package_spec;
use runtime_core::modules::MODULES_DIR;
use std::path::{Path, PathBuf};
use stdio;

const INSTALL_COMMAND: CommandSpec = CommandSpec {
    name: "install",
    category: "package",
    summary: "install dependencies via the package manager",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

const ADD_COMMAND: CommandSpec = CommandSpec {
    name: "add",
    category: "package",
    summary: "install package(s) from the index",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

const I_COMMAND: CommandSpec = CommandSpec {
    name: "i",
    category: "package",
    summary: "install package(s) from the index",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

const UPDATE_COMMAND: CommandSpec = CommandSpec {
    name: "update",
    category: "package",
    summary: "update dependencies to latest within semver range",
    aliases: &[],
    subcommands: &[],
    handler: cmd_update,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(INSTALL_COMMAND);
    registry.add_command(ADD_COMMAND);
    registry.add_command(I_COMMAND);
    registry.add_command(UPDATE_COMMAND);
    registry.add_flag(FlagSpec {
        name: "--quiet",
        aliases: &["-q"],
        description: "suppress informational output",
    });
    registry.add_flag(FlagSpec {
        name: "--yes",
        aliases: &["-y"],
        description: "assume yes when prompted",
    });
    registry.add_flag(FlagSpec {
        name: "--prompt",
        aliases: &["-p"],
        description: "prompt before installing",
    });
    registry.add_flag(FlagSpec {
        name: "--rehash",
        aliases: &[],
        description: "rehash php package integrity and update deka.lock",
    });
    registry.add_flag(FlagSpec {
        name: "--locked",
        aliases: &[],
        description: "fail if deka.lock is missing or would change",
    });
    registry.add_param(ParamSpec {
        name: "--payload",
        description: "path to a JSON payload describing the install",
    });
    registry.add_param(ParamSpec {
        name: "--spec",
        description: "package spec or comma-separated list of specs",
    });
    registry.add_param(ParamSpec {
        name: "--concurrency",
        description: "number of concurrent downloads (ignored for now)",
    });
    registry.add_param(ParamSpec {
        name: "--registry",
        description: "registry base URL (default: https://git.tana.gg)",
    });
    registry.add_param(ParamSpec {
        name: "--token",
        description: "auth token",
    });
}

pub fn cmd(context: &Context) {
    if context.args.flags.contains_key("--rehash") {
        let project_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let specs = rehash_specs(context);
        match rehash_phpx_packages(&project_dir, &specs) {
            Ok(packages) => {
                stdio::log("install", &format!("rehashed {}", packages.join(", ")));
            }
            Err(err) => {
                stdio::error("install", &format!("rehash failed: {}", err));
                std::process::exit(1);
            }
        }
        return;
    }

    let explicit_specs = install_explicit_specs(context);
    let specs = explicit_specs.clone();

    let payload = build_payload_for_specs(context, specs);

    match payload {
        Ok(payload) => {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            if let Err(err) = runtime.block_on(run_install(payload)) {
                let message = err.to_string();
                stdio::error("install", &message);
                std::process::exit(1);
            }
        }
        Err(err) => {
            let message = err.to_string();
            stdio::error("install", &message);
            std::process::exit(1);
        }
    }
}

fn install_explicit_specs(context: &Context) -> Vec<String> {
    context
        .args
        .params
        .get("--spec")
        .map(|value| parse_spec_list(value))
        .filter(|specs| !specs.is_empty())
        .unwrap_or_else(|| context.args.positionals.clone())
}

pub fn cmd_update(context: &Context) {
    // Shop-mode: cwd is inside store/tenants/{shop_id}/. Run the per-shop
    // flow (bump, build-verify, git commit with a version-diff message)
    // so a downstream post-commit hook can redeploy via #84.
    if let Some(shop_dir) = detect_shop_working_tree() {
        if let Err(err) = run_shop_update(context, &shop_dir) {
            stdio::error("update", &err);
            std::process::exit(1);
        }
        return;
    }

    // For update, check if positionals or deka.json deps contain scoped packages
    let mut specs = context.args.positionals.clone();
    if specs.is_empty() {
        specs = collect_deka_json_deps();
    }

    let phpx_specs: Vec<String> = specs
        .iter()
        .filter(|s| is_phpx_package(s))
        .cloned()
        .collect();

    if !phpx_specs.is_empty() {
        stdio::error(
            "update",
            "legacy linkhash/harar registry support has been removed. Use @deka/* stdlib packages or publish to GitHub.",
        );
        std::process::exit(1);
    }

    match build_update_payload(context) {
        Ok(payload) => {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            if let Err(err) = runtime.block_on(run_install(payload)) {
                let message = err.to_string();
                stdio::error("update", &message);
                std::process::exit(1);
            }
        }
        Err(err) => {
            let message = err.to_string();
            stdio::error("update", &message);
            std::process::exit(1);
        }
    }
}

/// Returns the shop working-tree root when cwd is inside `store/tenants/<id>/`
/// with a deka.json + git repo, otherwise None.
fn detect_shop_working_tree() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let mut dir = cwd.as_path();
    // Walk up looking for deka.json, then confirm the path contains
    // `.../store/tenants/<id>`.
    loop {
        if dir.join("deka.json").is_file() {
            // Path must contain /store/tenants/ somewhere above us.
            let s = dir.to_string_lossy();
            if s.contains("/store/tenants/") || s.contains("\\store\\tenants\\") {
                // And must be a git working tree.
                if dir.join(".git").exists() {
                    return Some(dir.to_path_buf());
                }
            }
            return None;
        }
        dir = dir.parent()?;
    }
}

/// Per-shop update: bump deps via the same linkhash-client flow as the
/// generic update, but record before/after versions, verify with a build,
/// and commit the lock/json on success with an author identifying the
/// platform-automated bump.
fn run_shop_update(context: &Context, project_dir: &std::path::Path) -> Result<(), String> {
    let shop_id = project_dir
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("<shop>")
        .to_string();

    stdio::log(
        "update",
        &format!("shop-mode: bumping deps for {}", shop_id),
    );

    Err(
        "shop-mode update depends on the legacy linkhash/harar registry, which has been removed"
            .to_string(),
    )
}

/// Read the current package version map from deka.lock in `dir`.
/// Returns `{ "@deka/cache": "0.1.0", ... }`.
fn read_php_lock_versions(dir: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let path = dir.join("deka.lock");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return out;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return out;
    };
    let Some(pkgs) = lock_packages(&json) else {
        return out;
    };
    for (name, entry) in pkgs {
        // Lock entries are [version, resolved, metadata, integrity].
        if let Some(v) = entry
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
        {
            out.insert(name.clone(), v.to_string());
        }
    }
    out
}

/// Diff the before/after version maps into a sorted list of
/// `(package, Option<before>, after)` tuples. Includes adds and changes;
/// omits untouched packages.
fn diff_versions(
    before: &std::collections::BTreeMap<String, String>,
    after: &std::collections::BTreeMap<String, String>,
) -> Vec<(String, Option<String>, String)> {
    let mut out = Vec::new();
    for (name, new_v) in after {
        match before.get(name) {
            Some(old_v) if old_v == new_v => continue,
            Some(old_v) => out.push((name.clone(), Some(old_v.clone()), new_v.clone())),
            None => out.push((name.clone(), None, new_v.clone())),
        }
    }
    out
}

/// Render a version diff as a single-line summary, e.g.
/// `@deka/cache 0.1.0 -> 0.2.0, @tana/store 0.1.0 -> 0.2.0`.
fn format_diff_line(diff: &[(String, Option<String>, String)]) -> String {
    diff.iter()
        .map(|(name, before, after)| match before {
            Some(b) => format!("{} {} -> {}", name, b, after),
            None => format!("{} added@{}", name, after),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Collect scoped deps declared in `<dir>/deka.json`. Mirrors
/// `collect_deka_json_deps` but takes an explicit project directory
/// instead of using cwd.
fn collect_deka_json_deps_in(dir: &std::path::Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(dir.join("deka.json")) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let Some(deps) = json.get("dependencies").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    deps.iter()
        .map(|(name, version)| {
            if let Some(v) = version.as_str() {
                let clean = v
                    .trim_start_matches('^')
                    .trim_start_matches('~')
                    .trim_start_matches(">=");
                format!("{}@{}", name, clean)
            } else {
                name.clone()
            }
        })
        .collect()
}

/// Run `deka build` on the shop tree as a post-bump verification step.
/// We invoke the single-file build via the in-process build helper
/// instead of shelling out to the deka binary so tests and reverts are
/// deterministic.
fn run_verify_build(project_dir: &std::path::Path) -> Result<(), String> {
    // Shell out to deka build — the in-process entry assumes a populated
    // context, while this path is called from the CLI after the context
    // is already committed to the update flow. Using the current
    // executable keeps behavior aligned with what a human would see.
    let exe = std::env::current_exe().map_err(|e| format!("resolve self: {}", e))?;
    let out = std::process::Command::new(&exe)
        .arg("build")
        .current_dir(project_dir)
        .output()
        .map_err(|e| format!("spawn deka build: {}", e))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(stderr
            .lines()
            .next()
            .unwrap_or("unknown build failure")
            .to_string())
    }
}

/// Revert deka.json + deka.lock in the shop working tree to the git HEAD
/// state. Best-effort — if git is missing the caller still surfaces the
/// underlying failure.
fn revert_shop_update(project_dir: &std::path::Path) {
    let _ = std::process::Command::new("git")
        .args(["checkout", "HEAD", "--", "deka.json", "deka.lock"])
        .current_dir(project_dir)
        .output();
}

/// Commit deka.json + deka.lock with an identifying author so downstream
/// systems (branch protection, audit log, broadcast telemetry) can tell
/// platform-automated bumps from human commits.
fn git_commit_shop_update(project_dir: &std::path::Path, subject: &str) -> Result<(), String> {
    // Stage the lock + manifest. Use explicit paths — never `git add -A`.
    let add = std::process::Command::new("git")
        .args(["add", "deka.json", "deka.lock"])
        .current_dir(project_dir)
        .output()
        .map_err(|e| format!("git add: {}", e))?;
    if !add.status.success() {
        return Err(format!(
            "git add failed: {}",
            String::from_utf8_lossy(&add.stderr)
        ));
    }

    // Commit with deka-update as author. The committer defaults to the
    // repo's configured identity so audit logs still attribute the push.
    let commit = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=deka-update",
            "-c",
            "user.email=platform@tana.gg",
            "commit",
            "--author",
            "deka-update <platform@tana.gg>",
            "-m",
            subject,
        ])
        .current_dir(project_dir)
        .output()
        .map_err(|e| format!("git commit: {}", e))?;

    if !commit.status.success() {
        let stderr = String::from_utf8_lossy(&commit.stderr);
        // If there is nothing to commit, treat as soft no-op.
        if stderr.contains("nothing to commit") {
            return Ok(());
        }
        return Err(format!("git commit failed: {}", stderr));
    }
    Ok(())
}

fn build_payload_for_specs(context: &Context, specs: Vec<String>) -> Result<InstallPayload> {
    let mut payload = InstallPayload {
        specs,
        yes: false,
        prompt: false,
        quiet: false,
        rehash: false,
        locked: false,
    };

    let mut resolved_specs = Vec::new();
    for spec in &payload.specs {
        resolved_specs.push(resolve_php_spec(spec)?);
    }
    payload.specs = resolved_specs;

    apply_flags(&mut payload, context);
    Ok(payload)
}

fn build_update_payload(context: &Context) -> Result<InstallPayload> {
    // For update: read specs from positionals or deka.json dependencies.
    let mut specs = context.args.positionals.clone();
    if specs.is_empty() {
        if let Some(s) = context.args.params.get("--spec") {
            specs = parse_spec_list(s);
        }
    }

    // If no specs given, collect from deka.json dependencies
    if specs.is_empty() {
        specs = collect_deka_json_deps();
    }

    let mut resolved = Vec::new();
    for spec in &specs {
        resolved.push(resolve_php_spec(spec)?);
    }
    specs = resolved;

    let mut payload = InstallPayload {
        specs,
        yes: false,
        prompt: false,
        quiet: false,
        rehash: false,
        locked: false,
    };
    apply_flags(&mut payload, context);
    Ok(payload)
}

fn collect_deka_json_deps() -> Vec<String> {
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let raw = match std::fs::read_to_string(cwd.join("deka.json")) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let json: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(j) => j,
        Err(_) => return Vec::new(),
    };
    let deps = match json.get("dependencies").and_then(|v| v.as_object()) {
        Some(d) => d,
        None => return Vec::new(),
    };
    deps.iter()
        .map(|(name, version)| {
            if let Some(v) = version.as_str() {
                // Strip semver range prefixes for resolution
                let clean = v
                    .trim_start_matches('^')
                    .trim_start_matches('~')
                    .trim_start_matches(">=");
                format!("{}@{}", name, clean)
            } else {
                name.clone()
            }
        })
        .collect()
}

fn apply_flags(payload: &mut InstallPayload, context: &Context) {
    if context.args.flags.contains_key("--yes") || context.args.flags.contains_key("-y") {
        payload.yes = true;
    }
    if context.args.flags.contains_key("--prompt") || context.args.flags.contains_key("-p") {
        payload.prompt = true;
    }
    if context.args.flags.contains_key("--quiet") || context.args.flags.contains_key("-q") {
        payload.quiet = true;
    }
    if context.args.flags.contains_key("--rehash") {
        payload.rehash = true;
    }
    if context.args.flags.contains_key("--locked") {
        payload.locked = true;
    }
}

fn parse_spec_list(value: &str) -> Vec<String> {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

fn resolve_php_spec(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(trimmed.to_string());
    }
    if trimmed.starts_with('@') {
        if is_valid_scoped_name(trimmed) {
            return Ok(trimmed.to_string());
        }
        return Err(anyhow::anyhow!(
            "invalid php package `{}` (expected @scope/name[@version])",
            trimmed
        ));
    }

    if let Some(mapped) = canonical_php_package_spec(trimmed) {
        return Ok(mapped);
    }
    Err(anyhow::anyhow!(
        "unscoped php package `{}` is not allowed. use @scope/name (bare names map to @deka/*)",
        trimmed
    ))
}

/// Extract registry URL and token from CLI flags (deka#801: config flows
/// explicitly through flags/deka.json/auth profile, never the environment).
#[doc(hidden)]
pub fn get_registry_config(context: &Context) -> (String, Option<String>) {
    let registry = context
        .args
        .params
        .get("--registry")
        .cloned()
        .unwrap_or_else(|| "https://git.tana.gg".to_string());

    let token = context.args.params.get("--token").cloned();

    (registry, token)
}

/// Parse a spec like `@tana/store@1.0.0` into `("@tana/store", "1.0.0")`.
/// If no version suffix, returns `("@tana/store", "latest")`.
fn parse_spec_with_version(spec: &str) -> (String, String) {
    if !spec.starts_with('@') {
        return (spec.to_string(), "latest".to_string());
    }
    // @scope/name@version — find the second @
    if let Some(idx) = spec[1..].find('@') {
        let name = spec[..idx + 1].to_string();
        let version = spec[idx + 2..].to_string();
        (name, version)
    } else {
        (spec.to_string(), "latest".to_string())
    }
}

fn rehash_specs(context: &Context) -> Vec<String> {
    let mut specs = context
        .args
        .params
        .get("--spec")
        .map(|value| parse_spec_list(value))
        .unwrap_or_default();
    if specs.is_empty() {
        specs = context.args.positionals.clone();
    }
    specs
}

fn rehash_phpx_packages(project_dir: &Path, specs: &[String]) -> Result<Vec<String>> {
    let versions = read_php_lock_versions(project_dir);
    let packages = if specs.is_empty() {
        versions.keys().cloned().collect::<Vec<_>>()
    } else {
        let mut packages = Vec::new();
        for spec in specs {
            let resolved = resolve_php_spec(spec)?;
            let (name, _) = parse_spec_with_version(&resolved);
            packages.push(name);
        }
        packages
    };

    if packages.is_empty() {
        return Err(anyhow::anyhow!("no packages found to rehash"));
    }

    for name in &packages {
        let version = versions
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("{} is missing from deka.lock", name))?;
        let target = project_dir.join(MODULES_DIR).join(name);
        if !target.is_dir() {
            return Err(anyhow::anyhow!(
                "{} is missing from ds_modules at {}",
                name,
                target.display()
            ));
        }
        let integrity = deka_host::integrity::compute_package_integrity(&target)
            .map_err(|err| anyhow::anyhow!("integrity hash failed for {}: {}", name, err))?;
        update_deka_lock(
            project_dir,
            name,
            version,
            &integrity.module_graph,
            &integrity.fs_graph,
        )?;
    }

    Ok(packages)
}

/// Returns true for scoped package names like `@deka/core` or `@tana/app`.
fn is_phpx_package(spec: &str) -> bool {
    let trimmed = spec.trim();
    if !trimmed.starts_with('@') {
        return false;
    }
    // Strip an optional version suffix: @scope/name@version
    let name_part = if let Some(idx) = trimmed[1..].find('@') {
        &trimmed[..idx + 1]
    } else {
        trimmed
    };
    name_part.contains('/')
}

fn copy_dir_all(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_all(&source_path, &target_path)?;
        } else {
            std::fs::copy(source_path, target_path)?;
        }
    }
    Ok(())
}

/// Update deka.lock with the installed package version.
///
/// The lock format expected by the module validator is:
/// `{ "lockfileVersion": 1, "packages": {...} }`.
/// Each entry value is a 4-tuple `[descriptor, resolved, metadata, integrity]`.
fn update_deka_lock(
    project_dir: &std::path::Path,
    name: &str,
    version: &str,
    module_graph_hash: &str,
    fs_graph_hash: &str,
) -> Result<()> {
    let lock_path = project_dir.join("deka.lock");
    let mut lock: serde_json::Value = if lock_path.exists() {
        let raw = std::fs::read_to_string(&lock_path)?;
        serde_json::from_str(&raw).unwrap_or_else(|_| default_lock())
    } else {
        default_lock()
    };

    let mut packages = lock_packages(&lock).cloned().unwrap_or_default();
    packages.insert(
        name.to_string(),
        serde_json::json!([
            version,
            format!("linkhash:{}", name),
            {
                "moduleGraph": { "hash": module_graph_hash },
                "fsGraph": { "hash": fs_graph_hash },
            },
            "",
        ]),
    );

    lock = serde_json::json!({
        "lockfileVersion": lock
            .get("lockfileVersion")
            .and_then(|value| value.as_u64())
            .unwrap_or(1),
        "packages": packages,
    });

    std::fs::write(&lock_path, serde_json::to_string_pretty(&lock)?)?;
    Ok(())
}

fn lock_packages(lock: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
    if let Some(packages) = lock.get("packages").and_then(|value| value.as_object()) {
        return Some(packages);
    }

    lock.get("php")
        .and_then(|value| value.get("packages"))
        .and_then(|value| value.as_object())
}

fn default_lock() -> serde_json::Value {
    serde_json::json!({
        "lockfileVersion": 1,
        "packages": {},
    })
}

fn is_valid_scoped_name(spec: &str) -> bool {
    let without_version = if let Some(idx) = spec[1..].find('@') {
        &spec[..idx + 1]
    } else {
        spec
    };
    let mut parts = without_version.split('/');
    let scope = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return false;
    }
    if !scope.starts_with('@') || scope.len() <= 1 || name.is_empty() {
        return false;
    }
    scope[1..]
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

#[cfg(test)]
mod shop_update_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn lock_with(pkgs: &[(&str, &str)]) -> String {
        let mut packages = serde_json::Map::new();
        for (name, version) in pkgs {
            packages.insert(
                (*name).to_string(),
                serde_json::json!([version, format!("linkhash:{}", name), {}, ""]),
            );
        }
        serde_json::json!({
            "lockfileVersion": 1,
            "packages": packages,
        })
        .to_string()
    }

    #[test]
    fn read_php_lock_versions_parses_php_packages() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(
            tmp.path().join("deka.lock"),
            lock_with(&[("@deka/cache", "0.1.0"), ("@tana/store", "0.2.3")]),
        )
        .expect("write");

        let versions = read_php_lock_versions(tmp.path());
        assert_eq!(
            versions.get("@deka/cache").map(String::as_str),
            Some("0.1.0")
        );
        assert_eq!(
            versions.get("@tana/store").map(String::as_str),
            Some("0.2.3")
        );
    }

    #[test]
    fn read_php_lock_versions_parses_legacy_php_packages() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(
            tmp.path().join("deka.lock"),
            serde_json::json!({
                "lockfileVersion": 1,
                "node": { "packages": {} },
                "php": {
                    "packages": {
                        "@deka/cache": [
                            "0.1.0",
                            "linkhash:@deka/cache",
                            {},
                            ""
                        ]
                    }
                }
            })
            .to_string(),
        )
        .expect("write");

        let versions = read_php_lock_versions(tmp.path());
        assert_eq!(
            versions.get("@deka/cache").map(String::as_str),
            Some("0.1.0")
        );
    }

    #[test]
    fn read_php_lock_versions_tolerates_missing_file() {
        let tmp = tempfile::tempdir().expect("tmp");
        let versions = read_php_lock_versions(tmp.path());
        assert!(versions.is_empty());
    }

    #[test]
    fn diff_versions_reports_bumps_and_adds_only() {
        let mut before: BTreeMap<String, String> = BTreeMap::new();
        before.insert("@deka/cache".into(), "0.1.0".into());
        before.insert("@deka/core".into(), "0.1.0".into());

        let mut after: BTreeMap<String, String> = BTreeMap::new();
        after.insert("@deka/cache".into(), "0.2.0".into()); // bumped
        after.insert("@deka/core".into(), "0.1.0".into()); // unchanged
        after.insert("@tana/store".into(), "0.5.0".into()); // added

        let diff = diff_versions(&before, &after);
        assert_eq!(diff.len(), 2);
        let (name, old, new) = &diff[0];
        assert_eq!(name, "@deka/cache");
        assert_eq!(old.as_deref(), Some("0.1.0"));
        assert_eq!(new, "0.2.0");
        let (name, old, new) = &diff[1];
        assert_eq!(name, "@tana/store");
        assert!(old.is_none());
        assert_eq!(new, "0.5.0");
    }

    #[test]
    fn format_diff_line_is_parseable_for_broadcast() {
        let diff = vec![
            ("@deka/cache".into(), Some("1.2.0".into()), "1.3.0".into()),
            ("@tana/store".into(), Some("0.4.1".into()), "0.5.0".into()),
        ];
        let line = format_diff_line(&diff);
        assert_eq!(
            line,
            "@deka/cache 1.2.0 -> 1.3.0, @tana/store 0.4.1 -> 0.5.0"
        );
    }

    #[test]
    fn format_diff_line_marks_added_deps() {
        let diff = vec![("@deka/new".into(), None, "0.1.0".into())];
        let line = format_diff_line(&diff);
        assert_eq!(line, "@deka/new added@0.1.0");
    }

    #[test]
    fn rehash_phpx_packages_updates_lock_integrity_without_network() {
        let tmp = tempfile::tempdir().expect("tmp");
        let package_dir = tmp.path().join(MODULES_DIR).join("@deka").join("core");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        std::fs::write(
            package_dir.join("index.phpx"),
            "export function ok(): int { return 1 }",
        )
        .expect("package file");
        std::fs::write(
            tmp.path().join("deka.lock"),
            serde_json::json!({
                "lockfileVersion": 1,
                "packages": {
                    "@deka/core": [
                        "0.1.0",
                        "linkhash:@deka/core",
                        {
                            "moduleGraph": { "hash": "stale-module" },
                            "fsGraph": { "hash": "stale-fs" }
                        },
                        ""
                    ]
                }
            })
            .to_string(),
        )
        .expect("lock");

        let changed =
            rehash_phpx_packages(tmp.path(), &["@deka/core".to_string()]).expect("rehash");

        assert_eq!(changed, vec!["@deka/core".to_string()]);
        let raw = std::fs::read_to_string(tmp.path().join("deka.lock")).expect("read lock");
        let lock: serde_json::Value = serde_json::from_str(&raw).expect("json");
        let metadata = &lock["packages"]["@deka/core"][2];
        assert_ne!(metadata["moduleGraph"]["hash"], "stale-module");
        assert_ne!(metadata["fsGraph"]["hash"], "stale-fs");
    }

    #[test]
    fn collect_deka_json_deps_in_strips_semver_prefixes() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(
            tmp.path().join("deka.json"),
            r#"{"dependencies":{"@deka/cache":"^0.1.0","@tana/store":"~0.2.0","@deka/core":">=0.3.0"}}"#,
        )
        .expect("write");
        let mut specs = collect_deka_json_deps_in(tmp.path());
        specs.sort();
        assert_eq!(
            specs,
            vec![
                "@deka/core@0.3.0".to_string(),
                "@deka/cache@0.1.0".to_string(),
                "@tana/store@0.2.0".to_string(),
            ]
        );
    }

    #[test]
    fn detect_shop_working_tree_rejects_non_tenant_paths() {
        // cwd is the deka runtime workspace — not inside store/tenants/.
        // Must return None even if a deka.json is present nearby.
        let detected = detect_shop_working_tree();
        assert!(
            detected.is_none()
                || detected
                    .as_ref()
                    .map(|p| {
                        let s = p.to_string_lossy();
                        s.contains("/store/tenants/") || s.contains("\\store\\tenants\\")
                    })
                    .unwrap_or(true),
            "detect_shop_working_tree must only fire inside store/tenants/"
        );
    }
}
