use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(cli_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run deka")
}

#[test]
fn link_and_unlink_preserve_package_and_use_project_local_state() {
    let consumer = tempfile::tempdir().unwrap();
    let package = tempfile::tempdir().unwrap();
    fs::write(consumer.path().join("deka.json"), "{}\n").unwrap();
    fs::write(
        package.path().join("deka.json"),
        r#"{"name":"@deka/local","version":"0.1.0"}
"#,
    )
    .unwrap();
    fs::write(package.path().join("index.ds"), "export const value = 1;\n").unwrap();

    let package_path = package.path().to_str().unwrap();
    let linked = run(consumer.path(), &["link", package_path]);
    assert!(
        linked.status.success(),
        "link failed: {}{}",
        String::from_utf8_lossy(&linked.stdout),
        String::from_utf8_lossy(&linked.stderr)
    );

    let links: Value = serde_json::from_str(
        &fs::read_to_string(consumer.path().join(".deka/links.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(links["version"], 1);
    assert_eq!(
        links["packages"]["@deka/local"]["path"],
        package.path().canonicalize().unwrap().to_str().unwrap()
    );

    let unlinked = run(consumer.path(), &["unlink", "@deka/local"]);
    assert!(
        unlinked.status.success(),
        "unlink failed: {}{}",
        String::from_utf8_lossy(&unlinked.stdout),
        String::from_utf8_lossy(&unlinked.stderr)
    );
    let links: Value = serde_json::from_str(
        &fs::read_to_string(consumer.path().join(".deka/links.json")).unwrap(),
    )
    .unwrap();
    assert!(links["packages"].as_object().unwrap().is_empty());
    assert!(package.path().join("index.ds").is_file());
}

#[test]
fn help_documents_link_commands() {
    let output = Command::new(cli_bin())
        .arg("--help")
        .output()
        .expect("run deka help");
    assert!(output.status.success());
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(help.contains("link"), "{help}");
    assert!(help.contains("unlink"), "{help}");
}
