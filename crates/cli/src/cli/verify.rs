//! `deka verify` (deka#738 F3): verify the published `dist/` tree against the
//! build manifest's recorded artifact digests. This is the check to run
//! before a deploy — `dist/client` is what a static host serves, so a CDN
//! would serve tampered bytes without it. Thin wrapper over
//! [`runtime_core::framework::BuildManifest::verify_artifacts`]; exits
//! non-zero and names every mismatched path when anything disagrees.

use std::path::{Path, PathBuf};

use core::{CommandSpec, Context, Registry};

const COMMAND: CommandSpec = CommandSpec {
    name: "verify",
    category: "project",
    summary: "verify dist/ against the build manifest's artifact digests",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

pub fn cmd(context: &Context) {
    if let Err(err) = run(context) {
        stdio::error("verify", &err);
        std::process::exit(1);
    }
}

fn run(context: &Context) -> Result<(), String> {
    let root_hint = context
        .args
        .positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|err| err.to_string())?);
    let project_root = resolve_project_root(&root_hint)?;

    let manifest_path =
        runtime_core::framework::compiler_cache_dir(&project_root).join("build-manifest.json");
    if !manifest_path.is_file() {
        return Err(format!(
            "no build manifest at {}; run `deka build` first",
            manifest_path.display()
        ));
    }
    let dist = project_root.join("dist");
    if !dist.is_dir() {
        return Err(format!(
            "no dist/ directory at {}; run `deka build` first",
            dist.display()
        ));
    }

    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;
    let manifest: runtime_core::framework::BuildManifest = serde_json::from_str(&raw)
        .map_err(|err| format!("invalid {}: {err}", manifest_path.display()))?;

    let problems = manifest.verify_artifacts(&project_root);
    if !problems.is_empty() {
        for problem in &problems {
            stdio::error("verify", problem);
        }
        return Err(format!(
            "{} artifact{} failed verification",
            problems.len(),
            if problems.len() == 1 { "" } else { "s" }
        ));
    }

    stdio::success(&format!(
        "verified {} artifacts against {}",
        manifest.artifacts.len(),
        manifest_path.display()
    ));
    Ok(())
}

/// Same root search `deka build` uses: the hint itself when it is a project
/// root, else the nearest ancestor carrying a deka.json.
fn resolve_project_root(input_path: &Path) -> Result<PathBuf, String> {
    let start = if input_path.is_dir() {
        input_path.to_path_buf()
    } else {
        input_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf()
    };

    for dir in start.ancestors() {
        if dir.join("deka.json").is_file() {
            return Ok(dir.to_path_buf());
        }
    }

    Err(format!(
        "deka verify requires a deka.json project root (searched from {})",
        input_path.display()
    ))
}
