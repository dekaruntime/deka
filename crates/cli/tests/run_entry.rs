use std::fs;
use std::path::Path;
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

fn run_in(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new(cli_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run deka");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

#[test]
fn run_without_entry_lists_what_was_looked_for() {
    let project = tempfile::tempdir().expect("tempdir");
    write(project.path(), "deka.json", "{}\n");
    write(project.path(), "deka.lock", EMPTY_DEKA_LOCK);

    let (ok, text) = run_in(project.path(), &["run"]);
    assert!(!ok, "empty project must not run: {text}");
    assert!(text.contains("looked for"), "{text}");
    assert!(text.contains("deka.json \"entry\""), "{text}");
    assert!(text.contains("deka.json \"main\""), "{text}");
    assert!(text.contains("serve.entry"), "{text}");
    assert!(text.contains("app/"), "{text}");
    assert!(text.contains("api/"), "{text}");
    assert!(text.contains("src/"), "{text}");
}

#[test]
fn run_app_directory_without_module_points_at_serve() {
    let project = tempfile::tempdir().expect("tempdir");
    write(project.path(), "deka.json", "{}\n");
    write(project.path(), "deka.lock", EMPTY_DEKA_LOCK);
    write(project.path(), "app/layout.dsx", "export const layout = true\n");

    let (ok, text) = run_in(project.path(), &["run"]);
    assert!(!ok, "directory app/ is not a run module: {text}");
    assert!(
        text.contains("deka serve") && text.contains("directory"),
        "{text}"
    );
}

#[test]
fn run_cli_file_wins_over_manifest_main() {
    let project = tempfile::tempdir().expect("tempdir");
    write(
        project.path(),
        "deka.json",
        r#"{"main":"from-main.ds"}"#,
    );
    write(project.path(), "deka.lock", EMPTY_DEKA_LOCK);
    write(project.path(), "from-main.ds", "export const main = true\n");
    write(project.path(), "cli.ds", "export const cli = true\n");

    let (ok, text) = run_in(project.path(), &["run", "missing-cli.ds"]);
    assert!(!ok, "missing CLI file must not fall through: {text}");
    assert!(text.contains("entry file not found"), "{text}");
    assert!(text.contains("missing-cli.ds"), "{text}");
}
