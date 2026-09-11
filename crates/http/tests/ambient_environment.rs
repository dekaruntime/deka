//! deka#801 regression pin for `deka_http`: every HTTP-layer setting is
//! caller-supplied through `HttpConfig`; contradictory process environment
//! must not change behavior.
//!
//! Pattern follows the pool pin (deka#840, crates/pool/src/esm_loader/policy.rs):
//! the parent test spawns this test binary as a child under contradictory
//! `DEKA_*` values and compares only the child's proof lines — never the
//! harness output, which carries a wall-clock "finished in" line that made
//! the raw-stdout comparison flaky.

use std::process::Command;

#[test]
fn http_config_ignores_contradictory_ambient_environment() {
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
        ("DEKA_HTTP_DEBUG", "0"),
        ("DEKA_PLATFORM_API", "0"),
        ("DEKA_RATE_LIMIT_DISABLED", "0"),
        ("DEKA_RATE_LIMIT_REQUESTS_PER_MINUTE", "1"),
        ("DEKA_RATE_LIMIT_BURST", "1"),
        ("DEKA_PROJECT_ROOT", "/not/a/project"),
        ("DEKA_REDIS_URL", "redis://127.0.0.1:1"),
        ("DEKA_NEO4J_URI", "bolt://127.0.0.1:1"),
    ];
    let contradictions_b = [
        ("DEKA_HTTP_DEBUG", "1"),
        ("DEKA_PLATFORM_API", "1"),
        ("DEKA_RATE_LIMIT_DISABLED", "1"),
        ("DEKA_RATE_LIMIT_REQUESTS_PER_MINUTE", "99999"),
        ("DEKA_RATE_LIMIT_BURST", "99999"),
        ("DEKA_PROJECT_ROOT", "/also/not/a/project"),
        ("DEKA_REDIS_URL", "redis://example.invalid:9999/1"),
        ("DEKA_NEO4J_URI", "bolt://example.invalid:9999"),
    ];
    assert_eq!(run(&contradictions_a), run(&contradictions_b));
}

#[test]
#[ignore]
fn ambient_environment_child() {
    // A real project dir whose deka.css.json disables utility CSS. The
    // explicit caller config points at it; a `DEKA_PROJECT_ROOT` env override
    // pointing elsewhere (set by the parent) must not change what loads.
    let root =
        std::env::temp_dir().join(format!("deka_http_ambient_{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("mkdir");
    std::fs::write(
        root.join("deka.css.json"),
        r#"{"utility":{"enabled":false,"preflight":false}}"#,
    )
    .expect("write deka.css.json");

    let config = deka_http::HttpConfig {
        project_root: Some(root.clone()),
        ..Default::default()
    };
    let css = deka_http::utility_css::load_config(config.project_root.as_deref());

    println!("ambient-proof:debug={}", config.debug);
    println!("ambient-proof:platform-api={}", config.platform_api);
    println!("ambient-proof:rate-disabled={}", config.rate_limit.disabled);
    println!("ambient-proof:rate-rpm={}", config.rate_limit.requests_per_minute);
    println!("ambient-proof:rate-burst={}", config.rate_limit.burst);
    println!("ambient-proof:neo4j-uri={}", config.neo4j.uri);
    println!("ambient-proof:neo4j-user={}", config.neo4j.user);
    println!("ambient-proof:redis-url={}", config.redis_url);
    println!("ambient-proof:css-enabled={}", css.enabled);
    println!("ambient-proof:css-preflight={}", css.preflight);

    std::fs::remove_dir_all(&root).ok();
}
