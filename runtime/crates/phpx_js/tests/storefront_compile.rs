/// Issue #8 — Verify that the default storefront handler compiles to valid JS.
///
/// Reads `tana/store/default/main.phpx` through the PHPX transpiler and asserts
/// that the output is non-empty valid JavaScript (no compilation errors).
///
/// The storefront imports stdlib modules (crypto, time, etc.) that only exist in
/// the runtime's full `php_modules/` directory. To make module resolution work,
/// we create a temp project with the storefront source and symlink to the
/// runtime's php_modules/ and deka.lock.

use phpx_js::{compile_phpx_source_to_js, SourceModuleMeta};
use std::time::{SystemTime, UNIX_EPOCH};

/// Path to the real storefront handler.
const STOREFRONT_PHPX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../",
    "../../tana/store/default/main.phpx"
);

/// Runtime root that contains the full php_modules with all stdlib modules.
const RUNTIME_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

#[test]
fn storefront_handler_compiles() {
    let abs = std::fs::canonicalize(STOREFRONT_PHPX)
        .unwrap_or_else(|e| panic!("cannot find storefront handler at {}: {}", STOREFRONT_PHPX, e));
    let runtime_root = std::fs::canonicalize(RUNTIME_ROOT)
        .unwrap_or_else(|e| panic!("cannot find runtime root at {}: {}", RUNTIME_ROOT, e));

    let source = std::fs::read_to_string(&abs)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", abs.display(), e));

    assert!(!source.is_empty(), "storefront handler is empty");

    // Create a temp project directory with the storefront source and symlinks
    // to the runtime's full php_modules/ and deka.lock so module resolution
    // can find all imported stdlib modules (crypto, time, neo4j, etc.).
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tmp = std::env::temp_dir().join(format!("deka_storefront_compile_test_{}", nanos));
    std::fs::create_dir_all(&tmp).expect("create temp dir");

    // Symlink php_modules and deka.lock from the runtime root
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            runtime_root.join("php_modules"),
            tmp.join("php_modules"),
        )
        .expect("symlink php_modules");
        std::os::unix::fs::symlink(
            runtime_root.join("deka.lock"),
            tmp.join("deka.lock"),
        )
        .expect("symlink deka.lock");
    }
    #[cfg(not(unix))]
    {
        // On non-unix, copy instead of symlink
        let _ = std::fs::copy(runtime_root.join("deka.lock"), tmp.join("deka.lock"));
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
