use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn gild_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gild")
}

#[test]
fn version_prints() {
    let output = Command::new(gild_bin()).arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("gild"));
}

#[test]
fn agent_ls_reads_config_file() {
    let dir = std::env::temp_dir().join(format!(
        "gild-cli-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let config = dir.join("agents.toml");
    fs::write(
        &config,
        r#"
[agents.agent-khalid]
port = 9434
name = "Khalid"
sandbox = "gild"
"#,
    )
    .unwrap();

    let output = Command::new(gild_bin())
        .args(["agent", "ls"])
        .env("GILD_AGENTS_CONFIG", &config)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("agent-khalid"));
    assert!(stdout.contains("9434"));
    assert!(stdout.contains("Khalid"));
}
