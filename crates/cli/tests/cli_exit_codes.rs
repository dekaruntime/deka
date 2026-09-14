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
fn unknown_top_level_command_exits_non_zero() {
    let output = std::process::Command::new(cli_bin())
        .args(["n0t-a-command"])
        .output()
        .expect("run deka");
    assert_eq!(output.status.code(), Some(2));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(text.contains("unknown argument 'n0t-a-command'"));
}

#[test]
fn unknown_subcommand_exits_non_zero_with_suggestion() {
    let output = std::process::Command::new(cli_bin())
        .args(["self", "fetchh"])
        .output()
        .expect("run deka");
    assert_eq!(output.status.code(), Some(2));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(text.contains("unknown subcommand 'fetchh' for 'self'"));
    assert!(text.contains("did you mean"));
    assert!(text.contains("self fetch"));
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

#[test]
#[cfg(feature = "native")]
fn task_json_empty_manifest_is_a_runtime_failure() {
    let project = tempfile::tempdir().expect("task project");
    std::fs::write(
        project.path().join("deka.json"),
        serde_json::json!({}).to_string(),
    )
    .unwrap();
    let output = Command::new(cli_bin())
        .args(["task", "--json"])
        .current_dir(project.path())
        .output()
        .expect("run deka task --json");
    assert_eq!(
        output.status.code(),
        Some(1),
        "task --json: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("no tasks found in deka.json"), "{text}");
}

// deka#1025: bare `deka task` (and `--list`) on the same empty manifest
// must report the identical condition with the identical exit code as
// `task --json` above -- not a different one, which is the exact defect
// #1010 exists to catch. Both are classified as a runtime failure (1),
// not a usage error (2): the invocation itself was correct, it's the
// project that has no tasks to run, which is project/environment state
// rather than a bad argument -- same reasoning already applied to
// `self test`'s "not fetched" and `auth whoami`'s "not logged in".
#[test]
#[cfg(feature = "native")]
fn task_bare_empty_manifest_matches_json_mode_exit_code() {
    let project = tempfile::tempdir().expect("task project");
    std::fs::write(
        project.path().join("deka.json"),
        serde_json::json!({}).to_string(),
    )
    .unwrap();

    let bare = Command::new(cli_bin())
        .arg("task")
        .current_dir(project.path())
        .output()
        .expect("run deka task");
    assert_eq!(
        bare.status.code(),
        Some(1),
        "bare task: stdout={} stderr={}",
        String::from_utf8_lossy(&bare.stdout),
        String::from_utf8_lossy(&bare.stderr)
    );
    assert!(
        String::from_utf8_lossy(&bare.stderr).contains("no tasks found in deka.json"),
        "{}",
        String::from_utf8_lossy(&bare.stderr)
    );

    let list = Command::new(cli_bin())
        .args(["task", "--list"])
        .current_dir(project.path())
        .output()
        .expect("run deka task --list");
    assert_eq!(
        list.status.code(),
        Some(1),
        "task --list: stdout={} stderr={}",
        String::from_utf8_lossy(&list.stdout),
        String::from_utf8_lossy(&list.stderr)
    );

    let json = Command::new(cli_bin())
        .args(["task", "--json"])
        .current_dir(project.path())
        .output()
        .expect("run deka task --json");
    assert_eq!(
        bare.status.code(),
        json.status.code(),
        "bare task and task --json must report the same condition with the same exit code"
    );
}

// deka#1010: usage errors exit 2 and runtime failures exit 1, consistently
// across the CLI and every owner crate — not just the top-level dispatch
// path exercised above. These tests drive the real built binary and assert
// the exit code, not just the printed message.

#[test]
#[cfg(feature = "native")]
fn db_bare_is_a_usage_error() {
    // Missing subcommand: usage error, exit 2. Previously exited 0 despite
    // printing "missing subcommand".
    let output = Command::new(cli_bin())
        .args(["db"])
        .output()
        .expect("run deka db");
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("missing subcommand"), "{text}");
}

#[test]
#[cfg(feature = "native")]
fn db_unknown_subcommand_lists_available_subcommands() {
    // Restores the specific-hint behavior displaced by #1008's generic
    // unknown-subcommand message (deka#1010).
    let output = Command::new(cli_bin())
        .args(["db", "bogus"])
        .output()
        .expect("run deka db bogus");
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("'db migrate'"), "{text}");
}

#[test]
#[cfg(feature = "native")]
fn cache_unknown_subcommand_restores_specific_clear_hint() {
    // deka#1010: `cache bogus` lost its specific "clear" hint when #1008's
    // generic unknown-subcommand message took over. No fuzzy match exists
    // between "bogus" and "clear", so the fallback (available subcommands)
    // is what restores it.
    let output = Command::new(cli_bin())
        .args(["cache", "bogus"])
        .output()
        .expect("run deka cache bogus");
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("'cache clear'"), "{text}");
}

#[test]
#[cfg(feature = "native")]
fn self_test_missing_suite_name_is_a_usage_error() {
    let output = Command::new(cli_bin())
        .args(["self", "test", "--list"])
        .output()
        .expect("run deka self test --list");
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("missing suite name"), "{text}");
}

#[test]
#[cfg(feature = "native")]
fn self_test_unknown_suite_is_a_usage_error() {
    let output = Command::new(cli_bin())
        .args(["self", "test", "bogus-suite"])
        .output()
        .expect("run deka self test bogus-suite");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
#[cfg(feature = "native")]
fn introspect_inspect_missing_handler_is_a_usage_error() {
    let output = Command::new(cli_bin())
        .args(["introspect", "inspect"])
        .output()
        .expect("run deka introspect inspect");
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("requires a handler argument"), "{text}");
}

#[test]
#[cfg(feature = "native")]
fn introspect_kill_missing_handler_is_a_usage_error() {
    let output = Command::new(cli_bin())
        .args(["introspect", "kill"])
        .output()
        .expect("run deka introspect kill");
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("requires a handler argument"), "{text}");
}

#[test]
#[cfg(feature = "native")]
fn link_missing_argument_is_a_usage_error() {
    let output = Command::new(cli_bin())
        .args(["link"])
        .output()
        .expect("run deka link");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
#[cfg(feature = "native")]
fn link_outside_a_project_is_a_runtime_failure() {
    // A well-formed argument (a real directory) but no deka.json anywhere
    // in the ancestor chain: the argument was fine, the environment
    // wasn't. Runtime failure, exit 1 (deka#1010).
    let project = tempfile::tempdir().expect("link project");
    let target = project.path().join("pkg");
    std::fs::create_dir_all(&target).unwrap();
    let output = Command::new(cli_bin())
        .args(["link", target.to_str().unwrap()])
        .current_dir(project.path())
        .output()
        .expect("run deka link");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
#[cfg(feature = "native")]
fn deploy_run_missing_pipeline_path_is_a_usage_error() {
    let output = Command::new(cli_bin())
        .args(["deploy", "run"])
        .output()
        .expect("run deka deploy run");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
#[cfg(feature = "native")]
fn deploy_run_unparsable_pipeline_is_a_runtime_failure() {
    let project = tempfile::tempdir().expect("deploy project");
    let pipeline = project.path().join("linkhash.yaml");
    std::fs::write(&pipeline, "not: [valid, yaml: for-a-pipeline").unwrap();
    let output = Command::new(cli_bin())
        .args(["deploy", "run", pipeline.to_str().unwrap()])
        .output()
        .expect("run deka deploy run");
    assert_eq!(output.status.code(), Some(1));
}
