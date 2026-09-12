use engine::config::resolve_handler_path;
use runtime_core::dist::{
    ARTIFACT_FORMAT, ArtifactClient, ArtifactCompat, ArtifactManifestV2, ArtifactProducer,
    ArtifactServer, MODULE_FORMAT, RUNTIME_ABI,
};
use std::fs;

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{}_{}", prefix, nonce));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn file_input_routes_to_correct_handler() {
    let dir = temp_dir("engine_test_file");
    let file = dir.join("handler.js");
    fs::write(&file, "export default () => {}").unwrap();
    let resolved = resolve_handler_path(file.to_str().unwrap()).unwrap();
    assert_eq!(
        resolved.path.canonicalize().unwrap(),
        file.canonicalize().unwrap()
    );
    assert!(matches!(resolved.mode, engine::config::ServeMode::Php));
}

#[test]
fn directory_with_serve_entry_routes_to_correct_handler() {
    let dir = temp_dir("engine_test_serve");
    fs::write(dir.join("main.js"), "").unwrap();
    fs::write(dir.join("serve.json"), r#"{"entry":"main.js"}"#).unwrap();
    let resolved = resolve_handler_path(dir.to_str().unwrap()).unwrap();
    assert_eq!(
        resolved.path.canonicalize().unwrap(),
        dir.join("main.js").canonicalize().unwrap()
    );
    assert!(matches!(resolved.mode, engine::config::ServeMode::Php));
}

#[test]
fn directory_with_app_subdir_without_index_html_falls_back_to_static() {
    // The legacy "app/ folder exists → PHP mode" convention was removed with
    // the PHPX runtime pocket (RFD 24 §12). A directory is only an app-router
    // project when it has deka.json + app/page.ds(x) (see
    // runtime_core::dist::is_source_app_router_project — the root
    // index.html is a build output, not a source requirement, deka#762);
    // anything else without an index file falls back to static directory
    // serving.
    let dir = temp_dir("engine_test_app");
    fs::create_dir(dir.join("app")).unwrap();
    fs::write(dir.join("app").join("page.ds"), "").unwrap();
    let resolved = resolve_handler_path(dir.to_str().unwrap()).unwrap();
    assert!(resolved.path.is_dir());
    assert!(matches!(resolved.mode, engine::config::ServeMode::Static));
}

#[test]
fn index_ds_routes_to_correct_handler() {
    let dir = temp_dir("engine_test_index");
    fs::write(dir.join("index.ds"), "").unwrap();
    let resolved = resolve_handler_path(dir.to_str().unwrap()).unwrap();
    assert_eq!(
        resolved.path.canonicalize().unwrap(),
        dir.join("index.ds").canonicalize().unwrap()
    );
}

#[test]
fn index_ds_routes_to_dekascript_handler() {
    let dir = temp_dir("engine_test_dekascript_index");
    fs::write(dir.join("index.ds"), "export const app = 1;").unwrap();

    let resolved = resolve_handler_path(dir.to_str().unwrap()).unwrap();
    assert_eq!(
        resolved.path.canonicalize().unwrap(),
        dir.join("index.ds").canonicalize().unwrap()
    );
    assert!(matches!(resolved.mode, engine::config::ServeMode::Php));
}

#[test]
fn index_js_routes_to_worker_handler() {
    let dir = temp_dir("engine_test_index_js");
    fs::write(
        dir.join("index.js"),
        "export default { async fetch(request) { return new Response(\"ok\"); } }\n",
    )
    .unwrap();
    fs::write(dir.join("index.html"), "<p>static</p>").unwrap();

    let resolved = resolve_handler_path(dir.to_str().unwrap()).unwrap();
    assert_eq!(
        resolved.path.canonicalize().unwrap(),
        dir.join("index.js").canonicalize().unwrap()
    );
    assert!(matches!(resolved.mode, engine::config::ServeMode::Php));
}

#[test]
fn package_json_main_is_ignored_for_handler_resolution() {
    let dir = temp_dir("engine_test_pkg");
    fs::write(dir.join("package.json"), r#"{"main":"lib.js"}"#).unwrap();
    fs::write(dir.join("lib.js"), "").unwrap();
    let resolved = resolve_handler_path(dir.to_str().unwrap()).unwrap();
    assert_eq!(
        resolved.path.canonicalize().unwrap(),
        dir.canonicalize().unwrap()
    );
}

#[test]
fn missing_entry_file_returns_error() {
    let dir = temp_dir("engine_test_missing_entry");
    fs::write(dir.join("serve.json"), r#"{"entry":"nonexistent.js"}"#).unwrap();
    let result = resolve_handler_path(dir.to_str().unwrap());
    if let Err(err) = result {
        assert!(err.contains("Entry file not found"));
    } else {
        panic!("expected error for missing entry file");
    }
}

fn write_artifact(root: &std::path::Path) {
    let dist = root.join("dist");
    fs::create_dir_all(dist.join("server")).unwrap();
    fs::create_dir_all(dist.join("client")).unwrap();
    fs::write(
        dist.join("server/serve-entry.js"),
        "export default { fetch() { return new Response('artifact'); } };\n",
    )
    .unwrap();
    fs::write(dist.join("client/index.html"), "<p>artifact</p>").unwrap();
    let mut manifest = ArtifactManifestV2 {
        format: ARTIFACT_FORMAT.to_string(),
        origin: "authored".to_string(),
        producer: ArtifactProducer {
            deka: "test".to_string(),
            dsc: "test".to_string(),
            plan_version: 2,
        },
        compat: ArtifactCompat {
            runtime_abi: RUNTIME_ABI,
            module_format: MODULE_FORMAT.to_string(),
            targets: vec!["native".to_string()],
            host_imports: Vec::new(),
        },
        client: ArtifactClient {
            root: "client".to_string(),
            index: Some("client/index.html".to_string()),
            trailing_slash: false,
        },
        server: ArtifactServer {
            root: "server".to_string(),
            entries: Vec::new(),
        },
        worker: None,
        routes: Vec::new(),
        slots: Vec::new(),
        payloads: Vec::new(),
        payload_root: String::new(),
    };
    manifest.record_payloads(&dist).unwrap();
    manifest.write_into(&dist).unwrap();
}

#[test]
fn authored_artifact_wins_without_consulting_source_config() {
    let dir = temp_dir("engine_artifact_precedence");
    write_artifact(&dir);
    // If source configuration were consulted before dist/, this malformed
    // source file would only be a distraction. Artifact resolution succeeds.
    fs::write(dir.join("deka.json"), "this is not JSON").unwrap();
    let resolved = resolve_handler_path(dir.to_str().unwrap()).unwrap();
    assert_eq!(
        resolved.path.canonicalize().unwrap(),
        dir.join("dist/server/serve-entry.js")
            .canonicalize()
            .unwrap()
    );
}

#[test]
fn incomplete_authored_dist_is_terminal_with_both_remedies() {
    let dir = temp_dir("engine_incomplete_artifact");
    fs::create_dir_all(dir.join("dist")).unwrap();
    let err = match resolve_handler_path(dir.to_str().unwrap()) {
        Ok(_) => panic!("incomplete dist/ must not fall back to source resolution"),
        Err(err) => err,
    };
    assert!(err.contains("incomplete artifact"), "{err}");
    assert!(err.contains("deka build"), "{err}");
    assert!(err.contains("deka dev"), "{err}");
}
