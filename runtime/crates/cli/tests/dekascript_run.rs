use std::fs;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn run_dekascript(name: &str, source: &str, expected_output: &str) {
    let project = tempfile::tempdir().expect("create DekaScript project");
    fs::write(project.path().join("deka.json"), "{}\n").expect("write project manifest");
    let entry = project.path().join(format!("{name}.ds"));
    fs::write(&entry, source).expect("write DekaScript entry");

    let output = Command::new(cli_bin())
        .args(["run", entry.to_str().expect("UTF-8 entry")])
        .current_dir(project.path())
        .output()
        .expect("run DekaScript entry through the CLI runtime");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "DekaScript run failed: {combined}");
    assert!(
        combined.contains(expected_output),
        "missing {expected_output:?} in runtime output: {combined}"
    );
}

#[test]
fn run_executes_dekascript_comparison_candidate() {
    run_dekascript(
        "comparison",
        "const result = 7 > 3;\nprint(result);\n",
        "true",
    );
}

#[test]
fn run_executes_dekascript_boolean_candidate() {
    run_dekascript(
        "boolean",
        "const result = true && !false;\nprint(result);\n",
        "true",
    );
}

#[test]
fn run_executes_dekascript_if_else_candidate() {
    run_dekascript(
        "if_else",
        "const ready = false;\nif (ready) { print(\"wrong-branch\"); } else { print(\"if-else-ok\"); }\n",
        "if-else-ok",
    );
}
