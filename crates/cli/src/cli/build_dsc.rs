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

pub use runtime_core::framework::{BuildPlan, BuildPlanSlot, PlannedSource};

const MISSING_DSC: &str = "dsc is required for deka build. Install dsc (https://deka.gg/install), set DEKA_DSC, or put dsc next to deka / on PATH.";

/// A compiler-generated entry written beside the staged JS module it imports.
#[derive(Debug)]
pub struct StagedBuildEntry {
    pub slot: BuildPlanSlot,
    pub path: PathBuf,
}

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

/// Bundle stage: `dsc transpile <entry> --bundle [--treeshake] --out <tmp>`,
/// run from `project_root` so graph imports resolve against the project.
/// `minify` maps onto dsc's `--treeshake` (dsc owns minification semantics).
pub fn transpile_bundle(
    project_root: &Path,
    entry: &Path,
    minify: bool,
) -> Result<String, String> {
    if minify {
        run_transpile(entry, Some(project_root), &["transpile", "--bundle", "--treeshake"])
    } else {
        run_transpile(entry, Some(project_root), &["transpile", "--bundle"])
    }
}

/// Ask the released compiler for a build plan. A plan is data, never an
/// instruction to execute arbitrary code: the build manifest validates its
/// version and shape (single spelling, owned by `runtime_core`) before any
/// build entry executes.
pub fn build_plan(project_root: &Path, input: &Path) -> Result<BuildPlan, String> {
    let dsc = dsc_bin()?;
    let input = path_utf8(input)?;
    let output = Command::new(&dsc)
        .current_dir(project_root)
        .env("DEKA_MODULE_ROOT", project_root)
        .args(["plan", input])
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if !output.status.success() {
        return Err(emit_failure_message(
            &dsc,
            &format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("dsc emitted an invalid build plan: {err}"))
}

/// Collect one compiler plan per source file in the project source trees.
/// Dsc does the language analysis; this host only aggregates its versioned
/// artifacts. Plan validation (version, slot shape, duplicate ids) is owned
/// by the build manifest.
pub fn collect_build_plans(
    project_root: &Path,
    source_roots: &[&Path],
) -> Result<Vec<PlannedSource>, String> {
    let mut planned = Vec::new();
    for source_root in source_roots {
        for source in collect_deka_source_files(source_root)? {
            let plan = build_plan(project_root, &source)?;
            planned.push(PlannedSource {
                file: source.to_string_lossy().into_owned(),
                plan,
            });
        }
    }
    Ok(planned)
}

/// `dsc --version` output (first line) when the binary reports one; None
/// otherwise. Recorded as compiler provenance in the build manifest.
pub fn dsc_identity() -> Option<String> {
    let dsc = dsc_bin().ok()?;
    let output = Command::new(&dsc).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    // dsc prints its version line to stderr (observed 0.8.x); fall back to
    // stderr when stdout is empty rather than recording no provenance.
    let text = if output.stdout.is_empty() {
        output.stderr
    } else {
        output.stdout
    };
    String::from_utf8_lossy(&text)
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
}

/// Place a generated entry next to its staged peer module. Relative imports
/// in Dsc's entry then resolve against the already-compiled JS tree, never
/// against the original DekaScript source tree.
pub fn stage_build_entries(
    project_root: &Path,
    staging_root: &Path,
    slots: Vec<BuildPlanSlot>,
) -> Result<Vec<StagedBuildEntry>, String> {
    let mut entries = Vec::with_capacity(slots.len());
    for slot in slots {
        let source = PathBuf::from(&slot.file);
        let source = if source.is_absolute() {
            source
        } else {
            project_root.join(source)
        };
        let relative = source.strip_prefix(project_root).map_err(|_| {
            format!(
                "dsc build plan source {} is outside project {}",
                source.display(),
                project_root.display()
            )
        })?;
        let peer = staging_root.join(relative).with_extension("js");
        if !peer.is_file() {
            return Err(format!(
                "dsc build plan source {} has no staged peer module at {}",
                source.display(),
                peer.display()
            ));
        }
        let parent = peer
            .parent()
            .ok_or_else(|| format!("staged module has no parent: {}", peer.display()))?;
        let path = parent.join(format!(".__deka_build_{}.js", slot.id));
        fs::write(&path, &slot.entry)
            .map_err(|err| format!("failed to write build entry {}: {err}", path.display()))?;
        entries.push(StagedBuildEntry { slot, path });
    }
    Ok(entries)
}

/// Generated entries are staging-only implementation details. Once the host
/// has materialized their values they must not be promoted into `dist/`.
pub fn remove_staged_build_entries(entries: &[StagedBuildEntry]) -> Result<(), String> {
    for entry in entries {
        if entry.path.exists() {
            fs::remove_file(&entry.path).map_err(|err| {
                format!(
                    "failed to remove staged build entry {}: {err}",
                    entry.path.display()
                )
            })?;
        }
    }
    Ok(())
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
        if let Some(message) = named_source_failure(project_root) {
            return Err(message);
        }
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

/// Project-mode dsc diagnostics currently carry locations but not file names.
/// Preserve the original error for non-source failures, but identify a source
/// failure through dsc's per-file check so `deka build` remains actionable.
fn named_source_failure(project_root: &Path) -> Option<String> {
    for directory in ["app", "api", "src"] {
        let Ok(paths) = collect_deka_source_files(&project_root.join(directory)) else {
            continue;
        };
        for path in paths {
            if let Err(diagnostic) = check_path(&path) {
                let path = path.strip_prefix(project_root).unwrap_or(&path);
                return Some(format!("{}: {}", path.display(), diagnostic.trim()));
            }
        }
    }
    None
}

fn path_utf8(path: &Path) -> Result<&str, String> {
    path.to_str().ok_or_else(|| "path is not UTF-8".to_string())
}

fn is_deka_source_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ds") | Some("dsx")
    )
}

/// 1:1 tree emit: `dsc transpile <dir> --out <out>` for `.ds` / `.dsx`, then
/// copy every other file verbatim. No routing.
pub fn emit_source_tree(
    src_dir: &Path,
    dest_dir: &Path,
    project_root: &Path,
) -> Result<(), String> {
    if !src_dir.is_dir() {
        return Ok(());
    }
    let sources = collect_deka_source_files(src_dir)?;
    if !sources.is_empty() {
        if let Err(err) = transpile_dir(src_dir, dest_dir, Some(project_root)) {
            for path in &sources {
                if let Err(check_err) = check_path(path) {
                    let check_err = check_err.trim();
                    if check_err.is_empty() {
                        return Err(format!("{}: {err}", path.display()));
                    }
                    return Err(format!("{}: {check_err}", path.display()));
                }
            }
            return Err(err);
        }
    }
    copy_non_ds_tree(src_dir, dest_dir, false)
}

/// Copy non-`.ds`/`.dsx` files under `src` into `dst`. When `skip_existing` is
/// set, leave files dsc (or a prior step) already wrote alone.
pub fn copy_non_ds_tree(src: &Path, dst: &Path, skip_existing: bool) -> Result<(), String> {
    if !src.is_dir() {
        return Ok(());
    }
    let entries =
        fs::read_dir(src).map_err(|err| format!("failed to read {}: {}", src.display(), err))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read_dir entry error: {}", err))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|err| format!("file_type error for {}: {}", src_path.display(), err))?;
        if file_type.is_dir() {
            copy_non_ds_tree(&src_path, &dst_path, skip_existing)?;
        } else if file_type.is_file() {
            if is_deka_source_path(&src_path) {
                continue;
            }
            if skip_existing && dst_path.is_file() {
                continue;
            }
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create {}: {}", parent.display(), err))?;
            }
            fs::copy(&src_path, &dst_path).map_err(|err| {
                format!(
                    "failed to copy {} -> {}: {}",
                    src_path.display(),
                    dst_path.display(),
                    err
                )
            })?;
        }
    }
    Ok(())
}

fn collect_deka_source_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Ok(files);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current)
            .map_err(|err| format!("failed to read {}: {}", current.display(), err))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("read_dir entry error: {}", err))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|err| format!("file_type error for {}: {}", path.display(), err))?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() && is_deka_source_path(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
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
