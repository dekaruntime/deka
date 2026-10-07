//! Compile one `.ds` / `.dsx` file by exec'ing dsc (rfd#38).

use std::env;
use std::path::PathBuf;

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
    compiler::dsc::find_dsc()
}

