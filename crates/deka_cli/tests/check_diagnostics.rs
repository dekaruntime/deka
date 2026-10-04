use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn check(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deka"))
        .current_dir(directory)
        .arg("check")
        .args(arguments)
        .output()
        .unwrap()
}

fn failure(output: Output, expected: &str) {
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
}

#[test]
fn successful_check_writes_only_stderr_progress_without_executing_source() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("main.ds"),
        "import {echo} from \"io\";\necho(\"must not run\");",
    )
    .unwrap();
    let output = check(project.path(), &["main.ds"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "[check] main.ds - ok\n"
    );
}

#[test]
fn syntax_refusal_preserves_the_exact_source_location_and_message() {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("main.ds"), "let broken = ;").unwrap();
    failure(
        check(project.path(), &["main.ds"]),
        "[check] main.ds\n[check] 1:14: expected expression, found ``;``\n",
    );
}

#[test]
fn every_type_finding_has_the_agreed_prefix_in_source_order() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("main.ds"),
        "import {echo} from \"io\";\necho(1);\necho(false);",
    )
    .unwrap();
    failure(
        check(project.path(), &["main.ds"]),
        "[check] main.ds\n[check] 2:6: expected argument type `string`, found type `number`\n[check] 3:6: expected argument type `string`, found type `boolean`\n",
    );
}

#[test]
fn imported_module_refusal_identifies_the_actual_file_not_the_entry_file() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("helper.ds"),
        "export const broken: string = 1;",
    )
    .unwrap();
    fs::write(
        project.path().join("main.ds"),
        "import {broken} from \"./helper.ds\";\nconsole.log(broken);",
    )
    .unwrap();
    failure(
        check(project.path(), &["main.ds"]),
        &format!(
            "[check] main.ds\n[check] {}\n[check] 1:31: expected type `string`, found type `number`\n",
            project
                .path()
                .join("helper.ds")
                .canonicalize()
                .unwrap()
                .display()
        ),
    );
}

#[test]
fn absolute_source_path_with_a_colon_retains_its_context_once() {
    let project = tempfile::tempdir().unwrap();
    // Colons are allowed in POSIX filenames, while Windows rejects them.
    let name = if cfg!(windows) {
        "source.ds"
    } else {
        "source: part.ds"
    };
    let source = project.path().join(name);
    fs::write(&source, "import {echo} from \"io\";\necho(1);").unwrap();
    failure(
        check(project.path(), &[source.to_str().unwrap()]),
        &format!(
            "[check] {}\n[check] 2:6: expected argument type `string`, found type `number`\n",
            source.display()
        ),
    );
}

#[test]
fn missing_source_is_a_check_failure_and_bad_arguments_remain_usage_errors() {
    let project = tempfile::tempdir().unwrap();
    let missing = check(project.path(), &["missing.ds"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    let error = String::from_utf8(missing.stderr).unwrap();
    assert!(error.starts_with("[check] "), "{error}");
    assert!(error.contains("missing.ds"), "{error}");
    for arguments in [&["main.ds", "second.ds"][..], &["--unknown"][..]] {
        let usage = check(project.path(), arguments);
        assert_eq!(usage.status.code(), Some(2));
        assert!(usage.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&usage.stderr).starts_with("[check] "));
    }
}
