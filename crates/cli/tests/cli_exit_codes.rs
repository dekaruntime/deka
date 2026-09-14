// deka#738 (F1): unrecognised arguments are usage errors — the CLI must exit
// non-zero (2, per the GNU usage-error convention). Previously every
// parse-error path printed `[cli] unknown argument ...` and fell through to
// exit 0, so a typo'd flag (e.g. inside `deka build ...`) reported success.
//
// These tests assert exit codes via the real binary, not message text.

use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn exit_code(args: &[&str]) -> Option<i32> {
    Command::new(cli_bin())
        .args(args)
        .output()
        .expect("run deka")
        .status
        .code()
}

#[test]
fn unknown_flag_exits_non_zero() {
    assert_eq!(exit_code(&["--bogus-flag-xyz"]), Some(2));
}

#[test]
fn build_unknown_flag_exits_non_zero() {
    assert_eq!(exit_code(&["build", "--bogus-xyz"]), Some(2));
}

#[test]
fn missing_param_value_exits_non_zero() {
    assert_eq!(exit_code(&["--port"]), Some(2));
}

#[test]
fn help_exits_zero() {
    assert_eq!(exit_code(&["--help"]), Some(0));
}

#[test]
fn duplicate_typo_suggestions_are_deduplicated_and_disambiguated() {
    let output = std::process::Command::new(cli_bin())
        .args(["--instal"])
        .output()
        .expect("run deka");
    assert_eq!(output.status.code(), Some(2));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(text.contains("did you mean"));
    let suggestions = text
        .split("did you mean ")
        .nth(1)
        .and_then(|tail| tail.split('?').next())
        .unwrap_or("");
    let parsed = suggestions
        .split(", ")
        .map(|value| value.trim().trim_matches('\''))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    assert!(
        parsed.contains(&"install"),
        "expected install in suggestions: {parsed:?}"
    );
    assert!(
        parsed.contains(&"pkg install"),
        "expected subcommand context in suggestions: {parsed:?}"
    );
    let mut deduped = parsed.clone();
    deduped.sort_unstable();
    deduped.dedup();
    assert_eq!(
        parsed.len(),
        deduped.len(),
        "did-you-mean suggestions should be deduplicated: {parsed:?}"
    );
}

#[test]
fn suggestion_list_is_capped() {
    let output = std::process::Command::new(cli_bin())
        .args(["--hel"])
        .output()
        .expect("run deka");
    assert_eq!(output.status.code(), Some(2));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    let suggestions = text
        .split("did you mean ")
        .nth(1)
        .and_then(|tail| tail.split('?').next())
        .unwrap_or("")
        .split(", ")
        .map(|value| value.trim().trim_matches('\''))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    assert!(suggestions.len() <= 3, "{suggestions:?}");
}

// deka#975: assert the process status, including non-1 child failures carried
// through dependency and wildcard execution in the task owner crate.
#[test]
#[cfg(feature = "native")]
fn task_propagates_child_exit_codes() {
    let project = tempfile::tempdir().expect("task project");
    std::fs::write(
        project.path().join("deka.json"),
        serde_json::json!({
            "tasks": {
                "dev": "exit 1",
                "fail": "exit 23",
                "dependent": {
                    "command": "echo should-not-run > ran.txt",
                    "dependencies": ["fail"]
                },
                "lint-fail": "exit 7",
                "ok": "exit 0"
            }
        })
        .to_string(),
    )
    .unwrap();
    for (task, expected) in [
        ("dev", 1),
        ("fail", 23),
        ("dependent", 23),
        ("lint-*", 7),
        ("ok", 0),
        ("missing", 1),
    ] {
        let output = Command::new(cli_bin())
            .args(["task", task])
            .current_dir(project.path())
            .output()
            .expect("run task");
        assert_eq!(
            output.status.code(),
            Some(expected),
            "task {task}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        !project.path().join("ran.txt").exists(),
        "failed dependency must stop its parent"
    );
}
