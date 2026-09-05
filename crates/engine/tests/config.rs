use engine::config::resolve_handler_path;
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
}

#[test]
fn directory_with_app_subdir_without_index_html_falls_back_to_static() {
    // The legacy "app/ folder exists → PHP mode" convention was removed with
    // the PHPX runtime pocket (RFD 24 §12). A directory is only an app-router
    // project when it has index.html + app/page.ds(x) (see
    // runtime_core::framework::is_app_router_project); anything else without
    // an index file falls back to static directory serving.
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
