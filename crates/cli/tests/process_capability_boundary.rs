use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

const EMPTY_DEKA_LOCK: &str = r#"{"lockfileVersion":1,"packages":{}}"#;

// ---------------------------------------------------------------------------
// deka#378: `process` sits behind TWO independent gates and needs both.
//
//   1. the `env` capability, granted in deka.json
//   2. an `unsafe { }` block -- it is not a DekaScript name
//
// Removing the grant must disable the unsafe path too: the global is never
// installed, so the call fails rather than the capability being advisory.
// ---------------------------------------------------------------------------

fn project(manifest: &str, source: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create project");
    fs::write(dir.path().join("deka.json"), manifest).expect("manifest");
    fs::write(dir.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("lockfile");
    fs::write(dir.path().join("main.ds"), source).expect("entry");
    dir
}

fn run(project: &Path) -> String {
    let output = Command::new(cli_bin())
        .args(["run", "main.ds"])
        .current_dir(project)
        .output()
        .expect("run through the CLI");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const NO_ENV: &str = r#"{"name":"p","security":{"prompt":false}}"#;
const ENV_LIST: &str = r#"{"name":"p","security":{"allow":{"env":["HOME"]},"prompt":false}}"#;
const ENV_ALL: &str = r#"{"name":"p","security":{"allow":{"env":true},"prompt":false}}"#;
const ENV_DENIED: &str =
    r#"{"name":"p","security":{"allow":{"env":true},"deny":{"env":true},"prompt":false}}"#;

/// Reports `reached` or `denied` so the two outcomes are distinguishable.
const PROBE: &str = r#"export fn main() {
  let cwd = unsafe { process.cwd() }
  let label = match (cwd) {
    Ok(v) => "reached",
    Err(e) => "denied"
  }
  let printed = unsafe { console.log(label) }
  match (printed) {
    Ok(v) => v,
    Err(e) => e
  };
}

main()
"#;

#[test]
fn bare_process_is_not_a_dekascript_name() {
    let dir = project(
        ENV_ALL,
        "export fn main() {\n  const here = process.cwd()\n}\n\nmain()\n",
    );
    let out = run(dir.path());
    assert!(
        out.contains("unknown identifier `process`"),
        "plain code must not reach the host process object even with the grant: {out}"
    );
}

#[test]
fn unsafe_without_the_env_capability_fails() {
    let dir = project(NO_ENV, PROBE);
    let out = run(dir.path());
    assert!(
        out.contains("denied"),
        "the global must be absent without an env grant: {out}"
    );
}

#[test]
fn unsafe_with_an_env_grant_reaches_it() {
    for manifest in [ENV_LIST, ENV_ALL] {
        let dir = project(manifest, PROBE);
        let out = run(dir.path());
        assert!(
            out.contains("reached"),
            "an env grant of any shape must install the global: {out}"
        );
    }
}

#[test]
fn deny_beats_allow() {
    let dir = project(ENV_DENIED, PROBE);
    let out = run(dir.path());
    assert!(
        out.contains("denied"),
        "an explicit env deny must win over the allow: {out}"
    );
}
