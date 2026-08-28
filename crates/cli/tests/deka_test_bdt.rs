use std::fs;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

const EMPTY_DEKA_LOCK: &str = r#"{"lockfileVersion":1,"packages":{}}"#;

fn write_project(source: &str) -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project");
    fs::write(project.path().join("deka.json"), "{}\n").expect("manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("lock");
    fs::create_dir_all(project.path().join("tests")).expect("tests dir");
    fs::write(project.path().join("tests/math.test.ds"), source).expect("test file");
    project
}

#[test]
#[ignore = "blocked on v2 top-level let/const scope and @deka/test library update (see dekaruntime/deka#330)"]
fn deka_test_runs_ds_tests() {
    let project = write_project(
        r#"
import { describe, it, toBe } from "@deka/test"

fn add(a: number, b: number) number {
  return a + b
}

describe("math", fn() {
  it("adds", fn() {
    add(1, 2) |> toBe(3)
  })
})
"#,
    );

    let output = Command::new(cli_bin())
        .arg("test")
        .current_dir(project.path())
        .output()
        .expect("deka test");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "deka test failed: {combined}"
    );
    assert!(
        combined.contains("[pass] math > adds"),
        "missing pass line: {combined}"
    );
    assert!(
        combined.contains("[test] total=1 passed=1 failed=0"),
        "missing summary: {combined}"
    );
}

#[test]
#[ignore = "blocked on v2 top-level let/const scope and @deka/test library update (see dekaruntime/deka#330)"]
fn deka_test_unwraps_ok() {
    let project = write_project(
        r#"
import { describe, it, toBe, toBeOk } from "@deka/test"

fn parse(n: number) Result<number, string> {
  return Ok(n)
}

describe("result", fn() {
  it("unwraps Ok", fn() {
    parse(3) |> toBeOk |> toBe(3)
  })
})
"#,
    );

    let output = Command::new(cli_bin())
        .arg("test")
        .current_dir(project.path())
        .output()
        .expect("deka test");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "deka test failed: {combined}");
    assert!(
        combined.contains("[pass] result > unwraps Ok"),
        "missing pass line: {combined}"
    );
}

#[test]
#[ignore = "blocked on v2 top-level let/const scope and @deka/test library update (see dekaruntime/deka#330)"]
fn deka_test_fails_on_assertion() {
    let project = write_project(
        r#"
import { describe, it, toBe } from "@deka/test"

describe("math", fn() {
  it("adds", fn() {
    1 |> toBe(2)
  })
})
"#,
    );

    let output = Command::new(cli_bin())
        .arg("test")
        .current_dir(project.path())
        .output()
        .expect("deka test");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "expected failure: {combined}"
    );
    assert!(
        combined.contains("[fail] math > adds"),
        "missing fail line: {combined}"
    );
}

#[test]
#[ignore = "blocked on v2 top-level let/const scope and @deka/test library update (see dekaruntime/deka#330)"]
fn deka_test_imports_application_source() {
    let project = tempfile::tempdir().expect("project");
    fs::write(project.path().join("deka.json"), "{}\n").expect("manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("lock");
    fs::create_dir_all(project.path().join("src")).expect("src dir");
    fs::create_dir_all(project.path().join("tests")).expect("tests dir");
    fs::write(
        project.path().join("src/math.ds"),
        "export fn add(a: number, b: number) number {\n  return a + b\n}\n",
    )
    .expect("source");
    fs::write(
        project.path().join("tests/math.test.ds"),
        r#"
import { describe, it, toBe } from "@deka/test"
import { add } from "../src/math.ds"

describe("math", fn() {
  it("adds", fn() {
    add(1, 2) |> toBe(3)
  })
})
"#,
    )
    .expect("test file");

    let output = Command::new(cli_bin())
        .arg("test")
        .current_dir(project.path())
        .output()
        .expect("deka test");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "deka test failed: {combined}");
    assert!(
        combined.contains("[pass] math > adds"),
        "missing pass line: {combined}"
    );
}

#[test]
#[ignore = "blocked on v2 top-level let/const scope and @deka/test library update (see dekaruntime/deka#330)"]
fn deka_test_name_pattern_filters_cases() {
    let project = write_project(
        r#"
import { describe, it, toBe } from "@deka/test"

describe("math", fn() {
  it("adds", fn() {
    1 + 2 |> toBe(3)
  })
  it("subtracts", fn() {
    5 - 2 |> toBe(3)
  })
})
"#,
    );

    let output = Command::new(cli_bin())
        .args(["test", "-t", "adds"])
        .current_dir(project.path())
        .output()
        .expect("deka test");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "deka test failed: {combined}");
    assert!(
        combined.contains("[pass] math > adds"),
        "missing filtered pass: {combined}"
    );
    assert!(
        !combined.contains("subtracts"),
        "unfiltered case ran: {combined}"
    );
    assert!(
        combined.contains("[test] total=1 passed=1 failed=0"),
        "missing summary: {combined}"
    );
}

#[test]
fn deka_test_ignores_hats_fixtures() {
    let project = tempfile::tempdir().expect("project");
    fs::write(project.path().join("deka.json"), "{}\n").expect("manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("lock");
    let hats = project
        .path()
        .join("tests/testsuite/functions/pipe_operator");
    fs::create_dir_all(&hats).expect("hats dir");
    fs::write(hats.join("pipe_operator.pass.ds"), "console.log(1)\n").expect("hats file");

    let output = Command::new(cli_bin())
        .arg("test")
        .current_dir(project.path())
        .output()
        .expect("deka test");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "deka test failed: {combined}");
    assert!(
        combined.contains("no deka tests found"),
        "should skip hats fixtures: {combined}"
    );
}
