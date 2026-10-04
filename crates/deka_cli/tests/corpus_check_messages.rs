use corpus_gate::{Case, Stage, Status, evaluate, run_case};
use std::path::Path;

fn case(source: &str) -> Case {
    Case {
        slug: "native-check-messages".into(),
        status: Status::Fail,
        stage: Stage::Typecheck,
        source: source.into(),
        entry_path: "main.ds".into(),
        files: vec![],
        expected_stdout: None,
        expected_diagnostic_contains: Some("expected argument type".into()),
        deka_json: None,
        packages: vec![],
    }
}

#[test]
fn actual_cli_findings_keep_primary_cause_order_and_source_positions() {
    let scratch = tempfile::tempdir().unwrap();
    let case = case("import {echo} from \"io\";\necho(1);\necho(false);");
    let result = run_case(Path::new(env!("CARGO_BIN_EXE_deka")), &case, scratch.path()).unwrap();
    assert!(!result.ok && result.transpile_failed);
    assert!(result.stdout.is_empty());
    assert!(evaluate(&case, &result).is_empty());
    assert_eq!(
        result.diagnostics,
        [
            "expected argument type `string`, found type `number`",
            "expected argument type `string`, found type `boolean`"
        ]
    );
    assert_eq!(
        result.error.as_deref(),
        Some("expected argument type `string`, found type `number`")
    );
    assert_eq!(
        result.stderr,
        "[check] ./test.ds\n[check] 2:6: expected argument type `string`, found type `number`\n[check] 3:6: expected argument type `string`, found type `boolean`"
    );
}

#[test]
fn imported_module_context_never_becomes_the_actual_cli_message() {
    let scratch = tempfile::tempdir().unwrap();
    let mut case = case("import {broken} from \"./helper^name.ds\";\nconsole.log(broken);");
    case.expected_diagnostic_contains = Some("expected type `string`, found type `number`".into());
    case.files.push((
        "helper^name.ds".into(),
        "export const broken: string = 1;".into(),
    ));
    let result = run_case(Path::new(env!("CARGO_BIN_EXE_deka")), &case, scratch.path()).unwrap();
    assert!(!result.ok && result.transpile_failed);
    assert!(evaluate(&case, &result).is_empty());
    assert_eq!(
        result.diagnostics,
        ["expected type `string`, found type `number`"]
    );
    assert_eq!(
        result.error.as_deref(),
        Some("expected type `string`, found type `number`")
    );
    let helper = scratch
        .path()
        .join(&case.slug)
        .join("helper^name.ds")
        .canonicalize()
        .unwrap();
    assert_eq!(
        result.stderr,
        format!(
            "[check] ./main.ds\n[check] {}\n[check] 1:31: expected type `string`, found type `number`",
            helper.display()
        )
    );
}

#[test]
fn actual_formatter_position_changes_do_not_change_gate_messages() {
    let scratch = tempfile::tempdir().unwrap();
    let mut case = case("import {echo} from \"io\"; echo(1); echo(false);");
    let deka = Path::new(env!("CARGO_BIN_EXE_deka"));
    let before = run_case(deka, &case, scratch.path()).unwrap();
    let directory = scratch.path().join(&case.slug);
    let output = std::process::Command::new(deka)
        .arg("fmt")
        .arg("test.ds")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let formatted = std::fs::read_to_string(directory.join("test.ds")).unwrap();
    assert_ne!(
        formatted, case.source,
        "actual formatter did not change source"
    );
    case.source = formatted;
    let after = run_case(deka, &case, scratch.path()).unwrap();
    assert!(evaluate(&case, &before).is_empty());
    assert!(evaluate(&case, &after).is_empty());
    assert_eq!(before.diagnostics.len(), 2);
    assert_eq!(before.diagnostics, after.diagnostics);
    assert_eq!(before.error, after.error);
    assert_ne!(
        before.stderr, after.stderr,
        "source positions should have changed"
    );
}
