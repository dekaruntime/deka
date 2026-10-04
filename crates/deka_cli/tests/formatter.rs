use std::{
    fs,
    io::Write,
    process::{Command, Output, Stdio},
};

fn cli(args: &[&str], directory: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deka"))
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}
fn stdin(source: &str, check: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_deka"));
    command.args(["fmt", "--stdin"]);
    if check {
        command.arg("--check");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn file_check_never_writes_and_formatting_preserves_execution() {
    let dir = tempfile::tempdir().unwrap();
    let source = "import {echo} from \"io\"\nconst answer=40+2\necho(string(answer))\n";
    let file = dir.path().join("main.ds");
    fs::write(&file, source).unwrap();
    let before = cli(&["run", "main.ds"], dir.path());
    assert!(before.status.success(), "{before:?}");
    assert_eq!(before.stdout, b"42\n");
    let checked = cli(&["fmt", "main.ds", "--check"], dir.path());
    assert_eq!(checked.status.code(), Some(1), "{checked:?}");
    assert!(String::from_utf8_lossy(&checked.stderr).contains("main.ds would be reformatted"));
    assert_eq!(fs::read_to_string(&file).unwrap(), source);
    let formatted = cli(&["fmt", "main.ds"], dir.path());
    assert!(formatted.status.success(), "{formatted:?}");
    let once = fs::read_to_string(&file).unwrap();
    assert_ne!(once, source);
    assert!(once.contains("const answer = 40 + 2"), "{once}");
    let after = cli(&["run", "main.ds"], dir.path());
    assert!(after.status.success(), "{after:?}");
    assert_eq!(after.stdout, before.stdout);
    assert!(
        cli(&["fmt", "main.ds", "--check"], dir.path())
            .status
            .success()
    );
    assert!(cli(&["fmt", "main.ds"], dir.path()).status.success());
    assert_eq!(fs::read_to_string(file).unwrap(), once);
}
#[test]
fn directory_visits_ds_and_dsx_reports_every_change_and_ignores_other_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    let paths = [dir.path().join("a.ds"), dir.path().join("nested/b.dsx")];
    let sources = [
        "const x=1\n",
        "fn App(){return <view><Card title=\"Hi\"><slot /></Card></view>}\n",
    ];
    for (path, source) in paths.iter().zip(sources) {
        fs::write(path, source).unwrap();
    }
    fs::write(dir.path().join("keep.js"), "const x=1").unwrap();
    let checked = cli(&["fmt", ".", "--check"], dir.path());
    assert_eq!(checked.status.code(), Some(1), "{checked:?}");
    let errors = String::from_utf8_lossy(&checked.stderr);
    assert!(
        errors.contains("a.ds would be reformatted")
            && errors.contains("b.dsx would be reformatted"),
        "{errors}"
    );
    for (path, source) in paths.iter().zip(sources) {
        assert_eq!(fs::read_to_string(path).unwrap(), source);
    }
    let formatted = cli(&["fmt", "."], dir.path());
    assert!(formatted.status.success(), "{formatted:?}");
    assert!(
        String::from_utf8_lossy(&formatted.stdout)
            .contains("visited 2 DekaScript file(s) under . (2 reformatted)")
    );
    for (path, source) in paths.iter().zip(sources) {
        assert_ne!(fs::read_to_string(path).unwrap(), source);
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("keep.js")).unwrap(),
        "const x=1"
    );
    assert!(cli(&["fmt", ".", "--check"], dir.path()).status.success());
}
#[test]
fn stdin_formats_and_check_returns_status_without_output() {
    let source = "const x=1\n";
    let output = stdin(source, false);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, b"const x = 1\n");
    let check = stdin(source, true);
    assert_eq!(check.status.code(), Some(1));
    assert!(check.stdout.is_empty() && check.stderr.is_empty());
    let check = stdin("const x = 1\n", true);
    assert!(check.status.success());
    assert!(check.stdout.is_empty() && check.stderr.is_empty());
}
#[test]
fn incomplete_source_is_unchanged_and_usage_errors_are_not_success() {
    let incomplete = "fn App(\n";
    assert_eq!(stdin(incomplete, false).stdout, incomplete.as_bytes());
    assert!(stdin(incomplete, true).status.success());
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["fmt"][..],
        &["fmt", "a.ds", "b.ds"],
        &["fmt", "--stdin", "a.ds"],
        &["fmt", "--lang", "js"],
        &["fmt", "--entry", "main", "a.ds"],
    ] {
        assert_eq!(cli(args, dir.path()).status.code(), Some(2), "{args:?}");
    }
    assert_eq!(
        cli(&["fmt", "missing.ds"], dir.path()).status.code(),
        Some(1)
    );
    let help = cli(&["fmt", "--help"], dir.path());
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(
        help.contains("--check") && help.contains("--stdin") && !help.contains("--entry"),
        "{help}"
    );
}

#[test]
fn formatter_flags_require_the_formatter_command() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["run", "--stdin"][..],
        &["check", "--check"],
        &["missing.ds", "--check"],
        &["--stdin"],
    ] {
        let output = cli(args, dir.path());
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("requires fmt"));
    }
}

#[cfg(unix)]
fn bounded_directory_cli(args: &[&str], directory: &std::path::Path) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deka"))
        .args(args)
        .current_dir(directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match child.try_wait().unwrap() {
            Some(_) => return child.wait_with_output().unwrap(),
            None if std::time::Instant::now() >= deadline => {
                // Only terminate the child this test started, then reap it.
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!("directory formatting did not terminate: {output:?}");
            }
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
}

#[test]
#[cfg(unix)]
fn directory_format_preserves_vendor_build_and_symlink_targets() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("app.ds"), "const answer=7\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/widget.dsx"), "const widget=8\n").unwrap();
    let vendor_source = "const vendor=1\n";
    let folders = [
        "node_modules",
        "ds_modules",
        ".git",
        ".target",
        "target",
        "dist",
    ];
    let mut untouched = Vec::new();
    for parent in [dir.path().to_path_buf(), dir.path().join("src")] {
        for folder in folders {
            let path = parent.join(folder).join("pkg");
            fs::create_dir_all(&path).unwrap();
            let source = path.join("vendor.ds");
            fs::write(&source, vendor_source).unwrap();
            untouched.push(source);
        }
    }
    let linked_file = outside.path().join("external.ds");
    fs::write(&linked_file, vendor_source).unwrap();
    symlink(&linked_file, dir.path().join("linked.ds")).unwrap();
    fs::create_dir(outside.path().join("package")).unwrap();
    let linked_folder_file = outside.path().join("package/external.dsx");
    fs::write(&linked_folder_file, vendor_source).unwrap();
    symlink(
        outside.path().join("package"),
        dir.path().join("linked-package"),
    )
    .unwrap();
    untouched.extend([linked_file, linked_folder_file]);

    let output = bounded_directory_cli(&["fmt", "."], dir.path());
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("app.ds")).unwrap(),
        "const answer = 7\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("src/widget.dsx")).unwrap(),
        "const widget = 8\n"
    );
    for path in untouched {
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            vendor_source,
            "recursive formatting modified excluded source {}",
            path.display()
        );
    }
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("visited 2 DekaScript file(s) under . (2 reformatted)"),
        "{output:?}"
    );
    let output = bounded_directory_cli(&["fmt", ".", "--check"], dir.path());
    assert!(output.status.success(), "{output:?}");
}

#[test]
#[cfg(unix)]
fn directory_format_terminates_at_a_symlink_cycle() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    fs::write(dir.path().join("app.ds"), "const answer=7\n").unwrap();
    symlink(dir.path(), dir.path().join("nested/back-to-root")).unwrap();
    let output = bounded_directory_cli(&["fmt", "."], dir.path());
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("app.ds")).unwrap(),
        "const answer = 7\n"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("visited 1 DekaScript file(s) under . (1 reformatted)"),
        "{output:?}"
    );
}
