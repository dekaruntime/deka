use deka_host::integrity::compute_package_integrity;
use serde_json::json;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    thread,
};

#[test]
fn update_process_rejects_tampered_missing_and_malformed_digests() {
    for case in [
        DigestCase::Tampered,
        DigestCase::Missing,
        DigestCase::Malformed,
    ] {
        assert_update_rejected(case, false);
    }
}

#[test]
fn shop_update_process_rejects_tampered_missing_and_malformed_digests() {
    for case in [
        DigestCase::Tampered,
        DigestCase::Missing,
        DigestCase::Malformed,
    ] {
        assert_update_rejected(case, true);
    }
}

#[derive(Clone, Copy, Debug)]
enum DigestCase {
    Tampered,
    Missing,
    Malformed,
}

fn assert_update_rejected(case: DigestCase, shop_mode: bool) {
    let project = tempfile::tempdir().expect("project tempdir");
    let worktree = if shop_mode {
        project.path().join("store/tenants/shop_alpha")
    } else {
        project.path().to_path_buf()
    };
    std::fs::create_dir_all(&worktree).expect("worktree directory");
    if shop_mode {
        std::fs::write(
            worktree.join("deka.json"),
            r#"{"dependencies":{"@tana/store":"1.0.0"}}"#,
        )
        .expect("shop manifest");
        std::fs::write(
            worktree.join("deka.lock"),
            json!({
                "lockfileVersion": 1,
                "packages": {
                    "@tana/store": ["0.9.0", "linkhash:@tana/store", {}, ""]
                }
            })
            .to_string(),
        )
        .expect("shop lock");
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&worktree)
            .status()
            .expect("init shop git repo");
        assert!(init.success(), "git init failed");
        let config_name = Command::new("git")
            .args(["config", "user.name", "fixture"])
            .current_dir(&worktree)
            .status()
            .expect("configure git user name");
        assert!(config_name.success(), "git config user.name failed");
        let config_email = Command::new("git")
            .args(["config", "user.email", "fixture@example.test"])
            .current_dir(&worktree)
            .status()
            .expect("configure git user email");
        assert!(config_email.success(), "git config user.email failed");
    }

    let package = worktree.join("php_modules/@tana/store");
    std::fs::create_dir_all(&package).expect("package directory");
    std::fs::write(package.join("marker.phpx"), "old\n").expect("old package marker");

    let original_lock = std::fs::read(worktree.join("deka.lock")).ok();
    if shop_mode {
        let add = Command::new("git")
            .args(["add", "deka.json", "deka.lock", "php_modules"])
            .current_dir(&worktree)
            .status()
            .expect("stage shop fixture");
        assert!(add.success(), "git add failed");
        let commit = Command::new("git")
            .args(["commit", "--quiet", "-m", "fixture"])
            .current_dir(&worktree)
            .status()
            .expect("commit shop fixture");
        assert!(commit.success(), "git commit failed");
    }

    let server = FixtureServer::start(case);
    let cli = std::env::var("CARGO_BIN_EXE_cli").expect("Cargo must provide the cli binary");
    let output = Command::new(cli)
        .current_dir(&worktree)
        .args(["update", "@tana/store@1.0.0"])
        .env("LINKHASH_REGISTRY_URL", &server.url)
        .output()
        .expect("spawn deka update");

    assert!(
        !output.status.success(),
        "{case:?} digest rejection returned success: status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(package.join("marker.phpx")).expect("old marker remains"),
        "old\n"
    );
    if shop_mode {
        assert_eq!(
            std::fs::read(worktree.join("deka.lock")).expect("shop lock remains"),
            original_lock.expect("original shop lock")
        );
    } else {
        assert!(!worktree.join("deka.lock").exists());
    }
}

struct FixtureServer {
    url: String,
}

impl FixtureServer {
    fn start(case: DigestCase) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
        let url = format!("http://{}", listener.local_addr().expect("fixture address"));
        let good = vec![(
            "index.phpx",
            b"export function marker() { return 1; }\n".to_vec(),
        )];
        let digest_root = tempfile::tempdir().expect("digest tempdir");
        std::fs::write(digest_root.path().join("index.phpx"), &good[0].1).expect("digest fixture");
        let digest = compute_package_integrity(digest_root.path())
            .expect("fixture digest")
            .fs_graph;
        let served = match case {
            DigestCase::Tampered => vec![(
                "index.phpx",
                b"export function marker() { return 999; }\n".to_vec(),
            )],
            _ => good,
        };
        let metadata = match case {
            DigestCase::Tampered => {
                json!({"version":"1.0.0", "repo":"fixture", "git_ref":"fixture", "digest": format!("sha256:{digest}")})
            }
            DigestCase::Missing => {
                json!({"version":"1.0.0", "repo":"fixture", "git_ref":"fixture"})
            }
            DigestCase::Malformed => {
                json!({"version":"1.0.0", "repo":"fixture", "git_ref":"fixture", "digest":"sha256:not-a-digest"})
            }
        };
        thread::spawn(move || {
            for _ in 0..5 {
                let (stream, _) = listener.accept().expect("fixture request");
                serve_request(stream, &served, &metadata);
            }
        });
        Self { url }
    }
}

fn serve_request(
    mut stream: TcpStream,
    files: &[(impl AsRef<str>, Vec<u8>)],
    metadata: &serde_json::Value,
) {
    let mut request = [0u8; 8192];
    let size = stream.read(&mut request).expect("fixture read");
    let request = String::from_utf8_lossy(&request[..size]);
    let path = request.split_whitespace().nth(1).unwrap_or("");
    let (status, body, content_type) = if path.ends_with("/versions") {
        (
            200,
            json!({"versions":["1.0.0"]}).to_string().into_bytes(),
            "application/json",
        )
    } else if path.ends_with("/latest") {
        (
            200,
            json!({"version":"1.0.0"}).to_string().into_bytes(),
            "application/json",
        )
    } else if path.ends_with("/tree") {
        let files = files
            .iter()
            .map(|(path, _)| json!({"path": path.as_ref(), "type":"blob"}))
            .collect::<Vec<_>>();
        (
            200,
            json!({"files":files}).to_string().into_bytes(),
            "application/json",
        )
    } else if path.contains("/blob?") {
        let file = path.split("path=").nth(1).unwrap_or("");
        let file = file.split('&').next().unwrap_or(file);
        let bytes = files
            .iter()
            .find(|(path, _)| path.as_ref() == file)
            .map(|(_, bytes)| bytes.clone())
            .expect("fixture blob");
        (
            200,
            json!({"content":String::from_utf8(bytes).unwrap()})
                .to_string()
                .into_bytes(),
            "application/json",
        )
    } else {
        (
            200,
            serde_json::to_vec(metadata).expect("fixture metadata"),
            "application/json",
        )
    };
    let header = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .expect("fixture headers");
    stream.write_all(&body).expect("fixture body");
}
