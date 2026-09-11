//! deka#801 regression pin for `cli`: registry/token/db/gild/monitor config
//! flows explicitly through CLI flags → auth profile → deka.json → built-in
//! defaults; contradictory process environment must not change what any of
//! the resolution functions returns.
//!
//! Pattern follows the transport pin (deka#850, crates/transport/tests/
//! ambient_environment.rs): the parent test spawns this test binary as a
//! child under contradictory `LINKHASH_*` / `TANA_GIT_*` / `DB_*` / `GILD_*`
//! / `DEKA_*` values and compares only the child's proof lines — never the
//! harness output, which carries a wall-clock "finished in" line that made
//! the raw-stdout comparison flaky (deka#840).

use core::Context;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn cli_config_ignores_contradictory_ambient_environment() {
    let test_bin = std::env::current_exe().expect("current test binary");
    let run = |envs: &[(&str, &str)]| {
        let mut command = Command::new(&test_bin);
        command.args([
            "--exact",
            "ambient_environment_child",
            "--ignored",
            "--nocapture",
        ]);
        for (key, value) in envs {
            command.env(key, value);
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

    let contradictions_a = [
        ("LINKHASH_TOKEN", "0"),
        ("TANA_GIT_TOKEN", "0"),
        ("LINKHASH_REGISTRY_URL", "http://127.0.0.1:0/a"),
        ("LINKHASH_REGISTRY", "http://127.0.0.1:0/b"),
        ("TANA_GIT_SERVER", "http://127.0.0.1:0/c"),
        ("LINKHASH_CARGO_INDEX", "http://127.0.0.1:0/d"),
        ("DATABASE_URL", "http://127.0.0.1:0/e"),
        ("DB_HOST", "0"),
        ("DB_PORT", "0"),
        ("DB_NAME", "0"),
        ("DB_USER", "0"),
        ("DB_PASSWORD", "0"),
        ("GILD_SOCKET_PATH", "0"),
        ("GILD_BEARER_TOKEN", "0"),
        ("DEKA_MONITOR_INTERVAL", "0"),
        ("DEKA_DEBUG", "0"),
    ];
    let contradictions_b = [
        ("LINKHASH_TOKEN", "1"),
        ("TANA_GIT_TOKEN", "1"),
        ("LINKHASH_REGISTRY_URL", "http://example.invalid:9999/a"),
        ("LINKHASH_REGISTRY", "http://example.invalid:9999/b"),
        ("TANA_GIT_SERVER", "http://example.invalid:9999/c"),
        ("LINKHASH_CARGO_INDEX", "http://example.invalid:9999/d"),
        ("DATABASE_URL", "http://example.invalid:9999/e"),
        ("DB_HOST", "1"),
        ("DB_PORT", "1"),
        ("DB_NAME", "1"),
        ("DB_USER", "1"),
        ("DB_PASSWORD", "1"),
        ("GILD_SOCKET_PATH", "1"),
        ("GILD_BEARER_TOKEN", "1"),
        ("DEKA_MONITOR_INTERVAL", "99999"),
        ("DEKA_DEBUG", "1"),
    ];
    assert_eq!(run(&contradictions_a), run(&contradictions_b));
}

fn dummy_context(cwd: PathBuf) -> core::Context {
    Context {
        args: core::Args {
            flags: HashMap::new(),
            params: HashMap::new(),
            commands: vec!["self".to_string(), "monitor".to_string()],
            positionals: Vec::new(),
        },
        env: core::EnvContext {
            vars: HashMap::new(),
            cwd,
        },
        handler: core::HandlerContext {
            input: ".".to_string(),
            resolved: core::ResolvedHandler {
                path: PathBuf::from("."),
                directory: PathBuf::from("."),
                mode: core::ServeMode::Php,
                config: core::ServeConfig::default(),
            },
            static_config: core::StaticServeConfig::default(),
            serve_config_path: None,
        },
    }
}

#[test]
#[ignore]
fn ambient_environment_child() {
    // Resolve every config channel exactly as the product code does and
    // print the values it dispatches on. The parent sets the vars these
    // functions used to read; a reintroduced env read would flip a proof
    // line and fail the parent's comparison.
    let dir = std::env::temp_dir().join(format!("deka-cli-ambient-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("deka.json"),
        r#"{"db": {"engine": "postgres"}, "self": {"monitor": {"interval_seconds": 42}}}"#,
    )
    .unwrap();

    let db = cli::cli::db::config::read_db_runtime_config(&dir);
    println!("ambient-proof:db-engine={:?}", db.engine);
    println!("ambient-proof:db-location={}", db.location);

    let ctx = dummy_context(dir.clone());

    let (registry, token) = cli::cli::install::get_registry_config(&ctx);
    println!("ambient-proof:install-registry={}", registry);
    println!("ambient-proof:install-token={:?}", token);

    let (registry, token, index) = cli::cli::self_cmd::update::get_registry_config(&ctx);
    println!("ambient-proof:self-update-registry={}", registry);
    println!("ambient-proof:self-update-token={:?}", token);
    println!("ambient-proof:self-update-index={:?}", index);

    let monitor = cli::cli::self_cmd::monitor::load_monitor_config(&ctx).unwrap();
    println!(
        "ambient-proof:monitor-poll-secs={}",
        monitor.poll_interval.as_secs()
    );
    println!(
        "ambient-proof:monitor-registry={}",
        monitor.update_config.registry_url
    );
    println!(
        "ambient-proof:monitor-token={:?}",
        monitor.update_config.token
    );
    println!(
        "ambient-proof:monitor-index={:?}",
        monitor.update_config.registry_index_url
    );

    let auth = cli::cli::publish::resolve_registry_auth(&HashMap::new(), None);
    println!("ambient-proof:publish-auth={:?}", auth);

    let (socket, bearer) = cli::cli::deploy::resolve_gild_endpoint(&HashMap::new());
    println!("ambient-proof:gild-socket={}", socket);
    println!("ambient-proof:gild-bearer={}", bearer);

    let _ = std::fs::remove_file(dir.join("deka.json"));
    let _ = std::fs::remove_dir(&dir);
}
