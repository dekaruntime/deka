//! Shared recursive source discovery policy for `deka test` and `deka fmt`.
use std::{fs::DirEntry, io};

pub(crate) fn discoverable(entry: &DirEntry) -> io::Result<bool> {
    if matches!(
        entry.file_name().to_str(),
        Some("node_modules" | "ds_modules" | ".git" | ".target" | "target" | "dist")
    ) {
        return Ok(false);
    }
    // DirEntry::file_type does not follow the link, unlike Path::is_dir.
    Ok(!entry.file_type()?.is_symlink())
}
