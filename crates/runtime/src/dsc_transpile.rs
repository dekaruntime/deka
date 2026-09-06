//! Compile one `.ds` / `.dsx` file by exec'ing dsc (rfd#38).

use std::fs;
use std::process::Command;

pub fn compile_file(path: &str) -> Result<String, String> {
    let dsc = runtime_core::dsc::find_dsc()?.ok_or_else(|| {
        "dsc is required to compile DekaScript. Set DEKA_DSC, install dsc next to deka, or put dsc on PATH.".to_string()
    })?;
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
    fs::read_to_string(out).map_err(|err| format!("failed to read {}: {err}", out.display()))
}
