//! Locate the dsc compiler binary (rfd#38).
//!
//! The compiler is a release artifact shipped beside `deka`. Test callers use
//! the repository-pinned release artifact. A compiler path is never selected
//! from the process environment.

use std::path::PathBuf;

pub fn find_dsc() -> Result<Option<PathBuf>, String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("dsc");
            if sibling.is_file() {
                return Ok(Some(sibling));
            }
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest.ancestors().nth(2) {
        let pinned = root.join("target/release/dsc");
        if pinned.is_file() {
            return Ok(Some(pinned));
        }
    }
    Ok(None)
}
