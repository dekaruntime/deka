//! End-to-end: a mini corpus plus a mock CLI exercises the whole gate —
//! fixture layout, evaluation, list membership, and exit codes.
use std::path::Path;
use std::process::Command;

fn write(dir: &Path, name: &str, content: &str, mode: Option<u32>) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
}

/// A stand-in for deka: prints "mock-ok" and exits 0 when the entry source
/// carries the mockpass marker; otherwise reports a run failure on stderr.
/// Checking succeeds independently and never executes the source.
fn mock_cli(dir: &Path) -> std::path::PathBuf {
    let script = "#!/bin/sh\nif [ \"$1\" = check ]; then exit 0; fi\nif grep -q mockpass \"$2\" 2>/dev/null; then echo mock-ok; exit 0; fi\necho \"mock badness\" >&2; exit 1\n";
    write(dir, "mock-deka", script, Some(0o755));
    dir.join("mock-deka")
}

fn mini_corpus(dir: &Path) {
    write(
        dir,
        "corpus/basics/ok_case/ok_case.pass.ds",
        "const mockpass = 1\n",
        None,
    );
    write(
        dir,
        "corpus/basics/ok_case/ok_case.json",
        "{\"title\":\"ok\"}\n",
        None,
    );
    write(
        dir,
        "corpus/basics/ok_case/ok_case.stdout",
        "mock-ok\n",
        None,
    );
    write(
        dir,
        "corpus/basics/bad_case/bad_case.fail.ds",
        "nope\n",
        None,
    );
    write(
        dir,
        "corpus/basics/bad_case/bad_case.json",
        "{\"stage\":\"run\",\"expectedDiagnosticContains\":\"mock badness\"}\n",
        None,
    );
}

fn gate(binary: &Path, corpus: &Path, list: &Path, deka: &Path) -> std::process::Output {
    Command::new(binary)
        .args([corpus, list, deka])
        .output()
        .unwrap()
}

#[test]
fn listed_cases_that_match_pass_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    mini_corpus(dir.path());
    let deka = mock_cli(dir.path());
    write(
        dir.path(),
        "list.txt",
        "basics-ok-case\nbasics-bad-case\n",
        None,
    );
    let output = gate(
        &std::path::PathBuf::from(env!("CARGO_BIN_EXE_corpus-gate")),
        &dir.path().join("corpus"),
        &dir.path().join("list.txt"),
        &deka,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_case_failing_its_expectation_fails_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    mini_corpus(dir.path());
    let deka = mock_cli(dir.path());
    // bad_case expects "something else", which the mock never prints.
    write(
        dir.path(),
        "corpus/basics/bad_case/bad_case.json",
        "{\"stage\":\"run\",\"expectedDiagnosticContains\":\"something else\"}\n",
        None,
    );
    write(
        dir.path(),
        "list.txt",
        "basics-ok-case\nbasics-bad-case\n",
        None,
    );
    let output = gate(
        &std::path::PathBuf::from(env!("CARGO_BIN_EXE_corpus-gate")),
        &dir.path().join("corpus"),
        &dir.path().join("list.txt"),
        &deka,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("FAIL basics-bad-case"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_listed_slug_missing_from_the_corpus_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    mini_corpus(dir.path());
    let deka = mock_cli(dir.path());
    write(
        dir.path(),
        "list.txt",
        "basics-ok-case\nbasics-ghost\n",
        None,
    );
    let output = gate(
        &std::path::PathBuf::from(env!("CARGO_BIN_EXE_corpus-gate")),
        &dir.path().join("corpus"),
        &dir.path().join("list.txt"),
        &deka,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("basics-ghost"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unlisted_cases_do_not_run() {
    let dir = tempfile::tempdir().unwrap();
    mini_corpus(dir.path());
    let deka = mock_cli(dir.path());
    // Only the passing case is listed; the failing one is ignored.
    write(dir.path(), "list.txt", "basics-ok-case\n", None);
    let output = gate(
        &std::path::PathBuf::from(env!("CARGO_BIN_EXE_corpus-gate")),
        &dir.path().join("corpus"),
        &dir.path().join("list.txt"),
        &deka,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_json_metadata_still_runs_the_source_and_checks_its_output() {
    let dir = tempfile::tempdir().unwrap();
    let deka = mock_cli(dir.path());
    for (name, source) in [("native", "const mockpass = 1"), ("broken", "broken")] {
        write(
            dir.path(),
            &format!("corpus/json/{name}/{name}.pass.ds"),
            source,
            None,
        );
        write(
            dir.path(),
            &format!("corpus/json/{name}/{name}.json"),
            r#"{"packages":["json"]}"#,
            None,
        );
        write(
            dir.path(),
            &format!("corpus/json/{name}/{name}.stdout"),
            "mock-ok\n",
            None,
        );
    }
    let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_corpus-gate"));
    let corpus = dir.path().join("corpus");
    let list = dir.path().join("list.txt");
    write(dir.path(), "list.txt", "json-native\n", None);
    assert!(gate(&binary, &corpus, &list, &deka).status.success());
    write(dir.path(), "list.txt", "json-broken\n", None);
    assert!(!gate(&binary, &corpus, &list, &deka).status.success());
    write(
        dir.path(),
        "corpus/json/native/native.pass.ds",
        "import { x } from \"json\"; const mockpass = 1;",
        None,
    );
    write(dir.path(), "list.txt", "json-native\n", None);
    let output = gate(&binary, &corpus, &list, &deka);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("offline and cannot install"));
    write(
        dir.path(),
        "corpus/json/native/native.json",
        r#"{"packages":["json","fs"]}"#,
        None,
    );
    write(dir.path(), "list.txt", "json-native\n", None);
    let output = gate(&binary, &corpus, &list, &deka);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("offline and cannot install"));
}
