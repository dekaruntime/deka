use std::{path::PathBuf, process::Command};

fn gild_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gild")
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn command_with_agent_fixture() -> Command {
    let fixture = fixture_path("agent-ports-mini.json");
    let mut command = Command::new(gild_bin());
    command
        .env("AGENT_PORTS_FILE", &fixture)
        .env("GILD_AGENT_PORTS_CONFIG", fixture);
    command
}

#[test]
fn dispatch_with_missing_agent_returns_clear_error() {
    let output = command_with_agent_fixture()
        .args(["dispatch", "agent-doesnt-exist", "task"])
        .output()
        .expect("spawn gild");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("unknown agent"),
        "stderr was: {stderr}"
    );
}

#[test]
fn dispatch_with_invalid_runtime_returns_clear_error() {
    let output = command_with_agent_fixture()
        .args(["dispatch", "agent-test1", "task", "--runtime", "nonsense"])
        .output()
        .expect("spawn gild");

    assert!(!output.status.success());
}

#[test]
fn agent_ls_prints_configured_agents() {
    let output = command_with_agent_fixture()
        .args(["agent", "ls"])
        .output()
        .expect("spawn gild");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("agent-test1"));
    assert!(stdout.contains("agent-test2"));
}
