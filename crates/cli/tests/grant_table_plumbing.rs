//! deka#797: grant-table plumbing, end to end through the real CLI binary.
//!
//! A fixture server serves the deka.gg registry + R2 CDN shape on 127.0.0.1
//! (via the test-only `DEKA_PM_REGISTRY_URL` / `DEKA_PM_STDLIB_CDN` injection
//! points — not a configuration channel, deka#801), publishing a fixture
//! `@deka/fs` release whose `index.ds` calls `bridge fs.read_file`. The test
//! then runs the real `deka add @deka/fs` and asserts:
//!
//! 1. `deka.grants.json` appears next to `deka.lock`, keyed by the
//!    lockfile-pinned fsGraph digest, granting exactly the catalog kinds the
//!    registry identity owns (`fs` — never anything the package manifest
//!    could ask for).
//! 2. `deka run` executes an app importing that package with **no**
//!    `DEKA_HOST_GRANTS` env var and no programmatic grant config.
//! 3. Removing the grant table, or tampering the lockfile digest, restores
//!    the RFD 27 denial ("not granted any host kinds") — the digest-keyed
//!    lookup stays the trust root.
//! 4. `deka install` re-run against a package whose lock entry no longer
//!    matches the release bytes fails integrity verification instead of
//!    refreshing the grant.
//!
//! Note: this binary links the workspace `core` crate, which shadows the
//! builtin `core` in macro expansions — so no `#[tokio::test]`; the fixture
//! server runs on a manually created multi-thread runtime kept alive for the
//! whole test.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use axum::{Json, Router, extract::State, routing::get};

const FIXTURE_VERSION: &str = "9.9.9-fixture";

/// The fixture release: a bridging `@deka/fs` package exactly like a real
/// stdlib release (manifest + DekaScript sources calling `bridge`).
const FIXTURE_INDEX_DS: &str = "export async fn read_file(p: string) Promise<Result<bytes, string>> {\n  return await bridge fs.read_file(p)\n}\n";

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Write the fixture package tree and pack it the way the stdlib release
/// workflow does (`tar -czf` of the tree contents).
fn pack_fixture_package(work: &Path) -> Vec<u8> {
    let tree = work.join("fixture-tree");
    fs::create_dir_all(&tree).expect("mkdir fixture tree");
    fs::write(
        tree.join("deka.json"),
        serde_json::json!({
            "name": "@deka/fs",
            "version": FIXTURE_VERSION
        })
        .to_string(),
    )
    .expect("write fixture manifest");
    fs::write(tree.join("index.ds"), FIXTURE_INDEX_DS).expect("write fixture module");

    let tarball = work.join("fs-fixture.tgz");
    let status = Command::new("tar")
        .args([
            "-czf",
            tarball.to_str().expect("tarball path"),
            "-C",
            tree.to_str().expect("tree path"),
            ".",
        ])
        .status()
        .expect("run tar");
    assert!(status.success(), "tar must succeed");
    fs::read(&tarball).expect("read fixture tarball")
}

#[derive(Clone)]
struct FixtureState(Arc<Vec<u8>>);

async fn registry_json() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "fs",
        "description": "grant-table plumbing fixture",
        "versions": [FIXTURE_VERSION],
    }))
}

async fn tarball_bytes(State(state): State<FixtureState>) -> Vec<u8> {
    state.0.as_ref().clone()
}

/// The fixture registry + CDN, alive for as long as this value is held.
struct FixtureRegistry {
    address: SocketAddr,
    _runtime: tokio::runtime::Runtime,
}

impl FixtureRegistry {
    fn start(tarball: Vec<u8>) -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        let address = runtime.block_on(async {
            let app = Router::new()
                .route("/api/registry/fs.json", get(registry_json))
                .route("/:name/:version/:file", get(tarball_bytes))
                .with_state(FixtureState(Arc::new(tarball)));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture registry listener");
            let address = listener.local_addr().expect("fixture registry address");
            tokio::spawn(async move {
                axum::serve(listener, app).await.expect("fixture registry");
            });
            address
        });
        Self {
            address,
            _runtime: runtime,
        }
    }
}

fn write_app_project(project: &Path) {
    fs::create_dir_all(project.join("data")).expect("mkdir data");
    fs::write(project.join("data").join("secret.txt"), "hello grants").expect("write data");
    fs::write(
        project.join("deka.json"),
        serde_json::json!({
            "name": "grant-plumbing-e2e",
            "security": { "allow": { "read": ["./data"] }, "prompt": false }
        })
        .to_string(),
    )
    .expect("write app manifest");
    fs::write(
        project.join("main.ds"),
        "import { read_file } from \"@deka/fs\"\n\nexport async fn app() Promise<string> {\n  const r = await read_file(\"data/secret.txt\")\n  return match r {\n    Ok(bytes) => \"read-ok\"\n    Err(e) => \"read-denied\"\n  }\n}\n",
    )
    .expect("write app entry");
}

fn run_cli(
    project: &Path,
    args: &[&str],
    registry: SocketAddr,
    env_grants: Option<&str>,
) -> (bool, String) {
    let mut command = Command::new(cli_bin());
    command
        .args(args)
        .current_dir(project)
        .env("DEKA_PM_REGISTRY_URL", format!("http://{registry}"))
        .env("DEKA_PM_STDLIB_CDN", format!("http://{registry}"));
    // The acceptance criterion: grants come from the project files, never
    // from the environment. Passing Some(env_grants) would exercise the
    // documented override; this test never does.
    match env_grants {
        Some(value) => {
            command.env("DEKA_HOST_GRANTS", value);
        }
        None => {
            command.env_remove("DEKA_HOST_GRANTS");
        }
    }
    // The CLI resolves dsc from DEKA_DSC / beside the binary / PATH; point
    // the subprocess at the same compiler this test binary found.
    if let Some(dsc) = dsc_available() {
        command.env("DEKA_DSC", dsc);
    }
    let output = command.output().expect("run deka");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn lock_fsgraph_digest(project: &Path) -> String {
    let lock: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project.join("deka.lock")).expect("read deka.lock"),
    )
    .expect("parse deka.lock");
    lock["packages"]["@deka/fs"][2]["fsGraph"]["hash"]
        .as_str()
        .expect("lockfile fsGraph hash")
        .to_string()
}

/// `deka run` compiles DekaScript through the pinned dsc; skip the run-phase
/// assertions (never the install assertions) when no compiler is available.
fn dsc_available() -> Option<PathBuf> {
    if std::env::var_os("DEKA_NO_DSC").is_some() {
        return None;
    }
    if let Ok(path) = std::env::var("DEKA_DSC")
        && Path::new(&path).is_file()
    {
        return Some(PathBuf::from(path));
    }
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/dsc");
    if local.is_file() {
        return Some(local);
    }
    None
}

#[test]
fn deka_add_delivers_grants_and_the_runtime_uses_them() {
    let work = tempfile::tempdir().expect("work dir");
    let tarball = pack_fixture_package(work.path());
    let registry = FixtureRegistry::start(tarball);
    let project = work.path().join("app");
    write_app_project(&project);

    // 1. The real install path against a registry-shaped fixture.
    let (success, output) = run_cli(&project, &["add", "@deka/fs"], registry.address, None);
    assert!(success, "deka add @deka/fs must succeed: {output}");

    // The dependency is recorded exactly like a real add.
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project.join("deka.json")).expect("read manifest"),
    )
    .expect("parse manifest");
    assert_eq!(
        manifest["dependencies"]["@deka/fs"].as_str(),
        Some(FIXTURE_VERSION)
    );

    // 2. The grant table: one record keyed by the lockfile-pinned fsGraph
    // digest, kinds from the catalog's grant-owner binding for the registry
    // identity — not from anything the package could declare.
    let digest = lock_fsgraph_digest(&project);
    let table: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project.join("deka.grants.json")).expect("deka.grants.json must exist"),
    )
    .expect("parse grant table");
    let grants = table.as_array().expect("grant table array");
    assert_eq!(grants.len(), 1, "exactly one grant expected: {table}");
    assert_eq!(grants[0]["name"].as_str(), Some("@deka/fs"));
    assert_eq!(grants[0]["version"].as_str(), Some(FIXTURE_VERSION));
    assert_eq!(grants[0]["digest"].as_str(), Some(digest.as_str()));
    assert_eq!(
        grants[0]["kinds"].as_array().expect("kinds array"),
        &vec![serde_json::json!("fs")]
    );

    let Some(_dsc) = dsc_available() else {
        println!("SKIP run-phase assertions: dsc unavailable");
        return;
    };

    // 3. No env var, no programmatic config: the app whose dependency calls
    // bridge fs.read_file boots and runs off the project-installed table.
    let (success, output) = run_cli(&project, &["run", "main.ds"], registry.address, None);
    assert!(
        success,
        "deka run must succeed with the project-installed grant table: {output}"
    );

    // 4a. Remove the grant table: the RFD 27 load-time denial is back.
    fs::rename(
        project.join("deka.grants.json"),
        project.join("deka.grants.json.bak"),
    )
    .expect("move grant table away");
    let (success, output) = run_cli(&project, &["run", "main.ds"], registry.address, None);
    assert!(!success, "deka run must fail without any grant table: {output}");
    assert!(
        output.contains("not granted any host kinds"),
        "expected the RFD 27 no-grants load error: {output}"
    );
    fs::rename(
        project.join("deka.grants.json.bak"),
        project.join("deka.grants.json"),
    )
    .expect("restore grant table");

    // 4b. Tamper the lockfile digest: the grant is keyed to the true digest,
    // so the lookup must miss even though the table is present.
    let lock_path = project.join("deka.lock");
    let lock_text = fs::read_to_string(&lock_path).expect("read lock");
    let tampered = lock_text.replacen(&digest, "sha256:0000000000000000", 1);
    assert_ne!(lock_text, tampered, "digest must appear in the lockfile");
    fs::write(&lock_path, tampered).expect("write tampered lock");
    let (success, output) = run_cli(&project, &["run", "main.ds"], registry.address, None);
    assert!(
        !success,
        "deka run must fail when the lockfile digest is tampered: {output}"
    );
    // Two gates deny this state, either is a pass: dsc's module-graph
    // integrity check rejects the lockfile mismatch before boot, and the
    // loader's digest-keyed grant lookup would miss anyway.
    assert!(
        output.contains("not granted any host kinds")
            || output.contains("failed integrity check"),
        "a grant for another digest must unlock nothing: {output}"
    );

    // 4c. The installer itself refuses to bless a tampered lock entry:
    // re-running install fails integrity verification instead of deriving a
    // grant for bytes the lock no longer describes.
    let (success, output) = run_cli(&project, &["install"], registry.address, None);
    assert!(
        !success,
        "deka install must reject the tampered lock entry: {output}"
    );
    assert!(
        output.contains("fsGraph hash mismatch"),
        "expected integrity verification failure: {output}"
    );
}
