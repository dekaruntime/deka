use std::fs;
use engine::config::resolve_handler_path;

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
    assert_eq!(resolved.path, dir.join("main.js"));
}

#[test]
fn directory_with_app_subdir_routes_to_php_mode() {
    let dir = temp_dir("engine_test_app");
    fs::create_dir(dir.join("app")).unwrap();
    fs::write(dir.join("app").join("page.phpx"), "").unwrap();
    let resolved = resolve_handler_path(dir.to_str().unwrap()).unwrap();
    assert!(resolved.path.is_dir());
    assert!(matches!(resolved.mode, engine::config::ServeMode::Php));
}

#[test]
fn index_phpx_routes_to_correct_handler() {
    let dir = temp_dir("engine_test_index");
    fs::write(dir.join("index.phpx"), "").unwrap();
    let resolved = resolve_handler_path(dir.to_str().unwrap()).unwrap();
    assert_eq!(resolved.path, dir.join("index.phpx"));
}

#[test]
fn package_json_main_routes_to_correct_handler() {
    let dir = temp_dir("engine_test_pkg");
    fs::write(dir.join("package.json"), r#"{"main":"lib.js"}"#).unwrap();
    fs::write(dir.join("lib.js"), "").unwrap();
    let resolved = resolve_handler_path(dir.to_str().unwrap()).unwrap();
    assert_eq!(resolved.path, dir.join("lib.js"));
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
