#![allow(clippy::all)]

use core::{CommandSpec, Context, ParamSpec, Registry, SubcommandSpec};
use std::path::{Path, PathBuf};

const COMMAND: CommandSpec = CommandSpec {
    name: "wasm",
    category: "extensions",
    summary: "manage wasm extensions",
    aliases: &[],
    subcommands: &[
        SubcommandSpec {
            name: "init",
            summary: "scaffold a wasm extension",
            aliases: &[],
            handler: cmd_init,
        },
        SubcommandSpec {
            name: "build",
            summary: "build a wasm extension",
            aliases: &[],
            handler: cmd_build,
        },
        SubcommandSpec {
            name: "stubs",
            summary: "generate wasm .d.phpx stubs",
            aliases: &[],
            handler: cmd_stubs,
        },
    ],
    handler: cmd_default,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_param(ParamSpec {
        name: "--root",
        description: "project root (defaults to handler directory)",
    });
}

fn cmd_default(_context: &Context) {
    stdio::raw("deka wasm");
    stdio::raw("");
    stdio::raw("Usage:");
    stdio::raw("  deka wasm init <@user/name>");
    stdio::raw("  deka wasm build <@user/name>");
    stdio::raw("  deka wasm stubs [@user/name]");
    stdio::raw("");
    stdio::raw("Options:");
    stdio::raw("  --root <path>   override project root (default: handler directory)");
}

fn cmd_init(context: &Context) {
    let Some(spec) = context.args.positionals.get(0) else {
        stdio::error("wasm", "missing module spec (e.g. @user/hello)");
        return;
    };

    let root = project_root(context);
    let module_spec = match normalize_module_spec(spec) {
        Ok(value) => value,
        Err(message) => {
            stdio::error("wasm", &message);
            return;
        }
    };

    let module_path = module_spec_path(&root, &module_spec);
    if module_path.exists() {
        stdio::error("wasm", "module already exists");
        return;
    }

    if let Err(err) = std::fs::create_dir_all(&module_path) {
        stdio::error("wasm", &format!("failed to create module dir: {err}"));
        return;
    }

    let module_name = module_spec
        .name
        .clone()
        .unwrap_or_else(|| "module".to_string());
    let world_name = sanitize_wit_ident(&module_name);
    let crate_name = sanitize_crate_name(&module_name);
    let package_namespace = sanitize_wit_ident(&module_spec.namespace);
    let package_name = format!("{}:{}", package_namespace, world_name);

    if let Err(err) = write_deka_manifest(&module_path, &crate_name, &world_name) {
        stdio::error("wasm", &format!("failed to write deka.json: {err}"));
        return;
    }

    if let Err(err) = write_wit_file(&module_path, &package_name, &world_name) {
        stdio::error("wasm", &format!("failed to write module.wit: {err}"));
        return;
    }

    let rust_dir = module_path.join("rust");
    if let Err(err) = std::fs::create_dir_all(rust_dir.join("src")) {
        stdio::error("wasm", &format!("failed to create rust crate: {err}"));
        return;
    }

    if let Err(err) = write_rust_crate(&rust_dir, &crate_name, &module_spec.namespace, &world_name)
    {
        stdio::error("wasm", &format!("failed to write rust crate: {err}"));
        return;
    }

    if let Err(err) = write_readme(&module_path, &module_spec.raw) {
        stdio::error("wasm", &format!("failed to write README: {err}"));
        return;
    }

    stdio::log("wasm", "scaffolded module");
    stdio::log("wasm", &format!("path: {}", module_path.display()));
}

fn cmd_build(context: &Context) {
    let Some(spec) = context.args.positionals.get(0) else {
        stdio::error("wasm", "missing module spec (e.g. @user/hello)");
        return;
    };

    let root = project_root(context);
    let module_spec = match normalize_module_spec(spec) {
        Ok(value) => value,
        Err(message) => {
            stdio::error("wasm", &message);
            return;
        }
    };

    let module_path = module_spec_path(&root, &module_spec);
    let manifest_path = module_path.join("deka.json");
    if !manifest_path.exists() {
        stdio::error("wasm", "missing deka.json in module directory");
        return;
    }

    let manifest = match read_manifest(&manifest_path) {
        Ok(value) => value,
        Err(message) => {
            stdio::error("wasm", &message);
            return;
        }
    };

    let crate_dir = manifest
        .crate_dir
        .as_ref()
        .map(|dir| module_path.join(dir))
        .unwrap_or_else(|| module_path.join("rust"));

    let crate_manifest = crate_dir.join("Cargo.toml");
    if !crate_manifest.exists() {
        stdio::error("wasm", "missing Cargo.toml for wasm crate");
        return;
    }

    let crate_name = manifest
        .crate_name
        .clone()
        .or_else(|| read_crate_name(&crate_manifest))
        .unwrap_or_else(|| "module".to_string());

    let status = std::process::Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("--manifest-path")
        .arg(&crate_manifest)
        .status();

    match status {
        Ok(status) if status.success() => {}
        Ok(status) => {
            stdio::error("wasm", &format!("cargo build failed (status {status})"));
            return;
        }
        Err(err) => {
            stdio::error("wasm", &format!("failed to run cargo build: {err}"));
            return;
        }
    }

    let target_dir = crate_dir
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release");

    let wasm_name = if crate_name.contains('-') {
        crate_name.replace('-', "_")
    } else {
        crate_name.clone()
    };
    let wasm_path = target_dir.join(format!("{wasm_name}.wasm"));
    if !wasm_path.exists() {
        stdio::error(
            "wasm",
            &format!("missing wasm output at {}", wasm_path.display()),
        );
        return;
    }

    let output_path = module_path.join(manifest.module_path.as_deref().unwrap_or("module.wasm"));
    if let Err(err) = std::fs::copy(&wasm_path, &output_path) {
        stdio::error("wasm", &format!("failed to copy wasm: {err}"));
        return;
    }

    if let Err(message) = run_stub_generation(&root, Some(&module_path)) {
        stdio::warn("wasm", &message);
    }

    stdio::log("wasm", "build complete");
    stdio::log("wasm", &format!("module: {}", output_path.display()));
}

fn cmd_stubs(context: &Context) {
    let root = project_root(context);
    let module_dir = context.args.positionals.get(0).map(|spec| {
        normalize_module_spec(spec)
            .ok()
            .map(|module_spec| module_spec_path(&root, &module_spec))
    });

    if let Some(Some(path)) = module_dir {
        if let Err(message) = run_stub_generation(&root, Some(&path)) {
            stdio::error("wasm", &message);
        }
        return;
    }

    if let Err(message) = run_stub_generation(&root, None) {
        stdio::error("wasm", &message);
    }
}

fn project_root(context: &Context) -> PathBuf {
    if let Some(root) = context.args.params.get("--root") {
        return PathBuf::from(root);
    }
    if let Some(root) = context.args.params.get("--folder") {
        return PathBuf::from(root);
    }
    context.handler.resolved.directory.clone()
}

struct ModuleSpec {
    raw: String,
    namespace: String,
    name: Option<String>,
    segments: Vec<String>,
}

fn normalize_module_spec(spec: &str) -> Result<ModuleSpec, String> {
    let raw = spec.trim();
    if raw.is_empty() {
        return Err("module spec cannot be empty".to_string());
    }
    if raw.contains('\\') || raw.contains("..") {
        return Err("module spec contains invalid path segments".to_string());
    }

    let mut namespace = "user".to_string();
    let name: String;
    let mut path = raw.to_string();
    let had_at = raw.starts_with('@');

    if raw.starts_with('@') {
        path = raw.trim_start_matches('@').to_string();
    }

    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return Err("module spec must include a name".to_string());
    }

    if parts.len() >= 3 {
        return Err("module spec must be @namespace/name".to_string());
    }

    if parts.len() == 2 {
        namespace = parts[0].to_string();
        name = parts[1].to_string();
    } else {
        if had_at {
            return Err("module spec must be @namespace/name".to_string());
        }
        name = parts[0].to_string();
    }

    let mut segments = Vec::new();
    segments.push(format!("@{}", namespace));
    segments.push(name.clone());

    Ok(ModuleSpec {
        raw: format!("@{}/{}", namespace, name),
        namespace,
        name: Some(name),
        segments,
    })
}

fn module_spec_path(root: &Path, spec: &ModuleSpec) -> PathBuf {
    let mut out = root.join("php_modules");
    for segment in &spec.segments {
        out = out.join(segment);
    }
    out
}

fn sanitize_crate_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        let mapped = match ch {
            'a'..='z' | '0'..='9' | '_' => ch,
            'A'..='Z' => ch.to_ascii_lowercase(),
            '-' | '.' | ' ' => '_',
            _ => '_',
        };
        out.push(mapped);
    }
    if out.is_empty() {
        "module".to_string()
    } else {
        out
    }
}

fn sanitize_wit_ident(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        let mapped = match ch {
            'a'..='z' | '0'..='9' => ch,
            'A'..='Z' => ch.to_ascii_lowercase(),
            '-' | '_' | '.' | ' ' => '-',
            _ => '-',
        };
        if mapped == '-' {
            if last_dash || out.is_empty() {
                continue;
            }
            last_dash = true;
            out.push(mapped);
        } else {
            last_dash = false;
            out.push(mapped);
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "module".to_string()
    } else {
        out
    }
}

fn sanitize_rust_ident(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        let mapped = match ch {
            'a'..='z' | '0'..='9' | '_' => ch,
            'A'..='Z' => ch.to_ascii_lowercase(),
            '-' | '.' | ' ' => '_',
            _ => '_',
        };
        out.push(mapped);
    }
    if out.is_empty() {
        "module".to_string()
    } else {
        out
    }
}

fn write_deka_manifest(dir: &Path, crate_name: &str, world: &str) -> Result<(), std::io::Error> {
    let manifest = serde_json::json!({
        "module": "module.wasm",
        "wit": "module.wit",
        "abi": "wit",
        "records": "struct",
        "stubs": "module.d.phpx",
        "world": world,
        "interfacePrefix": false,
        "crate": "rust",
        "crateName": crate_name,
    });

    let contents = serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(dir.join("deka.json"), contents)
}

fn write_wit_file(dir: &Path, package_name: &str, world: &str) -> Result<(), std::io::Error> {
    let contents = format!(
        "package {package_name}@0.1.0;\n\ninterface api {{\n  greet: func(name: string) -> string;\n}}\n\nworld {world} {{\n  export api;\n}}\n"
    );
    std::fs::write(dir.join("module.wit"), contents)
}

fn write_rust_crate(
    dir: &Path,
    crate_name: &str,
    namespace: &str,
    world: &str,
) -> Result<(), std::io::Error> {
    let namespace_mod = sanitize_rust_ident(namespace);
    let world_mod = sanitize_rust_ident(world);
    let cargo = format!(
        "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\nwit-bindgen = \"0.46\"\n\n"
    );

    let lib_rs = format!(
        "use wit_bindgen::generate;\n\n\ngenerate!({{\n    path: \"../module.wit\",\n    world: \"{world}\",\n}});\n\nstruct Component;\n\nimpl exports::{namespace_mod}::{world_mod}::api::Guest for Component {{\n    fn greet(name: String) -> String {{\n        format!(\"Hello, {{}}!\", name)\n    }}\n}}\n\nexport!(Component);\n"
    );

    std::fs::write(dir.join("Cargo.toml"), cargo)?;
    std::fs::write(dir.join("src").join("lib.rs"), lib_rs)?;
    Ok(())
}

fn write_readme(dir: &Path, module_spec: &str) -> Result<(), std::io::Error> {
    let contents = format!(
        "# Wasm extension\n\nModule: `{module_spec}`\n\n## Build\n```sh\ndeka wasm build {module_spec}\n```\n\n## Regenerate stubs\n```sh\ndeka wasm stubs {module_spec}\n```\n"
    );
    std::fs::write(dir.join("README.md"), contents)
}

struct ManifestConfig {
    module_path: Option<String>,
    crate_dir: Option<String>,
    crate_name: Option<String>,
}

fn read_manifest(path: &Path) -> Result<ManifestConfig, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|err| format!("invalid json in {}: {err}", path.display()))?;

    Ok(ManifestConfig {
        module_path: json
            .get("module")
            .and_then(|val| val.as_str())
            .map(|val| val.to_string()),
        crate_dir: json
            .get("crate")
            .and_then(|val| val.as_str())
            .map(|val| val.to_string()),
        crate_name: json
            .get("crateName")
            .and_then(|val| val.as_str())
            .map(|val| val.to_string()),
    })
}

fn read_crate_name(manifest: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(manifest).ok()?;
    let mut in_package = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package && trimmed.starts_with("name") {
            let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
            if parts.len() != 2 {
                continue;
            }
            let value = parts[1].trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn run_stub_generation(root: &Path, module_dir: Option<&Path>) -> Result<(), String> {
    let script_path = root.join("scripts").join("gen-wit-stubs.js");
    if !script_path.exists() {
        return Err("missing scripts/gen-wit-stubs.js".to_string());
    }

    let mut command = std::process::Command::new("node");
    command.arg(script_path);
    if let Some(module_dir) = module_dir {
        command.arg("--root");
        command.arg(module_dir);
    } else {
        command.arg("--root");
        command.arg(root.join("php_modules"));
    }

    let status = command.status().map_err(|err| err.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("stub generation failed (status {status})"))
    }
}
