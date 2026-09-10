//! deka#731: watch eviction must target the pool that served the request.
//!
//! A source edit can appear to reload without eviction because the ESM module
//! graph key changes. This test asserts both externally visible output and the
//! non-zero eviction count after first populating the serving pool.

use reqwest::blocking::Client;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

struct ServeProcess(Child);

impl Drop for ServeProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read ephemeral port")
        .port()
}

fn write_handler(root: &Path, output: &str) {
    fs::write(
        root.join("index.js"),
        format!(
            r#"globalThis.app = {{
  async fetch() {{
    return new Response({output:?});
  }},
}};
"#
        ),
    )
    .expect("write watched handler");
}

fn get_body(client: &Client, port: u16) -> Result<String, String> {
    client
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .map_err(|err| err.to_string())?
        .error_for_status()
        .map_err(|err| err.to_string())?
        .text()
        .map_err(|err| err.to_string())
}

#[test]
fn watched_edit_changes_served_output_and_evicts_the_serving_pool() {
    let root = tempfile::tempdir().expect("create project");
    fs::write(
        root.path().join("deka.json"),
        r#"{"name":"watch-reload","security":{"prompt":false}}"#,
    )
    .expect("write manifest");
    write_handler(root.path(), "before edit");

    let port = free_port();
    let log_path = root.path().join("serve.log");
    let log = fs::File::create(&log_path).expect("create serve log");
    let mut command = Command::new(cli_bin());
    command
        .args([
            "serve",
            ".",
            "--dev",
            "--port",
            &port.to_string(),
            "--no-prompt",
        ])
        .current_dir(root.path())
        .env("DEKA_RATE_LIMIT_DISABLED", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone serve log")))
        .stderr(Stdio::from(log));
    let dsc_beside_cli = Path::new(cli_bin()).with_file_name("dsc");
    if dsc_beside_cli.is_file() {
        command.env("DEKA_DSC", dsc_beside_cli);
    }
    let _server = ServeProcess(command.spawn().expect("spawn deka serve"));

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("build HTTP client");
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last = String::new();
    while Instant::now() < deadline {
        match get_body(&client, port) {
            Ok(body) if body == "before edit" => break,
            Ok(body) => last = format!("unexpected body {body:?}"),
            Err(err) => last = err,
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        get_body(&client, port).expect("initial request should succeed"),
        "before edit",
        "server did not serve the initial source: {last}"
    );

    write_handler(root.path(), "after edit");
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut served_new_output = false;
    let mut saw_nonzero_eviction = false;
    let mut last_body = String::new();
    while Instant::now() < deadline {
        if let Ok(body) = get_body(&client, port) {
            served_new_output |= body == "after edit";
            last_body = body;
        }
        let log = fs::read_to_string(&log_path).unwrap_or_default();
        saw_nonzero_eviction |= log.lines().any(|line| {
            line.contains("[watch] evicted ")
                && line
                    .split("[watch] evicted ")
                    .nth(1)
                    .and_then(|count| count.split_whitespace().next())
                    .and_then(|count| count.parse::<usize>().ok())
                    .is_some_and(|count| count > 0)
        });
        if served_new_output && saw_nonzero_eviction {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let log = fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        served_new_output,
        "watched source edit did not change served output (last body {last_body:?}); log:\n{log}"
    );
    assert!(
        saw_nonzero_eviction,
        "watch must evict a non-zero number of serving isolates; log:\n{log}"
    );
}
