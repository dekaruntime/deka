/// Issue #8 — Verify that the default storefront handler compiles to valid JS.
///
/// Reads a vendored, self-contained storefront fixture at
/// `tests/fixtures/storefront/main.phpx` through the PHPX transpiler and
/// asserts that the output is non-empty valid JavaScript (no compilation
/// errors).
///
/// This fixture is a copy of `tana/store/default/main.phpx`, kept small and
/// vendored *inside this repository* (see #52) so the test has no dependency
/// on a sibling `tana` checkout. `compile_phpx_source_to_js` only validates
/// imports lexically (see `modules_php::validation::imports`) — it does not
/// resolve them against `php_modules/` on disk — so a vendored copy of the
/// storefront source is sufficient to exercise the real compilation path
/// without needing the real `@tana/store` package installed.
use deka_js::{SourceModuleMeta, compile_phpx_source_to_js};
use std::time::{SystemTime, UNIX_EPOCH};

/// Path to the vendored storefront fixture directory (contains main.phpx,
/// php_modules/, deka.lock). Lives inside this crate — no sibling repo needed.
const STOREFRONT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/storefront");

#[test]
fn storefront_handler_compiles() {
    let storefront_dir = std::fs::canonicalize(STOREFRONT_DIR).unwrap_or_else(|e| {
        panic!(
            "cannot find vendored storefront fixture at {}: {} (see issue #52 — this fixture \
             should be vendored inside the repo, not resolved from a sibling checkout)",
            STOREFRONT_DIR, e
        )
    });

    let storefront_phpx = storefront_dir.join("main.phpx");
    let source = std::fs::read_to_string(&storefront_phpx)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", storefront_phpx.display(), e));

    assert!(!source.is_empty(), "storefront handler is empty");

    // Create a temp project directory with the storefront source and symlinks
    // to the fixture's own php_modules/ and deka.lock. compile_phpx_source_to_js
    // does not actually read these (import resolution is lexical-only), but we
    // keep the same on-disk shape as a real storefront checkout for realism.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tmp = std::env::temp_dir().join(format!("deka_storefront_compile_test_{}", nanos));
    std::fs::create_dir_all(&tmp).expect("create temp dir");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(storefront_dir.join("php_modules"), tmp.join("php_modules"))
            .expect("symlink php_modules");
        std::os::unix::fs::symlink(storefront_dir.join("deka.lock"), tmp.join("deka.lock"))
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

    assert!(!js.is_empty(), "storefront compiled to empty JS output");

    // The compiled output must contain a function — the storefront defines `App`.
    assert!(
        js.contains("function") || js.contains("=>"),
        "compiled JS does not contain any function definitions:\n{}",
        &js[..js.len().min(500)]
    );
}
