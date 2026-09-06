//! Compile/check paths for `deka build` via dsc (rfd#38).
//!
//! Lookup is `DEKA_DSC`, then `dsc` next to this `deka` binary, then `PATH`
//! (`crate::dsc` / `runtime_core::dsc`). Missing dsc is a hard error.
//!
//! Project builds prefer default emit (`dsc --outdir <dir>` from the project
//! root). Published dsc that still lacks that entrypoint falls back to
//! per-tree `dsc transpile <dir> --out <out>`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MISSING_DSC: &str = "dsc is required for deka build. Install dsc (https://deka.gg/install), set DEKA_DSC, or put dsc next to deka / on PATH.";

fn dsc_bin() -> Result<PathBuf, String> {
    crate::dsc::find_dsc()?.ok_or_else(|| MISSING_DSC.to_string())
}

pub fn require_dsc() -> Result<PathBuf, String> {
    dsc_bin()
}

/// Result of trying default project emit (`dsc --outdir`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectEmit {
    /// Default emit wrote `outdir/{app,api,src}` for trees that exist.
    Default,
    /// CLI does not understand default emit; caller should transpile per tree.
    NeedsTranspileFallback,
}

pub fn check_path(path: &Path) -> Result<(), String> {
    let dsc = dsc_bin()?;
    let output = Command::new(&dsc)
        .arg("check")
        .arg(path)
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if output.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

pub fn transpile_file(path: &Path) -> Result<String, String> {
    run_transpile(path, None, &["transpile"])
}

pub fn transpile_bundle(project_root: &Path, entry: &Path) -> Result<String, String> {
    run_transpile(entry, Some(project_root), &["transpile", "--bundle"])
}

/// Prefer tsc-like default emit: `dsc --outdir <outdir>` from the project root
/// with `DEKA_MODULE_ROOT` set. Falls back narrowly when the installed dsc
/// does not understand that entrypoint (help / unknown arg), so real compile
/// errors still fail the build.
pub fn emit_project(project_root: &Path, outdir: &Path) -> Result<ProjectEmit, String> {
    let dsc = dsc_bin()?;
    let out_str = path_utf8(outdir)?;
    let output = Command::new(&dsc)
        .current_dir(project_root)
        .env("DEKA_MODULE_ROOT", project_root)
        .args(["--outdir", out_str])
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    if looks_like_unsupported_default_emit(&combined) {
        return Ok(ProjectEmit::NeedsTranspileFallback);
    }

    if !output.status.success() {
        return Err(emit_failure_message(&dsc, &combined));
    }

    // Some older dsc builds print help and exit 0 for unknown top-level flags
    // without the "unknown argument" spelling. Empty staging while app/ exists
    // means default emit did not run.
    if project_root.join("app").is_dir() && !outdir.join("app").is_dir() {
        return Ok(ProjectEmit::NeedsTranspileFallback);
    }

    Ok(ProjectEmit::Default)
}

/// Preserve-mode directory emit: `dsc transpile <dir> --out <out>`.
pub fn transpile_dir(
    input_dir: &Path,
    out_dir: &Path,
    project_root: Option<&Path>,
) -> Result<(), String> {
    let dsc = dsc_bin()?;
    let input_str = path_utf8(input_dir)?;
    let out_str = path_utf8(out_dir)?;
    let mut cmd = Command::new(&dsc);
    if let Some(root) = project_root {
        cmd.current_dir(root).env("DEKA_MODULE_ROOT", root);
    }
    let output = cmd
        .args(["transpile", input_str, "--out", out_str])
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Err(stderr);
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Err(stdout);
    }
    Err(format!("{} transpile failed", dsc.display()))
}

fn looks_like_unsupported_default_emit(combined: &str) -> bool {
    let lower = combined.to_ascii_lowercase();
    lower.contains("unknown argument")
        || lower.contains("unknown flag")
        || lower.contains("unexpected argument")
        || lower.contains("unexpected flag")
        || lower.contains("instructions unclear")
        || lower.contains("usage: dsc")
        || lower.contains("try '--help'")
        || lower.contains("try \"--help\"")
}

fn emit_failure_message(dsc: &Path, combined: &str) -> String {
    let trimmed = combined.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    format!("{} emit failed", dsc.display())
}

fn path_utf8(path: &Path) -> Result<&str, String> {
    path.to_str().ok_or_else(|| "path is not UTF-8".to_string())
}

fn run_transpile(entry: &Path, cwd: Option<&Path>, prefix: &[&str]) -> Result<String, String> {
    let dsc = dsc_bin()?;
    // dsc refuses to overwrite a file it did not generate.
    let tmp =
        tempfile::tempdir().map_err(|err| format!("failed to create build temp dir: {err}"))?;
    let out = tmp.path().join("out.js");
    let entry_str = entry
        .to_str()
        .ok_or_else(|| "path is not UTF-8".to_string())?;
    let out_str = out
        .to_str()
        .ok_or_else(|| "temp path is not UTF-8".to_string())?;
    let mut cmd = Command::new(&dsc);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd).env("DEKA_MODULE_ROOT", cwd);
    }
    let output = cmd
        .args(prefix)
        .arg(entry_str)
        .args(["--out", out_str])
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    fs::read_to_string(&out).map_err(|err| format!("failed to read {}: {err}", out.display()))
}
