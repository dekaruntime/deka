//! Native formatter answers for the generated-WASM conformance consumer.
use std::{
    fs,
    path::{Path, PathBuf},
};
fn collect(root: &Path, sources: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|error| format!("{}: {error}", root.display()))? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            collect(&path, sources)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension, "ds" | "dsx"))
        {
            sources.push(path);
        }
    }
    Ok(())
}
fn run() -> Result<(), String> {
    let mut files = Vec::new();
    for root in std::env::args_os().skip(1) {
        collect(Path::new(&root), &mut files)?;
    }
    if files.is_empty() {
        return Err("expected directories containing actual formatter sources".into());
    }
    files.sort();
    for file in files {
        let source = fs::read_to_string(&file).map_err(|error| error.to_string())?;
        let expected = deka_fmt::format_ds(&source)?;
        println!(
            "{}",
            serde_json::json!({"file": file, "source": source, "expected": expected})
        );
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
