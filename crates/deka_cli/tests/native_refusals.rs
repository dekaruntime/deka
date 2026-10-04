use std::{fs, path::Path, process::Command};
fn refusal(dir: &Path, entry: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_deka"))
        .current_dir(dir)
        .args(["check", entry])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    String::from_utf8(out.stderr).unwrap()
}
#[test]
fn lowerer_refusals_name_the_construct_and_keep_its_source_position() {
    let project = tempfile::tempdir().unwrap();
    for (name, source, finding) in [
        (
            "builtin.ds",
            "\n\nconst value = parseNumber(\"42\");",
            "3:15: unknown built-in parseNumber",
        ),
        (
            "view.dsx",
            "\n\nconst node = <article>hello</article>;",
            "3:14: unsupported VM UI primitive: article",
        ),
        (
            "unsafe.ds",
            "\n\nconst value = unsafe<number> { return 42; };",
            "3:15: unsafe blocks are not supported by the native VM",
        ),
    ] {
        fs::write(project.path().join(name), source).unwrap();
        assert_eq!(
            refusal(project.path(), name),
            format!("[check] {name}\n[check] {finding}\n")
        );
    }
}
#[test]
fn nested_lowering_keeps_the_innermost_refusal_without_duplicate_locations() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("nested.ds"),
        "\n\nconst value = string(parseNumber(\"42\"));",
    )
    .unwrap();
    assert_eq!(
        refusal(project.path(), "nested.ds"),
        "[check] nested.ds\n[check] 3:22: unknown built-in parseNumber\n"
    );
}
#[test]
fn imported_refusal_identifies_its_actual_file_even_with_a_colon_in_the_name() {
    let project = tempfile::tempdir().unwrap();
    let name = if cfg!(windows) {
        "helper.ds"
    } else {
        "helper: part.ds"
    };
    fs::write(
        project.path().join(name),
        "\nexport fn value() Option<number> {return parseNumber(\"42\");}",
    )
    .unwrap();
    fs::write(
        project.path().join("main.ds"),
        format!("import {{value}} from {:?};", format!("./{name}")),
    )
    .unwrap();
    assert_eq!(
        refusal(project.path(), "main.ds"),
        format!(
            "[check] main.ds\n[check] {}\n[check] 2:42: unknown built-in parseNumber\n",
            project.path().join(name).canonicalize().unwrap().display()
        )
    );
}
