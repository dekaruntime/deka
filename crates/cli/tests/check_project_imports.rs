use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn run_check(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(cli_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run deka check")
}

#[test]
fn project_check_resolves_local_imports_through_module_graph() {
    let project = tempfile::tempdir().expect("create project");
    fs::write(project.path().join("deka.json"), "{}\n").unwrap();
    fs::write(
        project.path().join("main.ds"),
        "import { answer } from \"./helper.ds\"\nexport fn main() number { return answer }\n",
    )
    .unwrap();
    fs::write(
        project.path().join("helper.ds"),
        "export const answer: number = 42\n",
    )
    .unwrap();

    let output = run_check(project.path(), &["check", "main.ds"]);
    assert!(
        output.status.success(),
        "project check rejected a valid local import: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn single_file_check_does_not_load_project_imports() {
    let project = tempfile::tempdir().expect("create project");
    fs::write(project.path().join("deka.json"), "{}\n").unwrap();
    fs::write(
        project.path().join("main.ds"),
        "import { answer } from \"./helper.ds\"\nexport fn main() number { return answer }\n",
    )
    .unwrap();
    fs::write(
        project.path().join("helper.ds"),
        "export const =\n",
    )
    .unwrap();

    let output = run_check(project.path(), &["check", "--single-file", "main.ds"]);
    assert!(!output.status.success(), "standalone check unexpectedly resolved a local import");
    // dsc 0.9.0 (dsc#111) reports the unresolvable import itself instead of
    // leaving the name as an unknown identifier; either way the import must
    // not resolve in --single-file mode.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot resolve imported name `answer`")
            || stderr.contains("unknown identifier `answer`"),
        "standalone diagnostic changed unexpectedly: {}",
        stderr
    );
}
