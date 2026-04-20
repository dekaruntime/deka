/// Issue #8 — Verify that the default storefront handler compiles to valid JS.
///
/// Reads `tana/store/default/main.phpx` through the PHPX transpiler and asserts
/// that the output is non-empty valid JavaScript (no compilation errors).
///
/// The storefront imports `@tana/store` and `@deka/*` stdlib modules. Both live in
/// `tana/store/default/php_modules/`. We symlink that directory (and its deka.lock)
/// into a temp project so module resolution finds them.

use phpx_js::{compile_phpx_source_to_js, SourceModuleMeta};
use std::time::{SystemTime, UNIX_EPOCH};

/// Path to the default storefront directory (contains main.phpx, php_modules/, deka.lock).
const STOREFRONT_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../",
    "../../tana/store/default"
);

#[test]
fn storefront_handler_compiles() {
    let storefront_dir = std::fs::canonicalize(STOREFRONT_DIR)
        .unwrap_or_else(|e| panic!("cannot find storefront dir at {}: {}", STOREFRONT_DIR, e));

    let storefront_phpx = storefront_dir.join("main.phpx");
    let source = std::fs::read_to_string(&storefront_phpx)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", storefront_phpx.display(), e));

    assert!(!source.is_empty(), "storefront handler is empty");

    // Create a temp project directory with the storefront source and symlinks
    // to the storefront's own php_modules/ (which contains @tana/store and @deka/*)
    // and deka.lock so module resolution can find all imported modules.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tmp = std::env::temp_dir().join(format!("deka_storefront_compile_test_{}", nanos));
    std::fs::create_dir_all(&tmp).expect("create temp dir");

    // Symlink the storefront's php_modules and deka.lock (not the runtime root's)
    // because the storefront ships its own @tana/store and @deka/* packages.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            storefront_dir.join("php_modules"),
            tmp.join("php_modules"),
        )
        .expect("symlink php_modules");
        std::os::unix::fs::symlink(
            storefront_dir.join("deka.lock"),
            tmp.join("deka.lock"),
        )
        .expect("symlink deka.lock");
    }
    #[cfg(not(unix))]
    {
        // On non-unix, just use the storefront dir directly (see entry path below)
        let _ = std::fs::copy(storefront_dir.join("deka.lock"), tmp.join("deka.lock"));
    }

    // Write the storefront source into the temp project
    let entry = tmp.join("main.phpx");
    std::fs::write(&entry, &source).expect("write storefront source");

    let entry_str = entry.to_str().expect("valid utf-8 path");
    let meta = SourceModuleMeta::empty();
    let result = compile_phpx_source_to_js(&source, entry_str, meta);

    // Clean up before asserting
    let _ = std::fs::remove_dir_all(&tmp);

    let js = result.unwrap_or_else(|err| panic!("storefront compilation failed:\n{}", err));

    assert!(
        !js.is_empty(),
        "storefront compiled to empty JS output"
    );

    // The compiled output must contain a function — the storefront defines `App`.
    assert!(
        js.contains("function") || js.contains("=>"),
        "compiled JS does not contain any function definitions:\n{}",
        &js[..js.len().min(500)]
    );
}
