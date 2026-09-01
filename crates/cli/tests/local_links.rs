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

/// End-to-end through the CLI binary: a linked package must actually resolve
/// and run with nothing installed. This is the case the resolver unit tests
/// cannot cover -- three separate gates sit in front of resolution (module
/// graph validation in `deka_host`, the project gate in `runtime_core`, and
/// the ESM loader in `pool`), and every one of them has to know about links.
/// Asserting on stdout is what proves the whole path, not just the bookkeeping.
#[test]
fn linked_package_resolves_and_runs_with_nothing_installed() {
    let consumer = tempfile::tempdir().unwrap();
    let package = tempfile::tempdir().unwrap();

    fs::write(
        package.path().join("deka.json"),
        r#"{"name":"@deka/greeter","version":"0.1.0","main":"index.ds"}
"#,
    )
    .unwrap();
    fs::write(
        package.path().join("index.ds"),
        "export fn greet(name: string) string {\n  return \"hello \" + name\n}\n",
    )
    .unwrap();

    fs::write(
        consumer.path().join("deka.json"),
        r#"{"name":"consumer","version":"0.0.1","main":"main.ds","dependencies":{"@deka/greeter":"0.1.0"}}
"#,
    )
    .unwrap();
    fs::write(
        consumer.path().join("deka.lock"),
        "{\n  \"version\": 1,\n  \"packages\": {}\n}\n",
    )
    .unwrap();
    fs::write(
        consumer.path().join("main.ds"),
        "import { greet } from \"@deka/greeter\"\nunsafe<void> { console.log(greet(\"link\")) }\n",
    )
    .unwrap();

    // Nothing is installed: no ds_modules anywhere.
    assert!(!consumer.path().join("ds_modules").exists());

    let linked = run(consumer.path(), &["link", package.path().to_str().unwrap()]);
    assert!(linked.status.success(), "link failed");

    let ran = run(consumer.path(), &["run", "main.ds"]);
    let stdout = String::from_utf8_lossy(&ran.stdout);
    let stderr = String::from_utf8_lossy(&ran.stderr);
    assert!(
        ran.status.success(),
        "run failed with a link present:\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("hello link"),
        "linked package did not execute; stdout: {stdout}\nstderr: {stderr}"
    );

    // Unlinking must put the failure back: the package was never installed.
    let unlinked = run(consumer.path(), &["unlink", "@deka/greeter"]);
    assert!(unlinked.status.success(), "unlink failed");
    let after = run(consumer.path(), &["run", "main.ds"]);
    assert!(
        !after.status.success(),
        "run succeeded after unlink, so the earlier pass did not prove the link was used"
    );
}

/// A link whose target has been moved or deleted must fail loudly rather than
/// falling through to whatever is installed. Silent fallback is how a stale
/// link ends up looking like it worked.
#[test]
fn stale_link_fails_closed_instead_of_falling_back() {
    let consumer = tempfile::tempdir().unwrap();
    let package = tempfile::tempdir().unwrap();

    fs::write(
        package.path().join("deka.json"),
        r#"{"name":"@deka/ghost","version":"0.1.0","main":"index.ds"}
"#,
    )
    .unwrap();
    fs::write(package.path().join("index.ds"), "export const value = 1;\n").unwrap();

    fs::write(
        consumer.path().join("deka.json"),
        r#"{"name":"consumer","version":"0.0.1","main":"main.ds","dependencies":{"@deka/ghost":"0.1.0"}}
"#,
    )
    .unwrap();
    fs::write(
        consumer.path().join("deka.lock"),
        "{\n  \"version\": 1,\n  \"packages\": {}\n}\n",
    )
    .unwrap();
    fs::write(
        consumer.path().join("main.ds"),
        "import { value } from \"@deka/ghost\"\n",
    )
    .unwrap();

    let linked = run(consumer.path(), &["link", package.path().to_str().unwrap()]);
    assert!(linked.status.success(), "link failed");

    // The manifest now names a target that no longer exists.
    package.close().unwrap();

    let ran = run(consumer.path(), &["run", "main.ds"]);
    assert!(!ran.status.success(), "stale link did not fail");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(
        combined.contains("link is unusable"),
        "stale link produced the wrong error: {combined}"
    );
}
