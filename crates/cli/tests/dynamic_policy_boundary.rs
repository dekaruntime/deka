use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

const EMPTY_DEKA_LOCK: &str = r#"{"lockfileVersion":1,"packages":{}}"#;

// ---------------------------------------------------------------------------
// deka#425: `security.allow.dynamic` enforced on the ESM path.
//
// The inline-handler check covers the platform path only; `deka run` and
// `deka serve` leave `handler_code` empty, so nothing gated them and the CLI
// printed `dynamic=false` while `eval` worked. These drive the real binary,
// because the previous check passed its unit tests on a path that never ran
// (deka#430).
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

const DENY: &str = r#"{"name":"gate","security":{"prompt":false}}"#;
const ALLOW: &str = r#"{"name":"gate","security":{"allow":{"dynamic":true},"prompt":false}}"#;

fn eval_source(spelling: &str) -> String {
    format!("export fn main() {{\n  let r = unsafe {{ {spelling} }}\n}}\n\nmain()\n")
}

#[test]
fn dynamic_denied_blocks_eval_before_execution() {
    let dir = project(DENY, &eval_source("eval(\"1 + 1\")"));
    let out = run(dir.path());
    assert!(
        out.contains("eval is disabled by the security policy"),
        "eval must be rejected under the default policy: {out}"
    );
}

#[test]
fn dynamic_denied_blocks_the_member_spellings_too() {
    // These walked past the first matcher, which only handled bare identifiers.
    for spelling in [
        "globalThis.eval(\"1 + 1\")",
        "globalThis[\"eval\"](\"1 + 1\")",
        "(0, eval)(\"1 + 1\")",
        "new Function(\"return 7\")()",
        "globalThis.Function(\"return 7\")()",
    ] {
        let dir = project(DENY, &eval_source(spelling));
        let out = run(dir.path());
        assert!(
            out.contains("disabled by the security policy"),
            "{spelling} must be rejected: {out}"
        );
    }
}

#[test]
fn dynamic_allowed_is_an_explicit_opt_in() {
    let dir = project(ALLOW, &eval_source("eval(\"1 + 1\")"));
    let out = run(dir.path());
    assert!(
        !out.contains("disabled by the security policy"),
        "an explicit dynamic grant must let the same program run: {out}"
    );
}

#[test]
fn an_ordinary_program_is_not_falsely_rejected() {
    let dir = project(
        DENY,
        "export fn main() {\n  let n = 1 + 1\n  return n\n}\n\nmain()\n",
    );
    let out = run(dir.path());
    assert!(
        !out.contains("disabled by the security policy"),
        "code with no dynamic evaluation must run under the default policy: {out}"
    );
}

#[test]
fn host_process_access_is_not_governed_by_this_policy() {
    // `process.cwd()` is a documented feature with a passing conformance
    // fixture. Whether it should need a capability is deka#378 / deka#435 --
    // but it is not dynamic code, and denying it here broke that fixture.
    let dir = project(
        DENY,
        "export fn main() {\n  let cwd = unsafe { process.cwd() }\n}\n\nmain()\n",
    );
    let out = run(dir.path());
    assert!(
        !out.contains("disabled by the security policy"),
        "ambient host access must not be rejected by the dynamic policy: {out}"
    );
}
