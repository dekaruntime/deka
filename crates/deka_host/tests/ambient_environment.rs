//! deka#801 regression pin for `deka_host`: host behavior (module-root
//! resolution, neo4j endpoint defaults, target-capability validation)
//! comes from explicit inputs and installer functions only; contradictory
//! process environment must not change behavior. Before this pin,
//! `DEKA_MODULE_ROOT`, `DEKA_TARGET` / `DEKA_HOST_PROFILE`, `DEKA_NEO4J_*`,
//! `HANDLER_PATH`, and `DEKA_SECURITY_NO_PROMPT` were read
//! from the ambient environment at call time.
//!
//! Pattern follows the engine pin (deka#847, crates/engine/tests/
//! ambient_environment.rs): the parent test spawns this test binary as a
//! child and compares only the child's proof lines — never the harness
//! timing output.

use std::process::Command;

#[test]
fn host_modules_ignore_contradictory_ambient_environment() {
    let test_bin = std::env::current_exe().expect("current test binary");
    let run = |with_poisoned_env: bool| {
        let mut command = Command::new(&test_bin);
        command.args([
            "--exact",
            "ambient_environment_child",
            "--ignored",
            "--nocapture",
        ]);
        if with_poisoned_env {
            // Every one of these WOULD flip host behavior if the ambient
            // environment were still a config channel:
            // - DEKA_MODULE_ROOT names a real project (deka.lock + ds_modules
            //   with the module the child's entry imports).
            // - DEKA_TARGET / DEKA_HOST_PROFILE select the 'adwa' target,
            //   which blocks capability imports the 'server' target allows.
            // - DEKA_NEO4J_* point the bridge defaults at
            //   nonexistent endpoints.
            // - HANDLER_PATH names a fake PHPX handler (project-kind hints).
            // - DEKA_SECURITY_NO_PROMPT suppresses interactive prompts.
            let offered = std::env::temp_dir().join(format!(
                "deka_host_ambient_offered_{}",
                std::process::id()
            ));
            command.envs([
                ("DEKA_MODULE_ROOT", offered.to_string_lossy().into_owned()),
                ("DEKA_TARGET", "adwa".to_string()),
                ("DEKA_HOST_PROFILE", "adwa".to_string()),
                ("DEKA_NEO4J_URI", "bolt://127.0.0.1:1".to_string()),
                ("DEKA_NEO4J_USER", "poisoned".to_string()),
                ("DEKA_NEO4J_PASSWORD", "poisoned".to_string()),
                ("DEKA_NEO4J_DB", "poisoned".to_string()),
                ("DEKA_SECURITY_NO_PROMPT", "1".to_string()),
                ("HANDLER_PATH", "/nonexistent/project/main.phpx".to_string()),
            ]);
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

    // A module root that WOULD resolve the child's unresolved import if the
    // ambient environment were still a config channel. The child must ignore
    // it.
    let offered = std::env::temp_dir().join(format!(
        "deka_host_ambient_offered_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(offered.join("ds_modules")).expect("write offered ds_modules");
    std::fs::write(offered.join("deka.lock"), "{}").expect("write offered lock");
    std::fs::write(
        offered.join("ds_modules").join("proven.ds"),
        "export const value = 1;\n",
    )
    .expect("write offered module");

    assert_eq!(run(true), run(false));

    std::fs::remove_dir_all(&offered).ok();
}

#[test]
#[ignore]
fn ambient_environment_child() {
    // cwd is an empty dir: no cwd-relative project candidate can load. Any
    // ambient env the parent sets points at values that would change host
    // behavior; the proof lines must show they were not read.
    let cwd = std::env::temp_dir().join(format!(
        "deka_host_ambient_cwd_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&cwd).expect("mkdir");
    std::env::set_current_dir(&cwd).expect("chdir");

    let entry = cwd.join("main.ds");
    std::fs::write(&entry, "import { value } from 'proven'\n").expect("write entry");

    // Module-root resolution: 'proven' exists only in the DEKA_MODULE_ROOT
    // the parent offers. Both runs must miss it identically.
    let resolution_errors =
        deka_host::validation::modules::validate_module_resolution("import { value } from 'proven'\n", entry.to_string_lossy().as_ref());
    println!("ambient-proof:module-resolution-errors={}", resolution_errors.len());

    // Target-capability validation against an explicit target: the ambient
    // DEKA_TARGET / DEKA_HOST_PROFILE must not select the target.
    let capability_errors = deka_host::validation::modules::validate_target_capabilities_for(
        "server",
        "import { query } from 'db/postgres'\n",
        entry.to_string_lossy().as_ref(),
    );
    println!(
        "ambient-proof:target-capability-errors={}",
        capability_errors.len()
    );

    // Bridge endpoint defaults: resolve from the installed deka_host store
    // (nothing installed here), never from DEKA_NEO4J_*.
    println!(
        "ambient-proof:neo4j-uri={}",
        deka_host::modules::neo4j::shard_route_neo4j(&serde_json::Value::Null)
    );
    println!(
        "ambient-proof:database-endpoints={:?}",
        deka_host::host_config::database_endpoints()
    );
    println!(
        "ambient-proof:handler-paths={:?}",
        deka_host::host_config::handler_paths()
    );

    std::fs::remove_dir_all(&cwd).ok();
}
