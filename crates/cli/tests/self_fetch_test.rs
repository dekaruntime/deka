//! End-to-end coverage for `deka self test` (deka#836): a real CLI
//! subprocess drives the fetch-less path — checkout detection, toolchain
//! pairing under `<checkout>/target/release/`, runner invocation via bun —
//! against a synthetic content checkout. The synthetic checkout stands in
//! for dekaruntime/testsuite and dekaruntime/tour content (pinned external
//! corpora, not vendored here); its runners assert the pairing contract the
//! real runners rely on. The conformance semantics themselves belong to the
//! content repos and the in-tree runner tests
//! (runner_modes_agree.rs, package_cache_key.rs).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn require_bun() -> bool {
    Command::new("bun")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// A stub `deka` CLI: records nothing, exits successfully. Stands in for the
/// paired toolchain — the real gate's `deka run` needs a full compiler
/// pairing, which is the content repos' own CI concern.
fn write_stub_deka(dir: &Path) -> PathBuf {
    let stub = dir.join("stub-deka");
    fs::write(&stub, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();
    stub
}

/// A stub `dsc` compiler, paired via `--dsc`.
fn write_stub_dsc(dir: &Path) -> PathBuf {
    let stub = dir.join("stub-dsc");
    fs::write(&stub, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();
    stub
}

/// Synthetic testsuite checkout: marker file, a fixture, and a runner that
/// verifies the pairing `self test` materialized and then grades the
/// fixture by execing the paired cli, like the real corpus runner does.
fn write_fake_testsuite(root: &Path) {
    let corpus = root.join("testsuite").join("corpus");
    fs::create_dir_all(corpus.join("basics").join("ok")).unwrap();
    fs::write(corpus.join("expected-failures.txt"), "# none\n").unwrap();
    fs::write(
        corpus.join("basics").join("ok").join("ok.pass.ds"),
        "export const ok = 1\n",
    )
    .unwrap();
    fs::write(
        corpus.join("run.mjs"),
        r#"// Synthetic stand-in for dekaruntime/testsuite corpus/run.mjs (deka#836
// tests). Asserts the pairing contract, then grades one fixture through the
// paired cli exactly like the real runner does (`deka run`).
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const binDir = join(import.meta.dir, "..", "target", "release");
const cli = join(binDir, "cli");
if (!existsSync(cli)) {
  console.error("synthetic runner: paired cli missing at " + cli);
  process.exit(2);
}
const filter = process.argv.includes("--filter");
if (filter && !process.argv.includes("ok")) {
  console.error("synthetic runner: --filter value was not forwarded");
  process.exit(2);
}
const run = spawnSync(cli, ["run", "./fixture.pass.ds"], { encoding: "utf-8" });
if (run.status !== 0) {
  console.error("synthetic runner: paired cli run failed: " + run.stderr);
  process.exit(1);
}
const lesson = readFileSync(join(import.meta.dir, "basics", "ok", "ok.pass.ds"), "utf8");
if (!lesson.includes("export const ok")) {
  console.error("synthetic runner: fixture unreadable");
  process.exit(1);
}
console.log("Passed: 1 | Failed: 0 | Total: 1");
process.exit(0);
"#,
    )
    .unwrap();
}

/// Synthetic tour checkout: manifest + a runner with the same contract as
/// dekaruntime/tour's run.mjs (binary lookup under target/release, io
/// install before lessons).
fn write_fake_tour(root: &Path) {
    let tour = root.join("tour").join("tests").join("tour");
    fs::create_dir_all(&tour).unwrap();
    fs::write(
        tour.join("manifest.json"),
        r#"[{ "id": "hello", "title": "hello", "expectCompile": true }]"#,
    )
    .unwrap();
    fs::write(tour.join("hello.ds"), "export const hello = 1\n").unwrap();
    fs::write(
        tour.join("run.mjs"),
        r#"// Synthetic stand-in for dekaruntime/tour tests/tour/run.mjs (deka#836
// tests). Asserts the pairing contract the real runner's findCliBinary
// relies on: an executable cli under <checkout>/target/release.
import { existsSync } from "node:fs";
import { join } from "node:path";

const candidates = [
  join(import.meta.dir, "..", "..", "target", "release", "dsc"),
  join(import.meta.dir, "..", "..", "target", "release", "cli"),
];
const binary = candidates.find((candidate) => {
  try {
    return existsSync(candidate);
  } catch {
    return false;
  }
});
if (!binary) {
  console.error("synthetic runner: no compiler binary under target/release");
  process.exit(2);
}
const filter = process.argv.includes("--filter");
if (filter && !process.argv.includes("hello")) {
  console.error("synthetic runner: --filter value was not forwarded");
  process.exit(2);
}
if (!existsSync(join(import.meta.dir, "manifest.json"))) {
  console.error("synthetic runner: manifest missing");
  process.exit(1);
}
console.log("Passed: 1 | Failed: 1 | Total: 1");
process.exit(0);
"#,
    )
    .unwrap();
}

fn run_self_test(root: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("self")
        .arg("test")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run deka self test");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

#[test]
fn self_test_suite_runs_fetched_checkout_against_paired_deka() {
    if !require_bun() {
        eprintln!("skipping: bun is not installed");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    write_fake_testsuite(root.path());
    let stub = write_stub_deka(root.path());

    let (ok, output) = run_self_test(
        root.path(),
        &["suite", "--deka", stub.to_str().unwrap(), "--filter", "ok"],
    );
    assert!(ok, "self test suite failed: {}", output);

    // The pairing must have been materialized inside the checkout.
    let paired = root.path().join("testsuite/target/release/cli");
    assert!(paired.is_file(), "paired cli missing: {}", paired.display());
    assert_eq!(fs::read(&paired).unwrap(), b"#!/bin/sh\nexit 0\n");
}

#[test]
fn self_test_suite_pairs_local_dsc_wrapper() {
    if !require_bun() {
        eprintln!("skipping: bun is not installed");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    write_fake_testsuite(root.path());
    let stub = write_stub_deka(root.path());
    let dsc = write_stub_dsc(root.path());

    let (ok, output) = run_self_test(
        root.path(),
        &[
            "testsuite",
            "--deka",
            stub.to_str().unwrap(),
            "--dsc",
            dsc.to_str().unwrap(),
        ],
    );
    assert!(ok, "self test suite --dsc failed: {}", output);

    let wrapper = root.path().join("testsuite/target/release/dsc");
    let text = fs::read_to_string(&wrapper).expect("dsc wrapper materialized");
    assert!(text.contains("add|install"), "wrapper forwards add: {}", text);
    assert!(text.contains(dsc.to_str().unwrap()));
}

#[test]
fn self_test_tour_runs_fetched_checkout() {
    if !require_bun() {
        eprintln!("skipping: bun is not installed");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    write_fake_tour(root.path());
    let stub = write_stub_deka(root.path());

    let (ok, output) = run_self_test(
        root.path(),
        &["tour", "--deka", stub.to_str().unwrap(), "--filter", "hello"],
    );
    assert!(ok, "self test tour failed: {}", output);
    assert!(root.path().join("tour/target/release/cli").is_file());
}

#[test]
fn self_test_requires_fetched_checkout() {
    let root = tempfile::tempdir().unwrap();
    let (ok, output) = run_self_test(root.path(), &["suite"]);
    assert!(!ok, "self test suite must fail without a checkout");
    assert!(
        output.contains("run `deka self fetch testsuite` first"),
        "unexpected output: {}",
        output
    );
}

#[test]
fn self_test_propagates_runner_failure_exit_code() {
    if !require_bun() {
        eprintln!("skipping: bun is not installed");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    // Marker present but the runner itself fails: the exit code must be the
    // runner's, not a generic 1 from the cli wrapper.
    let corpus = root.path().join("testsuite/corpus");
    fs::create_dir_all(&corpus).unwrap();
    fs::write(corpus.join("expected-failures.txt"), "").unwrap();
    fs::write(
        corpus.join("run.mjs"),
        "console.error('synthetic gate failure');\nprocess.exit(1);\n",
    )
    .unwrap();
    let stub = write_stub_deka(root.path());

    let output = Command::new(cli_bin())
        .arg("self")
        .arg("test")
        .arg("suite")
        .arg("--deka")
        .arg(&stub)
        .current_dir(root.path())
        .output()
        .expect("run deka self test");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("synthetic gate failure"));
}

#[test]
fn self_test_suite_accepts_checkout_directory_override() {
    if !require_bun() {
        eprintln!("skipping: bun is not installed");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    write_fake_testsuite(root.path());
    // A checkout-shaped --deka: target/release/cli inside a directory.
    let checkout = tempfile::tempdir().unwrap();
    let built = checkout.path().join("target/release/cli");
    fs::create_dir_all(built.parent().unwrap()).unwrap();
    fs::write(&built, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = fs::metadata(&built).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&built, perms).unwrap();

    let (ok, output) = run_self_test(root.path(), &["suite", "--deka", checkout.path().to_str().unwrap()]);
    assert!(ok, "self test suite with checkout --deka failed: {}", output);
    let paired = root.path().join("testsuite/target/release/cli");
    assert_eq!(fs::read(&paired).unwrap(), b"#!/bin/sh\nexit 0\n");
}
