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

}
