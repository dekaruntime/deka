//! Release-artifact regression test for deka#712.
//!
//! The CLI must not need the introspection crate's source tree after it has
//! been built. This test copies the release artifact out of `target/`, hides
//! the source UI directory, and runs the copied binary from an unrelated
//! temporary working directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("repository root")
        .to_path_buf()
}

fn release_cli() -> PathBuf {
    let path = repo_root().join("target").join("release").join("cli");
    assert!(
        path.is_file(),
        "release-artifact test needs {}; run `cargo build --release -p cli` first",
        path.display()
    );
    path
}

struct SourceUiGuard {
    source: PathBuf,
    hidden: PathBuf,
    _holding_directory: tempfile::TempDir,
}

impl SourceUiGuard {
    fn hide(source: PathBuf) -> Self {
        assert!(
            source.is_dir(),
            "introspection UI source directory is missing"
        );
        let holding_directory = tempfile::Builder::new()
            .prefix(".deka-introspect-ui-hidden-")
            .tempdir_in(source.parent().expect("UI source parent"))
            .expect("create source UI holding directory");
        let hidden = holding_directory.path().join("ui");
        fs::rename(&source, &hidden).expect("hide introspection UI source directory");
        Self {
            source,
            hidden,
            _holding_directory: holding_directory,
        }
    }
}

impl Drop for SourceUiGuard {
    fn drop(&mut self) {
        if self.hidden.exists() && !self.source.exists() {
            fs::rename(&self.hidden, &self.source)
                .expect("restore introspection UI source directory");
        }
    }
}

#[test]
fn release_cli_runs_introspect_without_source_ui() {
    let artifact_directory = tempfile::tempdir().expect("create artifact directory");
    let artifact = artifact_directory.path().join("deka");
    fs::copy(release_cli(), &artifact).expect("copy release CLI out of target");

    let source_ui = repo_root()
        .join("crates")
        .join("introspect")
        .join("src")
        .join("ui");
    let _source_ui_guard = SourceUiGuard::hide(source_ui);
    let working_directory = tempfile::tempdir().expect("create unrelated working directory");

    let output = Command::new(&artifact)
        .args(["introspect", "inspect"])
        .current_dir(working_directory.path())
        .output()
        .expect("run copied release CLI");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        output.status.code(),
        Some(1),
        "missing inspect handler should be the command error: {combined}"
    );
    assert!(
        combined.contains("inspect requires a handler argument"),
        "introspect should reach the command handler after loading embedded UI: {combined}"
    );
    assert!(
        !combined.contains("cli.ts not found"),
        "the release artifact must not look for the source-tree UI: {combined}"
    );
}
