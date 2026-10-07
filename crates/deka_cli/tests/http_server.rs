#[path = "support/check.rs"]
mod check;
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    time::Duration,
};
fn ok(o: Output) {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn running(mut command: Command) -> (Process, u16) {
    let mut child = Process(
        command
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stderr = child.0.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut first = String::new();
        let result = reader.read_line(&mut first).map(|_| first);
        let _ = tx.send(result);
        std::io::copy(&mut reader, &mut std::io::sink()).unwrap();
    });
    let line = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("server startup deadline")
        .unwrap();
    let port = line
        .strip_prefix("Listening on http://127.0.0.1:")
        .and_then(|line| line.strip_suffix("/\n"))
        .unwrap_or_else(|| panic!("unexpected server startup: {line}"))
        .parse()
        .unwrap();
    (child, port)
}
fn request(port: u16, expected: &str) {
    let r = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .post(format!("http://127.0.0.1:{port}/guide"))
        .body("hello")
        .send()
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().unwrap(), expected);
}
fn source_command(cli: &str, dir: &Path, cache: &Path, cmd: &str) -> Command {
    let mut c = Command::new(cli);
    c.current_dir(dir).env("DEKA_CACHE_DIR", cache).args([
        cmd,
        "server.ds",
        "--hostname",
        "127.0.0.1",
        "--port",
        "0",
    ]);
    c
}
#[test]
fn exact_guide_runs_from_source_warm_cache_and_relocated_source_free_executable() {
    let guide = include_str!("../../../docs/dekascript/native/http-server.mdx")
        .split("```ds\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    std::fs::write(root.path().join("server.ds"), guide).unwrap();
    check::checked(
        Command::new(cli)
            .current_dir(root.path())
            .args(["check", "server.ds"])
            .output()
            .unwrap(),
        "server.ds",
    );
    for cmd in ["serve", "run", "serve"] {
        let (child, port) = running(source_command(cli, root.path(), cache.path(), cmd));
        request(port, "POST:hello");
        drop(child);
    }
    assert_eq!(
        std::fs::read_dir(cache.path().join("native"))
            .unwrap()
            .count(),
        1
    );
    let programmatic = include_str!("../../../docs/dekascript/native/http-server.mdx")
        .split("```ds\n")
        .nth(2)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    std::fs::write(root.path().join("programmatic.ds"), programmatic).unwrap();
    check::checked(
        Command::new(cli)
            .current_dir(root.path())
            .args(["check", "programmatic.ds"])
            .output()
            .unwrap(),
        "programmatic.ds",
    );
    let executable = root.path().join("app");
    ok(Command::new(cli)
        .current_dir(root.path())
        .args(["build", "server.ds", "--outfile"])
        .arg(&executable)
        .output()
        .unwrap());
    let moved = tempfile::tempdir().unwrap();
    let app = moved.path().join("server");
    std::fs::rename(&executable, &app).unwrap();
    drop(root);
    drop(cache);
    let mut command = Command::new(&app);
    command
        .env_clear()
        .current_dir(moved.path())
        .args(["--hostname", "127.0.0.1", "--port", "0"]);
    let (child, port) = running(command);
    request(port, "POST:hello");
    drop(child);
    for cmd in ["serve", "run"] {
        let mut c = Command::new(cli);
        c.env_clear()
            .current_dir(moved.path())
            .arg(cmd)
            .arg(&app)
            .args(["--hostname", "127.0.0.1", "--port", "0"]);
        let (child, port) = running(c);
        request(port, "POST:hello");
        drop(child);
    }
}
#[test]
fn authored_artifact_wins_and_invalid_artifact_never_falls_back_to_source() {
    let root = tempfile::tempdir().unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    let source = root.path().join("server.ds");
    std::fs::write(
        &source,
        "export default {fetch(req){return Response(\"authored\");}};",
    )
    .unwrap();
    let app = root.path().join("dist/deka-app");
    ok(Command::new(cli)
        .current_dir(root.path())
        .args(["build", "server.ds", "--outfile"])
        .arg(&app)
        .output()
        .unwrap());
    std::fs::write(&source, "this is deliberately invalid source").unwrap();
    for path in [root.path().to_owned(), root.path().join("deka.json")] {
        let mut c = Command::new(cli);
        c.arg("serve")
            .arg(path)
            .args(["--hostname", "127.0.0.1", "--port", "0"]);
        let (child, port) = running(c);
        request(port, "authored");
        drop(child);
    }
    std::fs::write(&app, b"invalid authored artifact").unwrap();
    let o = Command::new(cli)
        .arg("serve")
        .arg(root.path())
        .args(["--port", "0"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    let message = String::from_utf8_lossy(&o.stderr);
    assert!(message.contains("authored application"), "{message}");
    assert!(!message.contains("Listening"));
}
#[test]
fn invalid_default_handler_and_options_fail_before_listening() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    for source in [
        "export default {fetch(req){return \"wrong\";}};",
        "export default {fetch(req){const s:string=req.method;const n:number=req.method;return Response(s);}};",
    ] {
        std::fs::write(root.path().join("server.ds"), source).unwrap();
        let o = source_command(cli, root.path(), cache.path(), "serve")
            .output()
            .unwrap();
        assert!(!o.status.success());
        assert!(!String::from_utf8_lossy(&o.stderr).contains("Listening"));
    }
    std::fs::write(
        root.path().join("server.ds"),
        "export default {fetch(req){return Response(\"ok\");}};",
    )
    .unwrap();
    for port in ["-1", "65536", "0.5", "NaN"] {
        let o = Command::new(cli)
            .current_dir(root.path())
            .env("DEKA_CACHE_DIR", cache.path())
            .args(["serve", "server.ds", "--port", port])
            .output()
            .unwrap();
        assert!(!o.status.success());
        assert!(!String::from_utf8_lossy(&o.stderr).contains("Listening"));
    }
    std::fs::write(root.path().join("ordinary.ds"), "fn main(){return;} ").unwrap();
    let o = Command::new(cli)
        .current_dir(root.path())
        .args(["serve", "ordinary.ds"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("default export"));
}
#[test]
fn initializer_guide_executes_and_default_method_formatter_keeps_server_semantics() {
    let cli = env!("CARGO_BIN_EXE_deka");
    let root = tempfile::tempdir().unwrap();
    let optional = include_str!("../../../docs/dekascript/native/optional-records.mdx")
        .split("```ds\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    std::fs::write(root.path().join("optional.ds"), optional).unwrap();
    let o = Command::new(cli)
        .current_dir(root.path())
        .args(["run", "optional.ds"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.stdout, b"default port\nport 8000\n");
    let guide = include_str!("../../../docs/dekascript/native/http-server.mdx")
        .split("```ds\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let path = root.path().join("server.ds");
    std::fs::write(&path, guide).unwrap();
    ok(Command::new(cli).arg("fmt").arg(&path).output().unwrap());
    check::checked(
        Command::new(cli).arg("check").arg(&path).output().unwrap(),
        &path.display().to_string(),
    );
    let cache = tempfile::tempdir().unwrap();
    let (child, port) = running(source_command(cli, root.path(), cache.path(), "serve"));
    request(port, "POST:hello");
    drop(child);
}
#[test]
fn changed_source_corrupt_cache_eviction_and_explicit_cleanup_use_the_real_server() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let cli = env!("CARGO_BIN_EXE_deka");
    let path = root.path().join("server.ds");
    std::fs::write(
        &path,
        "export default {fetch(req){return Response(\"first\");}};",
    )
    .unwrap();
    let (child, port) = running(source_command(cli, root.path(), cache.path(), "serve"));
    request(port, "first");
    drop(child);
    let dir = cache.path().join("native");
    let original = std::fs::read_dir(&dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(
        &path,
        "export default {fetch(req){return Response(\"second\");}};",
    )
    .unwrap();
    let (child, port) = running(source_command(cli, root.path(), cache.path(), "serve"));
    request(port, "second");
    drop(child);
    let changed = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p != &original)
        .unwrap();
    std::fs::write(&changed, b"corrupt runtime cache").unwrap();
    let (child, port) = running(source_command(cli, root.path(), cache.path(), "serve"));
    request(port, "second");
    drop(child);
    assert!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&changed).unwrap()).is_ok());
    for n in 0..65 {
        let p = dir.join(format!("{n:064x}.json"));
        let f = std::fs::File::create(p).unwrap();
        f.set_modified(std::time::UNIX_EPOCH).unwrap();
    }
    std::fs::write(
        &path,
        "export default {fetch(req){return Response(\"third\");}};",
    )
    .unwrap();
    let (child, port) = running(source_command(cli, root.path(), cache.path(), "serve"));
    request(port, "third");
    drop(child);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 64);
    let unrelated = dir.join("keep.txt");
    std::fs::write(&unrelated, "unrelated").unwrap();
    let legacy = cache.path().join("legacy");
    std::fs::create_dir(&legacy).unwrap();
    std::fs::write(legacy.join("keep"), "legacy").unwrap();
    let o = Command::new(cli)
        .env("DEKA_CACHE_DIR", cache.path())
        .args(["cache", "clean"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.stdout, b"Cleared 64 native cache entries\n");
    assert!(unrelated.exists());
    assert!(legacy.join("keep").exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
}
