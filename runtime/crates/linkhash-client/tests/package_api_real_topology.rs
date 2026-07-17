use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use linkhash_client::{LinkhashClient, PublishRequest};
use serde_json::json;

#[test]
#[ignore = "real-topology test starts the deka-git server binary"]
fn real_topology_scoped_package_api_crosses_http_boundary() {
    let topology = Topology::start();
    let client = LinkhashClient::new(&topology.base_url, Some(&topology.token));

    let publish_010 = publish_request("0.1.0");
    let preflight = client.preflight(&publish_010).expect("preflight 0.1.0");
    assert!(preflight.allowed);
    assert_eq!(preflight.required_bump.as_deref(), Some("initial"));
    assert_eq!(preflight.minimum_allowed_version.as_deref(), Some("0.1.0"));

    let published = client.publish(&publish_010).expect("publish 0.1.0");
    assert_eq!(published.package_name, "@tana/store");
    assert_eq!(published.version, "0.1.0");

    topology.update_fixture_repo_to_020();
    let publish_020 = publish_request("0.2.0");
    let preflight_020 = client.preflight(&publish_020).expect("preflight 0.2.0");
    assert!(preflight_020.allowed);
    assert_eq!(preflight_020.required_bump.as_deref(), Some("minor"));
    let published_020 = client.publish(&publish_020).expect("publish 0.2.0");
    assert_eq!(published_020.package_name, "@tana/store");
    assert_eq!(published_020.version, "0.2.0");

    let exact = client
        .resolve("@tana/store", "0.1.0")
        .expect("resolve exact");
    assert_eq!(exact.version, "0.1.0");
    assert_eq!(exact.repo.as_deref(), Some("store"));
    assert_eq!(exact.git_ref.as_deref(), Some("v0.1.0"));

    let ranged = client
        .resolve("@tana/store", "^0.1.0")
        .expect("resolve range");
    assert_eq!(ranged.version, "0.1.0");

    let latest = client
        .resolve("@tana/store", "latest")
        .expect("resolve latest");
    assert_eq!(latest.version, "0.2.0");

    let versions = client.list_versions("@tana/store").expect("list versions");
    assert!(versions.contains(&"0.1.0".to_string()));
    assert!(versions.contains(&"0.2.0".to_string()));

    let download_dir = topology.temp.path().join("downloaded-store");
    client
        .download("@tana/store", "0.2.0", &download_dir)
        .expect("download tree/blob");
    assert_eq!(
        fs::read_to_string(download_dir.join("index.phpx")).expect("downloaded index.phpx"),
        index_source_020()
    );
    assert_eq!(
        fs::read_to_string(download_dir.join("deka.json")).expect("downloaded deka.json"),
        manifest_source("0.2.0")
    );
}

struct Topology {
    temp: tempfile::TempDir,
    fixture_repo: PathBuf,
    base_url: String,
    token: String,
    server: Child,
}

impl Topology {
    fn start() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let repos_dir = temp.path().join("repos");
        let db_path = temp.path().join("db").join("linkhash.sqlite");
        let store_path = temp.path().join("store.zega");
        let fixture_repo = temp.path().join("fixture-store");
        let bare_repo = repos_dir.join("tana").join("store.git");
        fs::create_dir_all(bare_repo.parent().expect("bare repo parent")).expect("repos parent");
        seed_fixture_repo(&fixture_repo, &bare_repo);

        let port = free_port();
        let base_url = format!("http://127.0.0.1:{port}");
        let server_log = temp.path().join("deka-git.log");
        let log = fs::File::create(&server_log).expect("server log");
        let server = Command::new(deka_git_binary())
            .env("LINKHASH_PORT", port.to_string())
            .env("LINKHASH_REPOS_DIR", &repos_dir)
            .env("LINKHASH_DB_PATH", &db_path)
            .env("LINKHASH_STORE_PATH", &store_path)
            .stdout(Stdio::from(log.try_clone().expect("clone log")))
            .stderr(Stdio::from(log))
            .spawn()
            .expect("start deka-git server process");

        wait_for_server(&base_url, &server_log);
        let token = mint_token(&base_url);

        Self {
            temp,
            fixture_repo,
            base_url,
            token,
            server,
        }
    }

    fn update_fixture_repo_to_020(&self) {
        fs::write(
            self.fixture_repo.join("deka.json"),
            manifest_source("0.2.0"),
        )
        .expect("write deka.json 0.2.0");
        fs::write(self.fixture_repo.join("index.phpx"), index_source_020())
            .expect("write index.phpx 0.2.0");
        git(&self.fixture_repo, ["add", "deka.json", "index.phpx"]);
        git(&self.fixture_repo, ["commit", "-m", "release 0.2.0"]);
        git(&self.fixture_repo, ["tag", "v0.2.0"]);
        git(&self.fixture_repo, ["push", "origin", "main"]);
        git(&self.fixture_repo, ["push", "origin", "v0.2.0"]);
    }
}

impl Drop for Topology {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

fn publish_request(version: &str) -> PublishRequest {
    PublishRequest {
        name: "@tana/store".to_string(),
        version: version.to_string(),
        repo: "store".to_string(),
        git_ref: format!("v{version}"),
        description: Some("real topology package API fixture".to_string()),
        manifest: None,
    }
}

fn seed_fixture_repo(repo: &Path, bare_repo: &Path) {
    fs::create_dir_all(repo).expect("fixture repo dir");
    fs::create_dir_all(bare_repo.parent().expect("bare repo parent")).expect("bare repo parent");
    let output = Command::new("git")
        .args([
            "init",
            "--bare",
            bare_repo.to_str().expect("bare repo path"),
        ])
        .output()
        .expect("init bare repo");
    assert!(
        output.status.success(),
        "git init --bare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    git(repo, ["init", "--initial-branch=main"]);
    git(repo, ["config", "user.email", "samira@tana.test"]);
    git(repo, ["config", "user.name", "Samira"]);
    fs::write(repo.join("deka.json"), manifest_source("0.1.0")).expect("write deka.json");
    fs::write(repo.join("index.phpx"), index_source_010()).expect("write index.phpx");
    fs::write(repo.join("README.md"), "# Store fixture\n").expect("write README");
    git(repo, ["add", "deka.json", "index.phpx", "README.md"]);
    git(repo, ["commit", "-m", "release 0.1.0"]);
    git(repo, ["tag", "v0.1.0"]);
    git(
        repo,
        [
            "remote",
            "add",
            "origin",
            bare_repo.to_str().expect("bare repo path"),
        ],
    );
    git(repo, ["push", "origin", "main"]);
    git(repo, ["push", "origin", "v0.1.0"]);
}

fn git<const N: usize>(dir: &Path, args: [&str; N]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git failed in {}: {}\nstdout:\n{}\nstderr:\n{}",
        dir.display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn mint_token(base_url: &str) -> String {
    let body: serde_json::Value = reqwest::blocking::Client::new()
        .post(format!("{base_url}/api/tokens"))
        .json(&json!({
            "key_type": "agent",
            "owner": "tana",
            "scopes": ["packages:read", "packages:write"],
            "repos": ["*"]
        }))
        .send()
        .expect("create token request")
        .error_for_status()
        .expect("create token success")
        .json()
        .expect("create token json");
    body["token"]
        .as_str()
        .expect("token response has token")
        .to_string()
}

fn wait_for_server(base_url: &str, log_path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if reqwest::blocking::get(format!("{base_url}/health"))
            .is_ok_and(|response| response.status().is_success())
        {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "deka-git server did not become healthy; log:\n{}",
        fs::read_to_string(log_path).unwrap_or_else(|_| "<missing log>".to_string())
    );
}

fn deka_git_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("LINKHASH_DEKA_GIT_BIN") {
        return PathBuf::from(path);
    }
    workspace_root().join("target/debug/deka-git")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn manifest_source(version: &str) -> String {
    format!(
        "{{\n  \"name\": \"@tana/store\",\n  \"version\": \"{version}\",\n  \"deka.security\": {{ \"allow\": {{}} }}\n}}\n"
    )
}

fn index_source_010() -> &'static str {
    "/// Store title.\nexport function title(): string {\n  return \"store\";\n}\n"
}

fn index_source_020() -> &'static str {
    "/// Store title.\nexport function title(): string {\n  return \"store\";\n}\n\n/// Store version.\nexport function version(): string {\n  return \"0.2.0\";\n}\n"
}
