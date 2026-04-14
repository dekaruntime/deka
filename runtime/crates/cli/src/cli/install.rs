use anyhow::Result;
use core::{CommandSpec, Context, FlagSpec, ParamSpec, Registry};
use linkhash_client::{LinkhashClient, is_phpx_package};
use pm::{InstallPayload, run_install};
use runtime_core::module_spec::canonical_php_package_spec;
use std::path::PathBuf;
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
    summary: "install php package(s) (shorthand)",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

const I_COMMAND: CommandSpec = CommandSpec {
    name: "i",
    category: "package",
    summary: "install php package(s) (short alias)",
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
    registry.add_param(ParamSpec {
        name: "--payload",
        description: "path to a JSON payload describing the install",
    });
    registry.add_param(ParamSpec {
        name: "--ecosystem",
        description: "ecosystem hint (php)",
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
        description: "registry base URL (fallback: LINKHASH_REGISTRY, TANA_GIT_SERVER, or http://localhost:9418)",
    });
    registry.add_param(ParamSpec {
        name: "--token",
        description: "auth token (fallback: LINKHASH_TOKEN, TANA_GIT_TOKEN)",
    });
}

pub fn cmd(context: &Context) {
    // Set registry env vars from flags/env before delegating to pm
    apply_registry_env(context);

    // Check if any positional args are scoped PHPX packages
    let specs: Vec<String> = context.args.positionals.clone();
    let phpx_specs: Vec<&String> = specs.iter().filter(|s| is_phpx_package(s)).collect();

    if !phpx_specs.is_empty() {
        let (registry_url, token) = get_registry_config(context);
        let client = LinkhashClient::new(&registry_url, token.as_deref());
        let project_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        for spec in &phpx_specs {
            let (name, version_range) = parse_spec_with_version(spec);
            match install_phpx_package(&client, &name, &version_range, &project_dir) {
                Ok(version) => {
                    stdio::log("install", &format!("installed {}@{}", name, version));
                }
                Err(err) => {
                    stdio::error("install", &format!("failed to install {}: {}", name, err));
                }
            }
        }

        // If there are also non-phpx specs, fall through to pm
        let non_phpx: Vec<String> = specs.iter()
            .filter(|s| !is_phpx_package(s))
            .cloned()
            .collect();
        if non_phpx.is_empty() {
            return;
        }
    }

    match build_payload(context) {
        Ok(payload) => {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            if let Err(err) = runtime.block_on(run_install(payload)) {
                let message = err.to_string();
                stdio::error("install", &message);
            }
        }
        Err(err) => {
            let message = err.to_string();
            stdio::error("install", &message);
        }
    }
}

pub fn cmd_update(context: &Context) {
    // Set registry env vars from flags/env before delegating to pm
    apply_registry_env(context);

    // For update, check if positionals or deka.json deps contain scoped packages
    let mut specs = context.args.positionals.clone();
    if specs.is_empty() {
        specs = collect_deka_json_deps();
    }

    let phpx_specs: Vec<String> = specs.iter()
        .filter(|s| is_phpx_package(s))
        .cloned()
        .collect();

    if !phpx_specs.is_empty() {
        let (registry_url, token) = get_registry_config(context);
        let client = LinkhashClient::new(&registry_url, token.as_deref());
        let project_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        for spec in &phpx_specs {
            let (name, version_range) = parse_spec_with_version(spec);
            match install_phpx_package(&client, &name, &version_range, &project_dir) {
                Ok(version) => {
                    stdio::log("update", &format!("updated {}@{}", name, version));
                }
                Err(err) => {
                    stdio::error("update", &format!("failed to update {}: {}", name, err));
                }
            }
        }
    }

    // Still run pm for non-phpx packages
    match build_update_payload(context) {
        Ok(payload) => {
            // Filter out phpx specs from the payload
            let runtime = tokio::runtime::Runtime::new().unwrap();
            if let Err(err) = runtime.block_on(run_install(payload)) {
                let message = err.to_string();
                stdio::error("update", &message);
            }
        }
        Err(err) => {
            let message = err.to_string();
            stdio::error("update", &message);
        }
    }
}

/// Set LINKHASH_REGISTRY_URL from --registry flag or TANA_GIT_SERVER env,
/// and LINKHASH_TOKEN from --token flag or TANA_GIT_TOKEN env.
/// This ensures the pm crate picks up the right values.
fn apply_registry_env(context: &Context) {
    if let Some(registry) = context.args.params.get("--registry") {
        unsafe { std::env::set_var("LINKHASH_REGISTRY_URL", registry); }
    } else if std::env::var("LINKHASH_REGISTRY_URL").is_err() {
        if let Ok(val) = std::env::var("LINKHASH_REGISTRY") {
            unsafe { std::env::set_var("LINKHASH_REGISTRY_URL", val); }
        } else if let Ok(val) = std::env::var("TANA_GIT_SERVER") {
            unsafe { std::env::set_var("LINKHASH_REGISTRY_URL", val); }
        } else {
            unsafe { std::env::set_var("LINKHASH_REGISTRY_URL", "http://localhost:9418"); }
        }
    }

    if let Some(token) = context.args.params.get("--token") {
        unsafe { std::env::set_var("LINKHASH_TOKEN", token); }
    } else if std::env::var("LINKHASH_TOKEN").is_err() {
        if let Ok(val) = std::env::var("TANA_GIT_TOKEN") {
            unsafe { std::env::set_var("LINKHASH_TOKEN", val); }
        }
    }
}

fn build_payload(context: &Context) -> Result<InstallPayload> {
    let command_name = context
        .args
        .commands
        .first()
        .map(String::as_str)
        .unwrap_or("install");

    let mut payload = if let Some(payload_path) = context.args.params.get("--payload") {
        let mut payload = InstallPayload::from_file(&PathBuf::from(payload_path))?;
        if let Some(path) = context.args.params.get("--ecosystem") {
            payload.ecosystem = Some(path.clone());
        }
        payload
    } else {
        let mut specs = context
            .args
            .params
            .get("--spec")
            .map(|value| parse_spec_list(value))
            .unwrap_or_default();
        if specs.is_empty() && !context.args.positionals.is_empty() {
            specs = context.args.positionals.clone();
        }
        let ecosystem = context.args.params.get("--ecosystem").cloned().or_else(|| {
            if command_name == "install" {
                None
            } else {
                Some("php".to_string())
            }
        });
        InstallPayload {
            specs,
            ecosystem,
            yes: false,
            prompt: false,
            quiet: false,
            rehash: false,
        }
    };

    if payload.ecosystem.as_deref() == Some("php") {
        let mut resolved_specs = Vec::new();
        for spec in &payload.specs {
            resolved_specs.push(resolve_php_spec(spec)?);
        }
        payload.specs = resolved_specs;
    }

    apply_flags(&mut payload, context);
    Ok(payload)
}

fn build_update_payload(context: &Context) -> Result<InstallPayload> {
    // For update: read specs from positionals or deka.json dependencies.
    // The pm crate handles resolution; we pass specs with the ecosystem hint.
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

    let ecosystem = context.args.params.get("--ecosystem").cloned()
        .unwrap_or_else(|| "php".to_string());

    if ecosystem == "php" {
        let mut resolved = Vec::new();
        for spec in &specs {
            resolved.push(resolve_php_spec(spec)?);
        }
        specs = resolved;
    }

    let mut payload = InstallPayload {
        specs,
        ecosystem: Some(ecosystem),
        yes: false,
        prompt: false,
        quiet: false,
        rehash: false,
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
                let clean = v.trim_start_matches('^').trim_start_matches('~').trim_start_matches(">=");
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

/// Extract registry URL and token from CLI context / env.
fn get_registry_config(context: &Context) -> (String, Option<String>) {
    let registry = context.args.params.get("--registry")
        .cloned()
        .or_else(|| std::env::var("LINKHASH_REGISTRY_URL").ok())
        .or_else(|| std::env::var("LINKHASH_REGISTRY").ok())
        .or_else(|| std::env::var("TANA_GIT_SERVER").ok())
        .unwrap_or_else(|| "http://localhost:9418".to_string());

    let token = context.args.params.get("--token")
        .cloned()
        .or_else(|| std::env::var("LINKHASH_TOKEN").ok())
        .or_else(|| std::env::var("TANA_GIT_TOKEN").ok());

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

/// Install a scoped PHPX package using linkhash-client.
/// Returns the installed version on success.
fn install_phpx_package(
    client: &LinkhashClient,
    name: &str,
    version_range: &str,
    project_dir: &std::path::Path,
) -> Result<String> {
    // Resolve the version
    let resolved = client.resolve(name, version_range)?;

    // Determine target: php_modules/@scope/name
    let target = project_dir.join("php_modules").join(name);

    // Remove existing installation if present
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }

    // Download
    client.download(name, &resolved.version, &target)?;

    // Compute integrity hashes for the installed package so the lock matches
    // what the module validator recomputes at load time.
    let integrity = modules_php::integrity::compute_package_integrity(&target)
        .map_err(|err| anyhow::anyhow!("integrity hash failed for {}: {}", name, err))?;

    // Update deka.lock with the resolved version + integrity hashes
    update_deka_lock(project_dir, name, &resolved.version, &integrity.module_graph, &integrity.fs_graph)?;

    Ok(resolved.version)
}

/// Update deka.lock with the installed package version.
///
/// The lock format expected by the module validator is:
/// `{ "lockfileVersion": 1, "node": { "packages": {} }, "php": { "packages": {...} } }`.
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

    // Ensure the top-level shape exists, migrating older { "packages": {} } layouts.
    if !lock.get("lockfileVersion").is_some() {
        lock["lockfileVersion"] = serde_json::json!(1);
    }
    if !lock.get("node").and_then(|v| v.get("packages")).is_some() {
        lock["node"] = serde_json::json!({ "packages": {} });
    }
    // Migrate a bare top-level "packages" (old format) into php.packages.
    let legacy_packages = lock
        .get("packages")
        .and_then(|v| v.as_object())
        .cloned();
    if !lock.get("php").and_then(|v| v.get("packages")).is_some() {
        lock["php"] = serde_json::json!({ "packages": {} });
    }
    if let Some(legacy) = legacy_packages {
        if let Some(php_packages) = lock
            .get_mut("php")
            .and_then(|v| v.get_mut("packages"))
            .and_then(|v| v.as_object_mut())
        {
            for (k, v) in legacy {
                php_packages.entry(k).or_insert(v);
            }
        }
        lock.as_object_mut().map(|o| o.remove("packages"));
    }

    if let Some(php_packages) = lock
        .get_mut("php")
        .and_then(|v| v.get_mut("packages"))
        .and_then(|v| v.as_object_mut())
    {
        php_packages.insert(
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
    }

    std::fs::write(&lock_path, serde_json::to_string_pretty(&lock)?)?;
    Ok(())
}

fn default_lock() -> serde_json::Value {
    serde_json::json!({
        "lockfileVersion": 1,
        "node": { "packages": {} },
        "php": { "packages": {} },
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
