//! Test-only fixture registries for pm's install tests (deka#933).
//!
//! Hermetic stand-ins for the deka.gg registry + R2 tarball CDN on
//! 127.0.0.1, so install tests exercise the real fetch/extract/validate
//! path with no external network. Compiled only under `cfg(test)`.
//! `crates/cli/tests/install_ds_modules_e2e.rs` plays the same shape for
//! child-process CLI tests; this module is the in-process counterpart.

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    routing::get,
};
use deka_host::integrity::compute_package_integrity;
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Mutex, OnceLock},
};

/// Manifest for a fixture package tarball (name/version only, matching
/// the shape release tarballs carry).
pub(crate) fn fixture_manifest(name: &str, version: &str) -> String {
    json!({ "name": name, "version": version }).to_string()
}

/// Hermetic stand-in for the deka.gg registry + R2 tarball CDN on
/// 127.0.0.1 (deka#933). Serves the exact request shape
/// `registry::fetch_package` / `install_from_registry` issue against the
/// live endpoints — `/api/registry/<name>.json` metadata and
/// `/<name>/<version>/<file>` tarballs — from in-memory fixture bytes, so
/// the real install path runs with no external network. Every request is
/// recorded so tests can assert the installer actually exercised the
/// fixture instead of silently passing with dead registry code.
pub(crate) struct DekaGgFixture {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    _runtime: tokio::runtime::Runtime,
}

#[derive(Clone)]
struct DekaGgFixtureState {
    packages: Arc<BTreeMap<String, (Vec<String>, Vec<u8>)>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl DekaGgFixture {
    pub(crate) fn start(packages: BTreeMap<String, (Vec<String>, Vec<u8>)>) -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("fixture runtime");
        let state = DekaGgFixtureState {
            packages: Arc::new(packages),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let requests = state.requests.clone();
        let url = runtime.block_on(async move {
            let app = axum::Router::new()
                .route("/api/registry/:name", get(fixture_registry_metadata))
                .route("/:name/:version/:file", get(fixture_registry_tarball))
                .with_state(state);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("fixture listener");
            let address = listener.local_addr().expect("fixture address");
            tokio::spawn(async move {
                axum::serve(listener, app).await.expect("fixture registry");
            });
            format!("http://{address}")
        });
        Self {
            url,
            requests,
            _runtime: runtime,
        }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("fixture requests").clone()
    }
}

async fn fixture_registry_metadata(
    AxumPath(name): AxumPath<String>,
    State(state): State<DekaGgFixtureState>,
) -> Json<serde_json::Value> {
    let name = name.trim_end_matches(".json");
    state
        .requests
        .lock()
        .expect("fixture requests")
        .push(format!("registry {name}"));
    let versions = state
        .packages
        .get(name)
        .map(|(versions, _)| versions.clone())
        .unwrap_or_else(|| vec!["0.0.0-missing".to_string()]);
    Json(json!({ "name": name, "versions": versions }))
}

async fn fixture_registry_tarball(
    AxumPath((name, _version, _file)): AxumPath<(String, String, String)>,
    State(state): State<DekaGgFixtureState>,
) -> Vec<u8> {
    state
        .requests
        .lock()
        .expect("fixture requests")
        .push(format!("tarball {name}"));
    state
        .packages
        .get(&name)
        .map(|(_, bytes)| bytes.clone())
        .unwrap_or_default()
}

/// Pack fixture package bytes the way the real release workflow does
/// (`tar -czf` over the package tree), in memory.
pub(crate) fn pack_fixture_tarball(files: &[(&str, String)]) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (path, content) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, content.as_bytes())
            .expect("append fixture file");
    }
    builder
        .into_inner()
        .expect("finish fixture tar archive")
        .finish()
        .expect("finish fixture tar gzip")
}

/// Serializes in-process mutations of the test-only registry override env
/// vars (process-global state; same pattern as `auth_store_atomicity.rs`).
static REGISTRY_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn registry_env_lock() -> std::sync::MutexGuard<'static, ()> {
    REGISTRY_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap()
}

/// Points `DEKA_PM_REGISTRY_URL` / `DEKA_PM_STDLIB_CDN` (the test-only
/// injection points in `crate::registry`, deka#801) at the fixture server
/// for the current process and restores the previous environment on drop.
/// Must be created while holding `registry_env_lock`.
pub(crate) struct RegistryEnvGuard {
    previous: Vec<(&'static str, Option<String>)>,
}

impl RegistryEnvGuard {
    pub(crate) fn point_at(url: &str) -> Self {
        let vars = ["DEKA_PM_REGISTRY_URL", "DEKA_PM_STDLIB_CDN"];
        let previous = vars
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
        // SAFETY: env mutation is serialized by `registry_env_lock`, held
        // by the caller for this guard's lifetime, and every other test in
        // this binary that reads these vars does so under the same lock.
        unsafe {
            std::env::set_var("DEKA_PM_REGISTRY_URL", url);
            std::env::set_var("DEKA_PM_STDLIB_CDN", url);
        }
        Self { previous }
    }
}

impl Drop for RegistryEnvGuard {
    fn drop(&mut self) {
        // SAFETY: still under the caller's `registry_env_lock`.
        unsafe {
            for (key, value) in self.previous.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

/// Legacy linkhash-registry fixture, kept for the `#[ignore]`d legacy
/// install-graph tests in `install.rs` (deka#330).
pub(crate) async fn fixture_registry(
    packages: BTreeMap<String, serde_json::Value>,
) -> (String, tokio::sync::oneshot::Sender<()>) {
    use axum::{Router, extract::Query};
    use std::collections::HashMap;
    #[derive(Clone)]
    struct Fixture(Arc<BTreeMap<String, serde_json::Value>>);
    async fn versions(
        AxumPath((_scope, package)): AxumPath<(String, String)>,
    ) -> Json<serde_json::Value> {
        let versions = if package == "c" {
            vec!["1.1.0", "1.3.0", "1.4.0", "2.0.0"]
        } else {
            vec!["1.0.0"]
        };
        Json(json!({ "versions": versions }))
    }
    async fn resolve(
        AxumPath((scope, package, version)): AxumPath<(String, String, String)>,
        State(Fixture(packages)): State<Fixture>,
    ) -> Json<serde_json::Value> {
        let name = format!("@{scope}/{package}");
        let dependencies = packages.get(&name).cloned().unwrap_or_else(|| json!({}));
        let digest = fixture_package_digest(&name, &version, &dependencies);
        Json(json!({
            "version": version,
            "repo": "fixture",
            "git_ref": "fixture",
            "digest": format!("sha256:{digest}"),
        }))
    }
    async fn tree(
        AxumPath((scope, package, _version)): AxumPath<(String, String, String)>,
        State(Fixture(packages)): State<Fixture>,
    ) -> Json<serde_json::Value> {
        let name = format!("@{scope}/{package}");
        assert!(
            packages.contains_key(&name),
            "unknown fixture package {name}"
        );
        Json(json!({ "files": [{ "path": "deka.json" }, { "path": "index.phpx" }] }))
    }
    async fn blob(
        AxumPath((scope, package, version)): AxumPath<(String, String, String)>,
        Query(query): Query<HashMap<String, String>>,
        State(Fixture(packages)): State<Fixture>,
    ) -> Json<serde_json::Value> {
        let name = format!("@{scope}/{package}");
        let content = if query.get("path").is_some_and(|path| path == "deka.json") {
            json!({ "name": name, "version": version, "dependencies": packages[&name] }).to_string()
        } else {
            "export const fixture = true;\n".to_string()
        };
        Json(json!({ "content": content }))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("registry listener");
    let address = listener.local_addr().expect("registry address");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let app = Router::new()
        .route(
            "/api/scoped-packages/:scope/:package/versions",
            get(versions),
        )
        .route(
            "/api/scoped-packages/:scope/:package/:version",
            get(resolve),
        )
        .route(
            "/api/scoped-packages/:scope/:package/:version/tree",
            get(tree),
        )
        .route(
            "/api/scoped-packages/:scope/:package/:version/blob",
            get(blob),
        )
        .with_state(Fixture(Arc::new(packages)));
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("fixture registry");
    });
    (format!("http://{address}"), shutdown_tx)
}

pub(crate) fn fixture_package_digest(
    name: &str,
    version: &str,
    dependencies: &serde_json::Value,
) -> String {
    let root = tempfile::tempdir().expect("digest fixture tempdir");
    fs::write(
        root.path().join("deka.json"),
        json!({ "name": name, "version": version, "dependencies": dependencies }).to_string(),
    )
    .expect("digest fixture manifest");
    fs::write(
        root.path().join("index.phpx"),
        "export const fixture = true;\n",
    )
    .expect("digest fixture module");
    compute_package_integrity(root.path())
        .expect("digest fixture integrity")
        .fs_graph
}
