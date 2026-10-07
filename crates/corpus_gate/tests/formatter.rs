#![cfg(unix)]
use corpus_gate::{Case, Stage, Status, evaluate, run_case};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn fixture(source: &str) -> Case {
    Case {
        slug: "formatter-effect-fixture".into(),
        status: Status::Pass,
        stage: Stage::Run,
        source: source.into(),
        entry_path: "main.ds".into(),
        files: vec![],
        expected_stdout: Some("7\n".into()),
        expected_diagnostic_contains: None,
        deka_json: None,
        packages: vec![],
    }
}

fn cli(directory: &Path, formatting: &str, checking: &str, running: &str) -> std::path::PathBuf {
    let path = directory.join("formatter-cli-fixture");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$1\" >> phases\ncase \"$1\" in\nfmt) {formatting};;\ncheck) {checking};;\nrun) {running};;\n*) exit 2;;\nesac\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn formatting_runs_twice_and_the_canonical_source_is_executed() {
    let temp = tempfile::tempdir().unwrap();
    let executable = cli(
        temp.path(),
        "printf 'const value = 7\\n' > \"$2\"",
        "exit 0",
        "echo 7",
    );
    let expected = fixture("const value=7\n");
    let result = run_case(&executable, &expected, temp.path()).unwrap();
    assert!(evaluate(&expected, &result).is_empty(), "{result:?}");
    let project = temp.path().join(&expected.slug);
    assert_eq!(
        fs::read_to_string(project.join("test.ds")).unwrap(),
        "const value = 7\n"
    );
    assert_eq!(
        fs::read_to_string(project.join("phases")).unwrap(),
        "check\nrun\nfmt\nfmt\ncheck\nrun\n"
    );
}

#[test]
fn unchanged_invalid_source_keeps_the_recorded_failure() {
    let temp = tempfile::tempdir().unwrap();
    let executable = cli(
        temp.path(),
        "exit 0",
        "echo 'expected expression' >&2; exit 1",
        "echo 7",
    );
    let mut expected = fixture("const value=\n");
    expected.status = Status::Fail;
    expected.stage = Stage::Parse;
    expected.expected_stdout = None;
    expected.expected_diagnostic_contains = Some("expected expression".into());
    let result = run_case(&executable, &expected, temp.path()).unwrap();
    assert!(evaluate(&expected, &result).is_empty(), "{result:?}");
    let project = temp.path().join(&expected.slug);
    assert_eq!(
        fs::read_to_string(project.join("test.ds")).unwrap(),
        expected.source
    );
    assert_eq!(
        fs::read_to_string(project.join("phases")).unwrap(),
        "check\nfmt\nfmt\ncheck\n"
    );
}

#[test]
fn changed_program_output_cannot_pass_the_formatter_gate() {
    let temp = tempfile::tempdir().unwrap();
    let executable = cli(
        temp.path(),
        "printf 'const value = 8\\n' > \"$2\"",
        "exit 0",
        "if grep -q '8' \"$2\"; then echo 8; else echo 7; fi",
    );
    let error = run_case(&executable, &fixture("const value=7\n"), temp.path()).unwrap_err();
    assert!(
        error.contains("formatting changed observed check/run behavior"),
        "{error}"
    );
}

#[test]
fn formatter_must_not_repair_invalid_archived_source() {
    let temp = tempfile::tempdir().unwrap();
    let executable = cli(
        temp.path(),
        "printf 'const value = 7\\n' > \"$2\"",
        "exit 0",
        "echo 7",
    );
    let error = run_case(&executable, &fixture("const value=\n"), temp.path()).unwrap_err();
    assert!(
        error.contains("formatting changed invalid source"),
        "{error}"
    );
}

#[test]
fn lost_comments_and_non_idempotent_output_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let executable = cli(
        temp.path(),
        "printf 'const value = 7\\n' > \"$2\"",
        "exit 0",
        "echo 7",
    );
    let error = run_case(
        &executable,
        &fixture("// keep this\nconst value=7\n"),
        temp.path(),
    )
    .unwrap_err();
    assert!(error.contains("formatting changed comments"), "{error}");

    let executable = cli(
        temp.path(),
        "if [ -f first-pass ]; then printf 'const value = 8\\n' > \"$2\"; else printf 'const value = 7\\n' > \"$2\"; touch first-pass; fi",
        "exit 0",
        "echo 7",
    );
    let error = run_case(&executable, &fixture("const value=7\n"), temp.path()).unwrap_err();
    assert!(error.contains("formatter second pass changed"), "{error}");
}

#[test]
fn broken_formatter_process_does_not_count_as_a_parse_failure() {
    let temp = tempfile::tempdir().unwrap();
    let executable = cli(
        temp.path(),
        "echo 'formatter unavailable' >&2; exit 1",
        "echo 'expected expression' >&2; exit 1",
        "echo 7",
    );
    let error = run_case(&executable, &fixture("const value=\n"), temp.path()).unwrap_err();
    assert!(error.contains("formatter failed"), "{error}");
}
