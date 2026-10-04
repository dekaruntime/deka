use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn check(directory: &Path, source: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deka"))
        .current_dir(directory)
        .args(["check", source])
        .output()
        .unwrap()
}
fn failure(output: Output, expected: &str) {
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
}
fn write(directory: &Path, name: &str, source: &str) {
    fs::write(directory.join(name), source).unwrap();
}
fn missing_cause(directory: &Path, specifier: &str) -> String {
    let target = directory.canonicalize().unwrap().join(specifier);
    format!(
        "{}: {}",
        target.display(),
        target.canonicalize().unwrap_err()
    )
}
#[test]
fn missing_relative_import_retains_the_import_span_and_os_cause() {
    let project = tempfile::tempdir().unwrap();
    write(
        project.path(),
        "main.ds",
        "\n\n    import {value} from \"./missing.ds\";\n",
    );
    failure(
        check(project.path(), "main.ds"),
        &format!(
            "[check] main.ds\n[check] 3:5: {}\n",
            missing_cause(project.path(), "./missing.ds")
        ),
    );
}
#[test]
fn missing_reexport_retains_the_export_span_and_original_cause() {
    let project = tempfile::tempdir().unwrap();
    write(
        project.path(),
        "main.ds",
        "\n  export {value} from \"./missing.ds\";\n",
    );
    failure(
        check(project.path(), "main.ds"),
        &format!(
            "[check] main.ds\n[check] 2:3: {}\n",
            missing_cause(project.path(), "./missing.ds")
        ),
    );
}
#[test]
fn undeclared_and_uninstalled_packages_keep_their_specific_causes() {
    let project = tempfile::tempdir().unwrap();
    write(
        project.path(),
        "main.ds",
        "\n   import {value} from \"@example/missing\";\n",
    );
    write(project.path(), "deka.json", "{\"dependencies\":{}}");
    failure(
        check(project.path(), "main.ds"),
        "[check] main.ds\n[check] 2:4: package @example/missing is not declared in deka.json dependencies\n",
    );
    write(
        project.path(),
        "deka.json",
        "{\"dependencies\":{\"@example/missing\":\"1.0.0\"}}",
    );
    failure(
        check(project.path(), "main.ds"),
        "[check] main.ds\n[check] 2:4: package @example/missing is not installed (ds_modules/@example/missing is missing)\n",
    );
}
#[test]
fn package_without_a_project_manifest_points_to_the_import_statement() {
    let project = tempfile::tempdir().unwrap();
    write(
        project.path(),
        "main.ds",
        "\n import {value} from \"@example/missing\";\n",
    );
    failure(
        check(project.path(), "main.ds"),
        "[check] main.ds\n[check] 2:2: package @example/missing cannot be resolved: deka.json not found in this directory or any parent\n",
    );
}
#[test]
fn self_import_retains_the_existing_cyclic_module_cause() {
    let project = tempfile::tempdir().unwrap();
    write(
        project.path(),
        "main.ds",
        "\n\n import {value} from \"./main.ds\";\n",
    );
    failure(
        check(project.path(), "main.ds"),
        &format!(
            "[check] main.ds\n[check] 3:2: cyclic module import: {}\n",
            project
                .path()
                .join("main.ds")
                .canonicalize()
                .unwrap()
                .display()
        ),
    );
}
#[test]
fn nested_import_failure_reports_the_dependency_file_once() {
    let project = tempfile::tempdir().unwrap();
    write(
        project.path(),
        "main.ds",
        "import {value} from \"./helper.ds\";\n",
    );
    write(
        project.path(),
        "helper.ds",
        "\n\n    import {value} from \"./missing.ds\";\nexport {value};\n",
    );
    failure(
        check(project.path(), "main.ds"),
        &format!(
            "[check] main.ds\n[check] {}\n[check] 3:5: {}\n",
            project
                .path()
                .join("helper.ds")
                .canonicalize()
                .unwrap()
                .display(),
            missing_cause(project.path(), "./missing.ds")
        ),
    );
}
#[test]
fn imported_parse_error_keeps_its_own_span_without_an_outer_import_prefix() {
    let project = tempfile::tempdir().unwrap();
    write(
        project.path(),
        "main.ds",
        "\nimport {value} from \"./helper.ds\";\n",
    );
    write(project.path(), "helper.ds", "let broken = ;");
    failure(
        check(project.path(), "main.ds"),
        &format!(
            "[check] main.ds\n[check] {}\n[check] 1:14: expected expression, found ``;``\n",
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
fn run_uses_the_same_positioned_module_graph_refusal() {
    let project = tempfile::tempdir().unwrap();
    write(
        project.path(),
        "main.ds",
        "\n    import {value} from \"./missing.ds\";\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_deka"))
        .current_dir(project.path())
        .args(["run", "main.ds"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    let source = project.path().join("main.ds").canonicalize().unwrap();
    assert!(
        error.contains(&format!(
            "{}: 2:5: {}",
            source.display(),
            missing_cause(project.path(), "./missing.ds")
        )),
        "{error}"
    );
}

#[test]
fn importer_paths_with_colons_keep_one_source_context() {
    let project = tempfile::tempdir().unwrap();
    let name = if cfg!(windows) {
        "main.ds"
    } else {
        "main: part.ds"
    };
    write(
        project.path(),
        name,
        "\n  import {value} from \"./missing.ds\";\n",
    );
    failure(
        check(project.path(), name),
        &format!(
            "[check] {name}\n[check] 2:3: {}\n",
            missing_cause(project.path(), "./missing.ds")
        ),
    );
}
