#![cfg(unix)]
use corpus_gate::{Case, Stage, Status, evaluate, run_case};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn case(status: Status, stage: Stage) -> Case {
    Case {
        slug: "phase-fixture".into(),
        status,
        stage,
        source: "a staged source file".into(),
        entry_path: "main.ds".into(),
        files: vec![],
        expected_stdout: None,
        expected_diagnostic_contains: None,
        deka_json: None,
        packages: vec![],
    }
}

fn fake_cli(directory: &Path, body: &str) -> std::path::PathBuf {
    let file = directory.join("deka-fixture");
    fs::write(
        &file,
        format!("#!/bin/sh\nset -eu\nprintf '%s\\n' \"$1\" >> phases\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    file
}

#[test]
fn successful_execution_cannot_hide_a_check_refusal() {
    let scratch = tempfile::tempdir().unwrap();
    let cli = fake_cli(
        scratch.path(),
        "if [ \"$1\" = check ]; then echo 'type mismatch' >&2; exit 1; fi\necho accepted",
    );
    let expected = case(Status::Pass, Stage::Run);
    let result = run_case(&cli, &expected, scratch.path()).unwrap();
    assert!(!result.ok, "a runnable program must also pass check");
    assert!(result.transpile_failed);
    assert!(
        evaluate(&expected, &result)
            .iter()
            .any(|reason| reason.starts_with("status:"))
    );
    assert_eq!(
        fs::read_to_string(scratch.path().join("phase-fixture/phases")).unwrap(),
        "check\n"
    );
}

#[test]
fn failed_check_never_executes_the_program_and_diagnostics_still_match() {
    let scratch = tempfile::tempdir().unwrap();
    let cli = fake_cli(scratch.path(), "echo 'expected diagnostic' >&2\nexit 1");
    let mut expected = case(Status::Fail, Stage::Typecheck);
    expected.expected_diagnostic_contains = Some("expected diagnostic".into());
    let result = run_case(&cli, &expected, scratch.path()).unwrap();
    assert!(evaluate(&expected, &result).is_empty(), "{result:?}");
    assert_eq!(
        fs::read_to_string(scratch.path().join("phase-fixture/phases")).unwrap(),
        "check\n"
    );
}

#[test]
fn empty_stdout_runtime_failure_cannot_satisfy_a_compile_failure_expectation() {
    let scratch = tempfile::tempdir().unwrap();
    let cli = fake_cli(
        scratch.path(),
        "if [ \"$1\" = check ]; then echo 'OK main.ds'; exit 0; fi\necho 'authored failure' >&2\nexit 1",
    );
    let mut expected = case(Status::Fail, Stage::Typecheck);
    expected.expected_diagnostic_contains = Some("authored failure".into());
    let result = run_case(&cli, &expected, scratch.path()).unwrap();
    assert!(result.stdout.is_empty());
    assert!(!result.transpile_failed);
    assert!(
        evaluate(&expected, &result)
            .iter()
            .any(|reason| reason.starts_with("stage:"))
    );
    expected.stage = Stage::Run;
    assert!(evaluate(&expected, &result).is_empty(), "{result:?}");
    assert_eq!(
        fs::read_to_string(scratch.path().join("phase-fixture/phases")).unwrap(),
        "check\nrun\n"
    );
}

#[test]
fn both_output_pipes_drain_while_checking_and_running() {
    let scratch = tempfile::tempdir().unwrap();
    let cli = fake_cli(
        scratch.path(),
        "awk 'BEGIN {for(i=0;i<10000;i++) print \"0123456789abcdef\"}'\nawk 'BEGIN {for(i=0;i<10000;i++) print \"fedcba9876543210\"}' >&2",
    );
    let mut expected = case(Status::Pass, Stage::Run);
    expected.expected_stdout = Some("0123456789abcdef\n".repeat(10000));
    let result = run_case(&cli, &expected, scratch.path()).unwrap();
    assert!(evaluate(&expected, &result).is_empty(), "{result:?}");
    assert_eq!(result.stderr, "fedcba9876543210\n".repeat(10000).trim_end());
    assert_eq!(
        fs::read_to_string(scratch.path().join("phase-fixture/phases")).unwrap(),
        "check\nrun\n"
    );
}
