//! CLI coverage for `deka run` entry resolution.
//!
//! Isolate execution still goes through `dsc`, so preference/path cases use a
//! stub `dsc` that records the entry argument instead of requiring a full run.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

const EMPTY_DEKA_LOCK: &str = r#"{"lockfileVersion":1,"packages":{}}"#;

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdir");
    }
    fs::write(path, body).expect("write fixture");
}

fn project() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("tempdir");
    write(project.path(), "deka.json", "{}\n");
    write(project.path(), "deka.lock", EMPTY_DEKA_LOCK);
    project
}

fn run_in(dir: &Path, args: &[&str], dsc: Option<&Path>) -> (i32, String) {
    let mut command = Command::new(cli_bin());
    command.args(args).current_dir(dir);
    match dsc {
        Some(path) => {
            command.env("DEKA_DSC", path);
            command.env_remove("DEKA_NO_DSC");
        }
        None => {
            command.env_remove("DEKA_DSC");
            command.env("PATH", "/usr/bin:/bin");
        }
    }
    let output = command.output().expect("run deka");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code().unwrap_or(1), combined)
}

/// Stub `dsc` that records argv and exits 1 so the CLI surfaces the path.
fn write_dsc_stub(root: &Path) -> PathBuf {
    let log = root.join("dsc-args.log");
    let stub = root.join("dsc");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\necho stub-dsc-ran >&2\nexit 1\n",
        log.display()
    );
    fs::write(&stub, script).expect("write dsc stub");
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();
    stub
}

fn read_dsc_args(root: &Path) -> String {
    fs::read_to_string(root.join("dsc-args.log")).unwrap_or_default()
}

#[test]
fn run_without_args_attempts_src_main_ds() {
    let project = project();
    write(project.path(), "src/main.ds", "export const ok = true\n");
    let stub = write_dsc_stub(project.path());

    let (code, text) = run_in(project.path(), &["run"], Some(&stub));
    assert_ne!(code, 0, "stub dsc should fail closed: {text}");
    let args = read_dsc_args(project.path());
    assert!(
        args.contains("src/main.ds") || text.contains("src/main.ds"),
        "expected src/main.ds resolution; dsc args={args:?} output={text}"
    );
}

#[test]
fn run_without_entry_lists_what_was_looked_for() {
    let project = project();

    let (code, text) = run_in(project.path(), &["run"], None);
    assert_ne!(code, 0, "empty project must not run: {text}");
    assert!(text.contains("looked for"), "{text}");
    assert!(text.contains("CLI") && text.contains(".ds"), "{text}");
    assert!(text.contains("deka.json \"entry\""), "{text}");
    assert!(text.contains("deka.json \"main\""), "{text}");
    assert!(text.contains("serve.entry"), "{text}");
    assert!(text.contains("app/"), "{text}");
    assert!(text.contains("api/"), "{text}");
    assert!(text.contains("src/"), "{text}");
}

#[test]
fn run_prefers_dist_js_over_src_main_ds() {
    let project = project();
    write(project.path(), "src/main.ds", "export const from = \"ds\"\n");
    write(
        project.path(),
        "dist/server/src/main.js",
        "console.log(\"from-dist\")\n",
    );
    let stub = write_dsc_stub(project.path());

    let (code, text) = run_in(project.path(), &["run"], Some(&stub));
    let args = read_dsc_args(project.path());
    assert!(
        text.contains("from-dist") || args.contains("dist/server/src/main.js"),
        "expected dist JS to run (or be the compile input); dsc args={args:?} output={text}"
    );
    assert!(
        !args.contains("src/main.ds"),
        "must not compile src/main.ds when dist JS exists; dsc args={args:?}"
    );
    assert_eq!(
        code, 0,
        "preferred dist JS should run without dsc: {text}"
    );
}

#[test]
fn run_cli_ds_arg_wins_over_deka_json_main() {
    let project = project();
    write(project.path(), "deka.json", r#"{"main":"from-main.ds"}"#);
    write(project.path(), "from-main.ds", "export const main = true\n");
    write(project.path(), "cli.ds", "export const cli = true\n");
    let stub = write_dsc_stub(project.path());

    let (code, text) = run_in(project.path(), &["run", "cli.ds"], Some(&stub));
    assert_ne!(code, 0, "stub dsc should fail closed: {text}");
    let args = read_dsc_args(project.path());
    assert!(
        args.contains("cli.ds"),
        "CLI file must win; dsc args={args:?} output={text}"
    );
    assert!(
        !args.contains("from-main.ds"),
        "manifest main must lose to CLI file; dsc args={args:?}"
    );
}

#[test]
fn run_missing_cli_file_does_not_fall_through() {
    let project = project();
    write(project.path(), "deka.json", r#"{"main":"from-main.ds"}"#);
    write(project.path(), "from-main.ds", "export const main = true\n");
    write(project.path(), "cli.ds", "export const cli = true\n");

    let (code, text) = run_in(project.path(), &["run", "missing-cli.ds"], None);
    assert_ne!(code, 0, "missing CLI file must not fall through: {text}");
    assert!(text.contains("entry file not found"), "{text}");
    assert!(text.contains("missing-cli.ds"), "{text}");
}

#[test]
fn run_app_directory_without_module_points_at_serve() {
    let project = project();
    write(project.path(), "app/layout.dsx", "export const layout = true\n");

    let (code, text) = run_in(project.path(), &["run"], None);
    assert_ne!(code, 0, "directory app/ is not a run module: {text}");
    assert!(
        text.contains("deka serve") && text.contains("directory"),
        "{text}"
    );
}
