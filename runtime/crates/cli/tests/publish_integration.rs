use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::mpsc,
    thread,
};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn git(repo: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("run git {}: {err}", args.join(" ")));
    assert!(
        output.status.success(),
        "git {} failed:\nstdout: {}\nstderr: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn git_output(repo: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("run git {}: {err}", args.join(" ")));
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn package_repo(version: &str) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("temp package repo");
    let remote = temp.path().join("origin.git");
    let source = temp.path().join("source");
    git(
        temp.path(),
        &["init", "--bare", remote.to_str().expect("remote path")],
    );
    git(
        temp.path(),
        &["init", source.to_str().expect("source path")],
    );
    git(&source, &["config", "user.email", "test@tana.gg"]);
    git(
        &source,
        &["config", "user.name", "Deka publish integration test"],
    );
    git(&source, &["branch", "-M", "main"]);
    git(
        &source,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );
    fs::write(
        source.join("deka.json"),
        format!(
            r#"{{"name":"@deka/publish-fixture","version":"{version}","repository":"deka/publish-fixture","security":{{"allow":{{}}}}}}"#
        ),
    )
    .expect("write manifest");
    fs::write(source.join("index.phpx"), "export const released = true;\n")
        .expect("write package source");
    git(&source, &["add", "."]);
    git(&source, &["commit", "-m", "release candidate"]);
    git(&source, &["push", "-u", "origin", "main"]);
    git(
        temp.path(),
        &[
            "--git-dir",
            remote.to_str().expect("remote path"),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
    );
    (temp, source, remote)
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer).expect("read HTTP request");
        assert_ne!(read, 0, "client closed HTTP request early");
        bytes.extend_from_slice(&buffer[..read]);
        let Some(headers_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..headers_end + 4]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .expect("HTTP Content-Length");
        if bytes.len() >= headers_end + 4 + content_length {
            return String::from_utf8(bytes).expect("UTF-8 HTTP request");
        }
    }
}

fn registry_server() -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind registry fixture");
    let address = listener.local_addr().expect("registry address");
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        for response in [
            r#"{"preflight":{"allowed":true}}"#,
            r#"{"release":{"package_name":"@deka/publish-fixture","version":"1.2.3"}}"#,
        ] {
            let (mut stream, _) = listener.accept().expect("accept registry request");
            let request = read_http_request(&mut stream);
            sender.send(request).expect("record registry request");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.len(),
                response
            )
            .expect("respond to registry request");
        }
    });
    (format!("http://{address}"), receiver, worker)
}

#[test]
fn publish_command_rejects_artifact_with_vendored_php_modules() {
    let package = tempfile::tempdir().expect("create package source tree");
    let root = package.path();
    git(root, &["init"]);
    git(root, &["config", "user.email", "test@tana.gg"]);
    git(
        root,
        &["config", "user.name", "Deka publish integration test"],
    );

    fs::write(
        root.join("deka.json"),
        r#"{"name":"@tana/publish-fixture","version":"1.0.0","repository":"tana/publish-fixture","security":{"allow":{}}}"#,
    )
    .expect("write package manifest");
    let vendored_module = root
        .join("src")
        .join("generated")
        .join("php_modules")
        .join("nested")
        .join("index.phpx");
    fs::create_dir_all(vendored_module.parent().expect("module parent"))
        .expect("create vendored module directory");
    fs::write(&vendored_module, "export const nested = true;").expect("write vendored module");

    // A harmless-looking symlink must not make a vendored dependency tree
    // publishable. Git records symlinks as blobs, which is why this needs the
    // real publish path rather than a filesystem-only assertion.
    #[cfg(unix)]
    std::os::unix::fs::symlink("src/generated/php_modules", root.join("assets"))
        .expect("create innocuous symlink into vendored modules");

    git(root, &["add", "."]);
    git(root, &["commit", "-m", "package fixture"]);

    let output = Command::new(cli_bin())
        .current_dir(root)
        .args([
            "publish",
            "--dry-run",
            "--yes",
            "--token",
            "test-token",
            "--registry-url",
            "http://127.0.0.1:9",
        ])
        .output()
        .expect("run deka publish");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("publish rejected") && stderr.contains("php_modules"),
        "deka publish must reject the Git artifact before contacting the registry; \\
         status: {}\\nstderr: {}",
        output.status,
        stderr,
    );
}

#[test]
fn publish_preserves_qualified_repo_pushes_annotated_tag_and_uses_peeled_commit() {
    let (_temp, source, remote) = package_repo("1.2.3");
    let (registry_url, requests, worker) = registry_server();

    let output = Command::new(cli_bin())
        .current_dir(&source)
        .args([
            "publish",
            "--yes",
            "--repo",
            "deka/publish-fixture",
            "--token",
            "test-token",
            "--registry-url",
            &registry_url,
        ])
        .output()
        .expect("run deka publish");
    assert!(
        output.status.success(),
        "publish failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let preflight = requests.recv().expect("preflight request");
    let publish = requests.recv().expect("publish request");
    worker.join().expect("registry fixture completed");
    assert!(preflight.starts_with("POST /api/packages/preflight "));
    assert!(publish.starts_with("POST /api/packages/publish "));
    let body = publish.split("\r\n\r\n").nth(1).expect("publish body");
    let payload: serde_json::Value = serde_json::from_str(body).expect("publish JSON");
    let head = git_output(&source, &["rev-parse", "HEAD^{commit}"]);
    assert_eq!(payload["repo"], "deka/publish-fixture");
    assert_eq!(payload["git_ref"], head);
    assert_eq!(
        git_output(
            &source,
            &[
                "--git-dir",
                remote.to_str().expect("remote path"),
                "cat-file",
                "-t",
                "refs/tags/v1.2.3",
            ],
        ),
        "tag",
        "release tag must be annotated and pushed to origin"
    );
    assert_eq!(
        git_output(
            &source,
            &[
                "--git-dir",
                remote.to_str().expect("remote path"),
                "rev-parse",
                "refs/tags/v1.2.3^{commit}",
            ],
        ),
        head
    );
}

#[test]
fn publish_fails_before_tagging_when_head_is_not_pushed() {
    let (_temp, source, remote) = package_repo("1.2.3");
    fs::write(
        source.join("index.phpx"),
        "export const released = false;\n",
    )
    .expect("change package source");
    git(&source, &["add", "index.phpx"]);
    git(&source, &["commit", "-m", "unpushed release candidate"]);

    let output = Command::new(cli_bin())
        .current_dir(&source)
        .args([
            "publish",
            "--yes",
            "--token",
            "test-token",
            "--registry-url",
            "http://127.0.0.1:9",
        ])
        .output()
        .expect("run deka publish");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("push source commits to the default branch"),
        "{stderr}"
    );
    let tag = Command::new("git")
        .args([
            "--git-dir",
            remote.to_str().expect("remote path"),
            "show-ref",
            "--verify",
            "refs/tags/v1.2.3",
        ])
        .output()
        .expect("inspect remote tags");
    assert!(
        !tag.status.success(),
        "publish must not tag an unpushed commit"
    );
}

#[test]
fn publish_reports_exact_legacy_capability_key_migration() {
    let temp = tempfile::tempdir().expect("legacy manifest project");
    fs::write(
        temp.path().join("deka.json"),
        r#"{"name":"@deka/legacy","version":"1.0.0","repository":"deka/legacy","deka.security":{"allow":{"run":true}}}"#,
    )
    .expect("write legacy manifest");
    let output = Command::new(cli_bin())
        .current_dir(temp.path())
        .args(["publish", "--yes", "--token", "test-token"])
        .output()
        .expect("run deka publish");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("deka.security.allow.*` with `security.allow.*"),
        "{stderr}"
    );
}
