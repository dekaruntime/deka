//! deka#801 regression pin for `transport`: every listener option is
//! caller-supplied (`ListenConfig` and the embedded `deka_http::HttpConfig`);
//! contradictory process environment must not change what `serve()`
//! dispatches on.
//!
//! Pattern follows the http/engine pins (deka#847, crates/http/tests/
//! ambient_environment.rs): the parent test spawns this test binary as a
//! child under contradictory `DEKA_*` values and compares only the child's
//! proof lines — never the harness output, which carries a wall-clock
//! "finished in" line that made the raw-stdout comparison flaky (deka#840).

use std::process::Command;

#[test]
fn listen_config_ignores_contradictory_ambient_environment() {
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
        ("DEKA_RATE_LIMIT_DISABLED", "0"),
        ("DEKA_RATE_LIMIT_REQUESTS_PER_MINUTE", "1"),
        ("DEKA_RATE_LIMIT_BURST", "1"),
        ("DEKA_PROJECT_ROOT", "/not/a/project"),
    ];
    let contradictions_b = [
        ("DEKA_HTTP_DEBUG", "1"),
        ("DEKA_RATE_LIMIT_DISABLED", "1"),
        ("DEKA_RATE_LIMIT_REQUESTS_PER_MINUTE", "99999"),
        ("DEKA_RATE_LIMIT_BURST", "99999"),
        ("DEKA_PROJECT_ROOT", "/also/not/a/project"),
    ];
    assert_eq!(run(&contradictions_a), run(&contradictions_b));
}

#[test]
#[ignore]
fn ambient_environment_child() {
    // Build every ListenConfig variant exactly as a caller would and print
    // the values `serve()` dispatches on. The parent sets the DEKA_* vars
    // this config used to be read from; a reintroduced env read would flip
    // a proof line and fail the parent's comparison.
    let http = transport::ListenConfig::Http(transport::HttpOptions {
        port: 3000,
        listeners: 4,
        perf_mode: false,
        http: deka_http::HttpConfig::default(),
    });
    let unix = transport::ListenConfig::Unix(transport::UnixOptions {
        path: "/tmp/deka-ambient.sock".to_string(),
        http: deka_http::HttpConfig::default(),
    });
    let ws = transport::ListenConfig::Ws(transport::WsOptions { port: 3001 });
    let tcp = transport::ListenConfig::Tcp(transport::TcpOptions {
        addr: "127.0.0.1:9000".to_string(),
    });
    let udp = transport::ListenConfig::Udp(transport::UdpOptions {
        addr: "127.0.0.1:9001".to_string(),
    });
    let dns = transport::ListenConfig::Dns(transport::DnsOptions {
        addr: "127.0.0.1:53".to_string(),
    });

    if let transport::ListenConfig::Http(options) = &http {
        println!("ambient-proof:http-port={}", options.port);
        println!("ambient-proof:http-listeners={}", options.listeners);
        println!("ambient-proof:http-perf={}", options.perf_mode);
        println!("ambient-proof:http-debug={}", options.http.debug);
        println!(
            "ambient-proof:http-rate-disabled={}",
            options.http.rate_limit.disabled
        );
        println!(
            "ambient-proof:http-rate-rpm={}",
            options.http.rate_limit.requests_per_minute
        );
        println!(
            "ambient-proof:http-rate-burst={}",
            options.http.rate_limit.burst
        );
        println!(
            "ambient-proof:http-project-root={}",
            options
                .http
                .project_root
                .as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
    }
    if let transport::ListenConfig::Unix(options) = &unix {
        println!("ambient-proof:unix-path={}", options.path);
        println!("ambient-proof:unix-debug={}", options.http.debug);
    }
    if let transport::ListenConfig::Ws(options) = &ws {
        println!("ambient-proof:ws-port={}", options.port);
    }
    if let transport::ListenConfig::Tcp(options) = &tcp {
        println!("ambient-proof:tcp-addr={}", options.addr);
    }
    if let transport::ListenConfig::Udp(options) = &udp {
        println!("ambient-proof:udp-addr={}", options.addr);
    }
    if let transport::ListenConfig::Dns(options) = &dns {
        println!("ambient-proof:dns-addr={}", options.addr);
    }
}
