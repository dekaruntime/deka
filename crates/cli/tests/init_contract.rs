//! The default project must be safe to inspect and must follow the four
//! project-root conventions without extra setup.

use std::fs;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

#[test]
fn init_help_does_not_create_a_project() {
    let project = tempfile::tempdir().expect("tempdir");
    let output = Command::new(cli_bin())
        .args(["init", "--help"])
        .current_dir(project.path())
        .output()
        .expect("run deka init --help");

    assert!(
        output.status.success(),
        "deka init --help failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !project.path().join("deka.json").exists(),
        "deka init --help must not initialize a project"
    );
}

#[test]
fn init_creates_the_conventional_project_roots_and_formatted_templates() {
    let project = tempfile::tempdir().expect("tempdir");
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(project.path())
        .output()
        .expect("run deka init");
    assert!(output.status.success(), "deka init must succeed");

    for directory in ["app", "api", "src", "public"] {
        assert!(
            project.path().join(directory).is_dir(),
            "init must create {directory}/"
        );
    }

    for file in ["app/page.dsx", "app/layout.dsx", "app/not-found.dsx"] {
        let source = fs::read_to_string(project.path().join(file)).expect("read template");
        assert!(
            !source.contains("    "),
            "{file} must use formatter indentation"
        );
        assert!(
            !source.contains("</section>;"),
            "{file} must omit stale semicolons"
        );
    }
}
