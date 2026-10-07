// Binary identity in isolate compile failures (deka#1101). dev/serve/build
// surface dsc diagnostics through pool::dsc_compile; the error must name the
// producing dsc and deka binaries so a stale shadowed binary is visible even
// when version-skew detection did not fire.

use std::fs;
use std::os::unix::fs::PermissionsExt;

fn failing_dsc_stub(root: &std::path::Path) -> std::path::PathBuf {
    let stub = root.join("dsc");
    fs::write(
        &stub,
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then echo 'dsc 0.99.0-test'; exit 0; fi\n\
         echo '[transpile] 6:20: app.dsx: expected ``)``, found `newline`' >&2\n\
         exit 1\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();
    stub
}

#[test]
fn transpile_failure_names_the_producing_binaries() {
    let temp = tempfile::tempdir().unwrap();
    let stub = failing_dsc_stub(temp.path());
    let entry = temp.path().join("app.dsx");
    fs::write(&entry, "export fn main() {\n").unwrap();

    let err = pool::dsc_compile::compile_graph_with_dsc(temp.path(), &entry, &stub)
        .expect_err("stub dsc must fail");

    // The marker stays first and dsc's own diagnostic is preserved verbatim
    // (deka#739): identity goes after, never in front.
    assert!(
        err.starts_with(runtime_core::DEKA_VALIDATION_ERROR_MARKER),
        "{err}"
    );
    assert!(
        err.contains("expected ``)``, found `newline`"),
        "dsc's diagnostic must be preserved: {err}"
    );
    assert!(err.contains("dsc 0.99.0-test"), "dsc version: {err}");
    assert!(
        err.contains(stub.to_str().unwrap()),
        "dsc path: {err}"
    );
    assert!(
        err.contains(&format!("deka {}", env!("CARGO_PKG_VERSION"))),
        "deka version: {err}"
    );
    assert!(err.contains(" at "), "deka binary path: {err}");
}

#[test]
fn transpile_failure_identity_survives_island_wrapping() {
    // The exact wrapping dev prints for a client-island compile failure:
    // "failed to compile island <module>: <err>". The identity trailer must
    // still be present inside the wrapped error.
    let temp = tempfile::tempdir().unwrap();
    let stub = failing_dsc_stub(temp.path());
    let entry = temp.path().join("app.dsx");
    fs::write(&entry, "export fn main() {\n").unwrap();

    let err = pool::dsc_compile::compile_graph_with_dsc(temp.path(), &entry, &stub)
        .map_err(|err| format!("failed to compile island {}: {err}", entry.display()))
        .expect_err("stub dsc must fail");
    assert!(err.contains("failed to compile island"), "{err}");
    assert!(err.contains("dsc 0.99.0-test"), "{err}");
    assert!(
        err.contains(&format!("deka {}", env!("CARGO_PKG_VERSION"))),
        "{err}"
    );
}
