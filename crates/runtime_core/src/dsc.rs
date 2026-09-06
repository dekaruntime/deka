//! Locate the dsc compiler binary (rfd#38).
//!
//! Lookup order:
//! 1. `DEKA_DSC` — explicit path. If set and missing, fail closed.
//! 2. `dsc` next to the current executable.
//! 3. `dsc` on `PATH`.
//!
//! `DEKA_NO_DSC=1` forces the in-process compiler fallback.

use std::env;
use std::path::PathBuf;

pub fn find_dsc() -> Result<Option<PathBuf>, String> {
    if env::var_os("DEKA_NO_DSC").is_some() {
        return Ok(None);
    }
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
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("dsc");
            if sibling.is_file() {
                return Ok(Some(sibling));
            }
        }
    }
    if let Ok(path_var) = env::var("PATH") {
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join("dsc");
            if candidate.is_file() {
                return Ok(Some(candidate));
            }
        }
    }
    Ok(None)
}
