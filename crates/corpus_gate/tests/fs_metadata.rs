#![cfg(unix)]
use corpus_gate::{Case, Stage, Status, evaluate, run_case};
use std::path::{Path, PathBuf};
fn fixture() -> Case {
    Case {
        slug: "fs-read-missing".into(),
        status: Status::Pass,
        stage: Stage::Run,
        source: include_str!("../src/legacy/fs/read_missing.ds").into(),
        entry_path: "read_missing.pass.ds".into(),
        files: vec![],
        expected_stdout: Some("err\n".into()),
        expected_diagnostic_contains: None,
        packages: vec!["fs".into()],
        deka_json: Some(
            serde_json::json!({"name":"conformance-fixture","security":{"allow":{"read":["./"],"write":[".cache","php_modules"]},"prompt":false}}),
        ),
    }
}
#[cfg(unix)]
fn cli(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("deka-mock");
    std::fs::write(&path,"#!/bin/sh\nif [ \"$1\" = check ]; then touch observed-check; exit 0; fi\ntouch observed-run\nprintf 'err\\n'\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}
#[cfg(unix)]
#[test]
fn exact_filesystem_metadata_still_requires_check_run_and_stdout() {
    let root = tempfile::tempdir().unwrap();
    let case = fixture();
    let result = run_case(&cli(root.path()), &case, root.path()).unwrap();
    assert!(evaluate(&case, &result).is_empty());
    assert!(root.path().join(&case.slug).join("observed-check").exists());
    assert!(root.path().join(&case.slug).join("observed-run").exists());
}
#[cfg(unix)]
#[test]
fn changed_filesystem_source_metadata_output_and_other_packages_fail_closed() {
    for change in 0..9 {
        let root = tempfile::tempdir().unwrap();
        let mut case = fixture();
        match change {
            0 => case.source.push_str("echo(\"changed\")\n"),
            1 => case.packages.push("real-package".into()),
            2 => case.deka_json = None,
            3 => case.expected_stdout = Some("wrong\n".into()),
            4 => case.status = Status::Fail,
            5 => case.stage = Stage::Typecheck,
            6 => case.slug = "fs-unrelated".into(),
            7 => case.deka_json.as_mut().unwrap()["security"]["prompt"] = serde_json::json!(true),
            8 => case
                .files
                .push(("extra.ds".into(), "echo(\"extra\")".into())),
            _ => unreachable!(),
        }
        assert!(
            run_case(&cli(root.path()), &case, root.path())
                .unwrap_err()
                .contains("offline"),
            "case {change}"
        );
        assert!(!root.path().join(&case.slug).join("observed-run").exists());
    }
}
