//! Compile one `.ds` / `.dsx` file by exec'ing dsc (rfd#38).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn compile_file(path: &str) -> Result<String, String> {
    let dsc = find_dsc()?.ok_or_else(|| {
        "dsc is required to compile DekaScript. Set DEKA_DSC, install dsc next to deka, or put dsc on PATH.".to_string()
    })?;
    compile_file_with_dsc(path, &dsc)
}

/// Locate dsc for the legacy runtime-source helpers. CLI-dispatched source
/// work passes the selected compiler explicitly; this fallback preserves the
/// documented `DEKA_DSC` override for direct runtime callers. Artifact
/// posture does not call these helpers.
pub fn find_dsc() -> Result<Option<PathBuf>, String> {
    if let Ok(path) = env::var("DEKA_DSC") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(Some(path));
        }
        return Err(format!(
            "DEKA_DSC is set to {} but that path is not a file",
            path.display()
        ));
    }
    runtime_core::dsc::find_dsc()
}

/// Compile with a compiler selected by the caller. Build selects dsc at its
/// CLI boundary; runtime never re-reads process configuration for it.
pub fn compile_file_with_dsc(path: &str, dsc: &Path) -> Result<String, String> {
    // dsc refuses to overwrite a file it did not generate.
    let tmp = tempfile::tempdir().map_err(|err| format!("failed to create dsc temp dir: {err}"))?;
    let out = tmp.path().join("out.js");
    let out_str = out
        .to_str()
        .ok_or_else(|| "dsc temp path is not UTF-8".to_string())?;
    let output = Command::new(&dsc)
        .args(["transpile", path, "--out", out_str])
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    fs::read_to_string(&out).map_err(|err| format!("failed to read {}: {err}", out.display()))
}
