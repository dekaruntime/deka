use corpus_gate::{Case, Stage, Status, evaluate, run_case};
use std::path::Path;

fn fixture(source: &str, status: Status, stage: Stage) -> Case {
    Case {
        slug: "validation-fixture".into(),
        source: source.into(),
        entry_path: "main.ds".into(),
        status,
        stage,
        files: vec![],
        expected_stdout: None,
        expected_diagnostic_contains: None,
        deka_json: None,
        packages: vec![],
    }
}

#[test]
fn actual_cli_checks_then_runs_and_keeps_only_program_output() {
    let scratch = tempfile::tempdir().unwrap();
    let mut case = fixture(
        "import {echo} from \"io\"; echo(\"7\");",
        Status::Pass,
        Stage::Run,
    );
    case.expected_stdout = Some("7\n".into());
    let result = run_case(Path::new(env!("CARGO_BIN_EXE_deka")), &case, scratch.path()).unwrap();
    assert!(evaluate(&case, &result).is_empty(), "{result:?}");
    assert_eq!(result.stdout, "7\n");
}

#[test]
fn actual_runtime_fault_without_stdout_is_never_a_typecheck_failure() {
    let scratch = tempfile::tempdir().unwrap();
    let mut case = fixture("panic(\"authored failure\");", Status::Fail, Stage::Run);
    case.expected_diagnostic_contains = Some("authored failure".into());
    let result = run_case(Path::new(env!("CARGO_BIN_EXE_deka")), &case, scratch.path()).unwrap();
    assert!(!result.ok);
    assert!(result.stdout.is_empty());
    assert!(!result.transpile_failed, "{result:?}");
    assert!(evaluate(&case, &result).is_empty(), "{result:?}");
    case.stage = Stage::Typecheck;
    assert!(
        evaluate(&case, &result)
            .iter()
            .any(|reason| reason.starts_with("stage:"))
    );
}

#[test]
fn actual_compile_refusal_keeps_its_expected_diagnostic() {
    let scratch = tempfile::tempdir().unwrap();
    let mut case = fixture(
        "import {echo} from \"io\"; echo(missing);",
        Status::Fail,
        Stage::Typecheck,
    );
    case.expected_diagnostic_contains = Some("missing".into());
    let result = run_case(Path::new(env!("CARGO_BIN_EXE_deka")), &case, scratch.path()).unwrap();
    assert!(!result.ok);
    assert!(result.transpile_failed);
    assert!(evaluate(&case, &result).is_empty(), "{result:?}");
}
