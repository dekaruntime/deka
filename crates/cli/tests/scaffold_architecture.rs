use std::fs;
use std::path::{Path, PathBuf};

fn runtime_root() -> PathBuf {
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        return Path::new(&manifest_dir)
            .parent()
            .expect("cli crate has crates parent")
            .parent()
            .expect("crates dir has runtime parent")
            .to_path_buf();
    }

    let cwd = std::env::current_dir().expect("current directory");
    if cwd.join("runtime/Cargo.toml").exists() {
        return cwd.join("runtime");
    }
    cwd
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("read_dir {}: {err}", dir.display()))
    {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn cli_binary_entrypoint_stays_thin() {
    let root = runtime_root();
    let main_rs = root.join("crates/cli/src/main.rs");
    let source = read(&main_rs);
    let lines = source.lines().count();

    assert!(
        lines <= 25,
        "crates/cli/src/main.rs should stay a thin entrypoint, got {} lines",
        lines
    );
    assert!(
        source.contains("cli::run();"),
        "crates/cli/src/main.rs should delegate to cli::run()"
    );
}

#[test]
fn tana_cli_core_is_a_real_workspace_crate() {
    let root = runtime_root();
    let workspace = read(&root.join("Cargo.toml"));
    let core_manifest = root.join("crates/tana-cli-core/Cargo.toml");
    let core_lib = root.join("crates/tana-cli-core/src/lib.rs");

    assert!(
        workspace.contains("\"crates/tana-cli-core\""),
        "runtime/Cargo.toml must include crates/tana-cli-core as a workspace member"
    );
    assert!(
        core_manifest.exists(),
        "tana-cli-core must have its own Cargo.toml"
    );
    assert!(core_lib.exists(), "tana-cli-core must expose src/lib.rs");
}

#[test]
fn tana_cli_core_scaffold_is_modular_and_implemented() {
    let root = runtime_root();
    let core_src = root.join("crates/tana-cli-core/src");
    let mut files = Vec::new();
    collect_files(&core_src, &mut files);

    let rust_files: Vec<_> = files
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    assert!(
        rust_files.len() >= 3,
        "tana-cli-core should be split into focused modules, got {} Rust files",
        rust_files.len()
    );

    for path in rust_files {
        let source = read(path);
        let lines = source.lines().count();
        assert!(
            lines <= 1000,
            "{} has {lines} lines; scaffold modules must stay under 1000 lines",
            path.strip_prefix(&root).unwrap_or(path).display()
        );

        let lowered = source.to_ascii_lowercase();
        assert!(
            !lowered.contains("todo!")
                && !lowered.contains("unimplemented!")
                && !lowered.contains("stub"),
            "{} contains placeholder implementation text",
            path.strip_prefix(&root).unwrap_or(path).display()
        );
    }
}

#[test]
fn tana_cli_core_does_not_ship_repo_docs() {
    let root = runtime_root();
    let core_dir = root.join("crates/tana-cli-core");
    let mut files = Vec::new();
    collect_files(&core_dir, &mut files);

    let markdown: Vec<_> = files
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();

    assert!(
        markdown.is_empty(),
        "tana-cli-core should not add repo-local Markdown docs; suggest docs for Brain instead: {:?}",
        markdown
    );
}
