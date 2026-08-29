use deka_host::integrity::compute_package_integrity;
use pm::{InstallPayload, run_install};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    thread,
};

static CWD_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn real_install_rejects_tampered_artifact() {
    assert_install_rejected(DigestCase::Tampered);
}

#[test]
fn real_install_rejects_missing_digest() {
    assert_install_rejected(DigestCase::Missing);
}

#[test]
fn real_install_rejects_malformed_digest() {
    assert_install_rejected(DigestCase::Malformed);
}

#[derive(Clone, Copy)]
enum DigestCase {
    Tampered,
    Missing,
    Malformed,
}

fn assert_install_rejected(case: DigestCase) {
    let _guard = CWD_LOCK.lock().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("php_modules/@tana/store")).unwrap();
    std::fs::write(
        project.path().join("php_modules/@tana/store/marker.phpx"),
        "old",
    )
    .unwrap();
    let old_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(project.path()).unwrap();

    let server = FixtureServer::start(case);
    unsafe {
        std::env::set_var("LINKHASH_REGISTRY_URL", &server.url);
        std::env::remove_var("LINKHASH_TOKEN");
    }
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(run_install(InstallPayload::from_parts(vec![
            "@tana/store@1.0.0".to_string(),
        ])));

    unsafe {
        std::env::remove_var("LINKHASH_REGISTRY_URL");
    }
    std::env::set_current_dir(old_dir).unwrap();
    assert!(
        result.is_err(),
        "tampered/malformed/missing digest installed"
    );
    assert_eq!(
        std::fs::read_to_string(project.path().join("php_modules/@tana/store/marker.phpx"))
            .unwrap(),
        "old"
    );
    assert!(!project.path().join("deka.lock").exists());
}

struct FixtureServer {
    url: String,
}

impl FixtureServer {
    fn start(case: DigestCase) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let good = vec![(
            "index.phpx",
            b"export function marker() { return 1; }\n".to_vec(),
        )];
        let digest_root = tempfile::tempdir().unwrap();
        for (path, bytes) in &good {
            let target = digest_root.path().join(path);
            std::fs::write(target, bytes).unwrap();
        }
        let digest = compute_package_integrity(digest_root.path())
            .unwrap()
            .fs_graph;
        let served = match case {
            DigestCase::Tampered => vec![(
                "index.phpx",
                b"export function marker() { return 999; }\n".to_vec(),
            )],
            _ => good.clone(),
        };
        let metadata = match case {
            DigestCase::Missing => {
                json!({"version":"1.0.0", "repo":"fixture", "git_ref":"fixture"})
            }
            DigestCase::Malformed => {
                json!({"version":"1.0.0", "repo":"fixture", "git_ref":"fixture", "digest": "sha256:not-a-digest"})
            }
            DigestCase::Tampered => {
                json!({"version":"1.0.0", "repo":"fixture", "git_ref":"fixture", "digest": format!("sha256:{digest}")})
            }
        };
        thread::spawn(move || {
            for _ in 0..5 {
                let (stream, _) = listener.accept().unwrap();
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
    let size = stream.read(&mut request).unwrap();
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
            json!({"files": files}).to_string().into_bytes(),
            "application/json",
        )
    } else if path.contains("/blob?") {
        let file = path.split("path=").nth(1).unwrap_or("");
        let file = file.split('&').next().unwrap_or(file);
        let bytes = files
            .iter()
            .find(|(path, _)| path.as_ref() == file)
            .map(|(_, bytes)| bytes.clone())
            .unwrap();
        (
            200,
            json!({"content": String::from_utf8(bytes).unwrap()})
                .to_string()
                .into_bytes(),
            "application/json",
        )
    } else {
        (
            200,
            serde_json::to_vec(metadata).unwrap(),
            "application/json",
        )
    };
    let header = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).unwrap();
    stream.write_all(&body).unwrap();
}
