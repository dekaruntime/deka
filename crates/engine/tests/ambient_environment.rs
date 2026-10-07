//! deka#801 regression pin for `engine` config discovery: runtime config
//! comes from well-known files only; contradictory process environment must
//! not change behavior. Before this pin, `DEKA_RUNTIME_CONFIG` selected an
//! arbitrary config file from the ambient environment.
//!
//! Pattern follows the pool pin (deka#840, crates/pool/src/esm_loader/policy.rs):
//! the parent test spawns this test binary as a child and compares only the
//! child's proof lines — never the harness timing output.

use std::process::Command;

#[test]
fn runtime_config_ignores_contradictory_ambient_environment() {
    let test_bin = std::env::current_exe().expect("current test binary");
    let run = |deka_runtime_config: Option<&str>| {
        let mut command = Command::new(&test_bin);
        command.args([
            "--exact",
            "ambient_environment_child",
            "--ignored",
            "--nocapture",
        ]);
        if let Some(path) = deka_runtime_config {
            command.env("DEKA_RUNTIME_CONFIG", path);
        }
        let output = command.output().expect("run isolated child test");
        assert!(output.status.success(), "child failed: {output:?}");
        let stdout = String::from_utf8(output.stdout).expect("utf-8 child output");
        // Compare only the proof lines the child prints, never cargo's own
        // harness output -- that carries a wall-clock "finished in 0.01s"
        // line, so asserting on the raw stdout made this test fail whenever
        // the two child runs happened to land in different millisecond
        // buckets (deka#840).
        let proof: Vec<&str> = stdout
            .lines()
            .filter(|line| line.starts_with("ambient-proof:"))
            .collect();
        assert!(
            !proof.is_empty(),
            "child printed no ambient-proof lines: {stdout}"
        );
        proof.join("\n")
    };

    // A config file that WOULD enable the code cache if the ambient
    // environment were still a config channel. The child must ignore it.
    let offered = std::env::temp_dir().join(format!(
        "deka_engine_ambient_offered_{}.toml",
        std::process::id()
    ));
    std::fs::write(&offered, "[code_cache]\nenabled = true\n").expect("write offered config");

    assert_eq!(
        run(Some(offered.to_str().expect("utf-8 path"))),
        run(None)
    );

    std::fs::remove_file(&offered).ok();
}

#[test]
#[ignore]
fn ambient_environment_child() {
    // cwd is an empty dir: no cwd-relative config candidate can load. Any
    // DEKA_RUNTIME_CONFIG the parent sets points at a config that would turn
    // the code cache on; the proof lines must show it was not read.
    let cwd = std::env::temp_dir().join(format!(
        "deka_engine_ambient_cwd_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&cwd).expect("mkdir");
    std::env::set_current_dir(&cwd).expect("chdir");

    let loaded = engine::config::RuntimeConfig::load();
    println!("ambient-proof:code-cache={:?}", loaded.code_cache_enabled());
    println!("ambient-proof:retention={}", loaded.introspect_retention_days());
    println!("ambient-proof:profiling={}", loaded.introspect_profiling_enabled());

    std::fs::remove_dir_all(&cwd).ok();
}
