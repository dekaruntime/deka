//! Compile/check paths for `deka build` via dsc (rfd#38).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn dsc_bin() -> Result<PathBuf, String> {
    crate::dsc::find_dsc()?.ok_or_else(|| {
        "dsc is required for deka build. Set DEKA_DSC, install dsc next to deka, or put dsc on PATH.".to_string()
    })
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

fn run_transpile(entry: &Path, cwd: Option<&Path>, prefix: &[&str]) -> Result<String, String> {
    let dsc = dsc_bin()?;
    // dsc refuses to overwrite a file it did not generate.
    let tmp = tempfile::tempdir().map_err(|err| format!("failed to create build temp dir: {err}"))?;
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
    fs::read_to_string(out).map_err(|err| format!("failed to read {}: {err}", out.display()))
}
