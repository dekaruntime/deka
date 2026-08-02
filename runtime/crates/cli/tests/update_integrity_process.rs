use modules_php::integrity::compute_package_integrity;
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
        assert_update_rejected(case);
    }
}

#[derive(Clone, Copy, Debug)]
enum DigestCase {
    Tampered,
    Missing,
    Malformed,
}

fn assert_update_rejected(case: DigestCase) {
    let project = tempfile::tempdir().expect("project tempdir");
    let package = project.path().join("php_modules/@tana/store");
    std::fs::create_dir_all(&package).expect("package directory");
    std::fs::write(package.join("marker.phpx"), "old\n").expect("old package marker");

    let server = FixtureServer::start(case);
    let cli = std::env::var("CARGO_BIN_EXE_cli").expect("Cargo must provide the cli binary");
    let output = Command::new(cli)
        .current_dir(project.path())
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
    assert!(!project.path().join("deka.lock").exists());
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
