//! Shared native source snapshots and all-or-nothing edit classification.
use crate::Source;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
pub type Files = BTreeMap<PathBuf, String>;
pub fn snapshot(roots: &[PathBuf]) -> Result<Files, String> {
    fn visit(path: &Path, files: &mut Files) -> Result<(), String> {
        if !path.exists() {
            return Ok(());
        }
        if path.is_dir() {
            for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if matches!(
                    entry.file_name().to_str(),
                    Some(
                        "target" | ".target" | ".tmp" | ".git" | "node_modules" | "dist" | ".cache"
                    )
                ) {
                    continue;
                }
                if !entry.file_type().map_err(|e| e.to_string())?.is_symlink() {
                    visit(&entry.path(), files)?;
                }
            }
        } else if path.extension().is_some_and(|ext| ext == "rs")
            || path
                .file_name()
                .is_some_and(|name| name == "Cargo.toml" || name == "Cargo.lock")
        {
            match std::fs::read_to_string(path) {
                Ok(source) => {
                    files.insert(path.to_path_buf(), source);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    for root in roots {
        visit(root, &mut files)?;
    }
    Ok(files)
}
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Markup,
    Invalid(String),
    Restart(String),
}
pub fn classify(old: &Files, next: &Files) -> Change {
    if old.keys().ne(next.keys()) {
        return Change::Restart("source files added or removed".into());
    }
    let mut restart = None;
    for (path, source) in next {
        if old[path] == *source {
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "rs") {
            restart = Some(format!("{} changed", path.display()));
            continue;
        }
        let after = match Source::parse(source) {
            Ok(after) => after,
            Err(error) => return Change::Invalid(format!("{}: {error}", path.display())),
        };
        let before = match Source::parse(&old[path]) {
            Ok(before) => before,
            Err(_) => {
                restart = Some(format!(
                    "{} corrected after an invalid edit",
                    path.display()
                ));
                continue;
            }
        };
        if let Err(error) = before.compatible(&after) {
            restart = Some(format!("{}: {error}", path.display()));
        }
    }
    restart.map_or(Change::Markup, Change::Restart)
}

pub fn source_roots(arguments: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut metadata = Command::new("cargo");
    metadata.args(["metadata", "--format-version", "1", "--all-features"]);
    for flag in ["--locked", "--offline", "--frozen"] {
        if arguments.iter().any(|arg| arg == flag) {
            metadata.arg(flag);
        }
    }
    if let Some(index) = arguments.iter().position(|arg| arg == "--manifest-path") {
        metadata.args(
            arguments
                .get(index..index + 2)
                .ok_or("missing manifest path")?,
        );
    }
    for argument in arguments
        .iter()
        .filter(|arg| arg.starts_with("--manifest-path="))
    {
        metadata.arg(argument);
    }
    let output = metadata.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into());
    }
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let mut roots = vec![PathBuf::from(
        metadata["workspace_root"]
            .as_str()
            .ok_or("Cargo workspace root missing")?,
    )];
    for package in metadata["packages"]
        .as_array()
        .ok_or("Cargo packages missing")?
    {
        if !package["source"].is_null() {
            continue;
        }
        let manifest = Path::new(
            package["manifest_path"]
                .as_str()
                .ok_or("Cargo manifest missing")?,
        );
        roots.push(
            manifest
                .parent()
                .ok_or("Cargo manifest has no directory")?
                .to_path_buf(),
        );
    }
    roots.sort();
    roots.dedup();
    let mut independent: Vec<PathBuf> = vec![];
    for root in roots {
        if !independent.iter().any(|parent| root.starts_with(parent)) {
            independent.push(root);
        }
    }
    Ok(independent)
}
