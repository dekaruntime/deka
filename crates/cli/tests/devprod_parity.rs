//! Dev/prod compiler module-boundary parity after framework extraction (#893).
//! Compile the generated serve entry through the same graph compiler used by
//! the dev loader, then compare it with the real build's server modules.
//! Rendering/prerendering belongs to the extracted framework; entry generation,
//! import edges, module boundaries and graph shaking still belong here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

const ORPHAN_MARKER: &str = "orphan-body-marker";
const UNUSED_MARKER: &str = "unused-body-marker";

fn test_dsc() -> PathBuf {
    // CI installs the pinned compiler in .ci/dsc and exports DEKA_DSC.
    // runtime_core intentionally ignores the environment: compiler selection
    // belongs to the caller, just as it does at the CLI build boundary.
    let dsc = match std::env::var_os("DEKA_DSC") {
        Some(path) => PathBuf::from(path),
        None => runtime_core::dsc::find_dsc()
            .expect("resolve sibling/repository dsc")
            .expect("parity tests require dsc: set DEKA_DSC to the pinned compiler"),
    };
    assert!(
        dsc.is_file(),
        "parity compiler is not a file: {}",
        dsc.display()
    );
    dsc.canonicalize()
        .unwrap_or_else(|error| panic!("invalid parity compiler {}: {error}", dsc.display()))
}

#[path = "support/app_router.rs"]
mod app_router;
use app_router::init_project;

/// Scaffolds the multi-module source app-router fixture into `dir`.
fn write_fixture(dir: &Path) {
    init_project(dir);

    // Shared non-route modules. shared.ds imports leaf.ds; unused.ds is
    // never imported; orphan.ds is imported by page.dsx but never used, so
    // graph shaking must drop it.
    fs::create_dir_all(dir.join("lib")).expect("mkdir lib");
    fs::write(
        dir.join("lib").join("leaf.ds"),
        "export fn leaf_marker() string {\n    return \"leaf-body-marker\";\n}\n",
    )
    .expect("write leaf.ds");
    fs::write(
        dir.join("lib").join("shared.ds"),
        "import { leaf_marker } from \"./leaf.ds\"\nexport fn shared_message() string {\n    return \"shared-body-marker \" + leaf_marker();\n}\n",
    )
    .expect("write shared.ds");
    fs::write(
        dir.join("lib").join("unused.ds"),
        "export fn unused_secret() string {\n    return \"unused-body-marker\";\n}\n",
    )
    .expect("write unused.ds");
    fs::write(
        dir.join("lib").join("orphan.ds"),
        "export fn orphan_secret() string {\n    return \"orphan-body-marker\";\n}\n",
    )
    .expect("write orphan.ds");

    // Both routes import the shared module; the home page also imports (but
    // never uses) the orphan module.
    fs::write(
        dir.join("app").join("page.dsx"),
        "import { shared_message } from \"../lib/shared.ds\"\nimport { orphan_secret } from \"../lib/orphan.ds\"\nexport fn Page() {\n    return <main><h1>home {shared_message()}</h1></main>;\n}\n",
    )
    .expect("write app/page.dsx");
    let about = dir.join("app").join("about");
    fs::create_dir_all(&about).expect("mkdir app/about");
    fs::write(
        about.join("page.dsx"),
        "import { shared_message } from \"../../lib/shared.ds\"\nexport fn Page() {\n    return <main><h1>about {shared_message()}</h1></main>;\n}\n",
    )
    .expect("write app/about/page.dsx");
}

fn run_build(dir: &Path, dsc: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("build")
        .env("DEKA_DSC", dsc)
        .current_dir(dir)
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn serve_entry(root: &Path) -> PathBuf {
    root.join(".cache")
        .join("dekascript")
        .join("serve-entry.dsx")
}

fn read_serve_entry(root: &Path) -> String {
    fs::read_to_string(serve_entry(root)).expect("read generated serve-entry.dsx")
}

/// Collect every file under `dir`, joined with newline, for diagnostics.
fn tree_listing(dir: &Path) -> String {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn dev_and_prod_emit_the_same_modules() {
    let project = tempfile::tempdir().expect("create temp project dir");
    write_fixture(project.path());

    let root = project.path().canonicalize().expect("canonical project");
    let entry = runtime_core::dist::write_app_router_entry(&root).expect("generate dev entry");
    let dsc = test_dsc();
    let dev_graph = pool::dsc_compile::compile_graph_with_dsc(&root, &entry, &dsc)
        .unwrap_or_else(|error| panic!("compile dev graph with {}: {error}", dsc.display()));
    let dev_entry = read_serve_entry(&root);
    assert!(dev_entry.contains("app/about/page.dsx") && dev_entry.contains("app/page.dsx"));

    let (success, combined) = run_build(&root, &dsc);
    assert!(
        success,
        "build must compile the same graph as dev with {}: {combined}",
        dsc.display()
    );
    assert_eq!(
        dev_entry,
        read_serve_entry(&root),
        "dev/build entry generation drifted"
    );

    // The shared -> leaf edge and both page -> shared edges survive in both
    // graphs. Compare complete module bodies, including their import specifiers.
    for rel in [
        "app/page.dsx",
        "app/about/page.dsx",
        "lib/shared.ds",
        "lib/leaf.ds",
    ] {
        let source = root.join(rel);
        let dev = pool::dsc_compile::lookup_js(&dev_graph, &source).expect("dev module");
        let prod_path = root
            .join("dist/server")
            .join(Path::new(rel).with_extension("js"));
        let prod = fs::read_to_string(&prod_path).expect("built module");
        // The loader keeps source extensions; publication rewrites them to JS.
        let normalized = dev
            .replace(".dsx\"", ".js\"")
            .replace(".ds\"", ".js\"")
            .replace(".dsx'", ".js'")
            .replace(".ds'", ".js'");
        assert_eq!(normalized, prod, "dev/prod module drift: {rel}");
    }
    for (rel, marker) in [
        ("lib/shared.ds", "shared-body-marker"),
        ("lib/leaf.ds", "leaf-body-marker"),
    ] {
        let js = pool::dsc_compile::lookup_js(&dev_graph, &root.join(rel)).expect("shared module");
        assert!(js.contains(marker), "missing live module body: {rel}");
    }
    for js in dev_graph.values() {
        assert!(
            !js.contains(ORPHAN_MARKER) && !js.contains(UNUSED_MARKER),
            "unreachable body leaked into dev graph: {js}"
        );
    }

    // 3. Elimination parity: the shaken-out modules are absent from every
    //    emitted artifact, not just the HTML (this scans dist/ wholesale).
    let dist = project.path().join("dist");
    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![dist.clone()];
    while let Some(current) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&current) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let text = fs::read_to_string(&path).unwrap_or_default();
                if text.contains(ORPHAN_MARKER) || text.contains(UNUSED_MARKER) {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "shaken-out modules (orphan.ds, unused.ds) leaked into prod emission: {offenders:?}\n{}",
        tree_listing(&dist)
    );

    // Module-boundary parity for the route tree: dist/server/app receives the
    // compiled modules one-for-one — same boundaries, same paths, .js from dsc.
    let dist_app = dist.join("server").join("app");
    for rel in ["layout.js", "page.js", "about/page.js", "not-found.js"] {
        assert!(
            dist_app.join(rel).is_file(),
            "dist/server/app must contain the compiled app module {rel}:\n{}",
            tree_listing(&dist_app)
        );
    }
    for rel in ["layout.dsx", "page.dsx", "about/page.dsx", "not-found.dsx"] {
        assert!(
            !dist_app.join(rel).exists(),
            "must not copy raw app/ .ds as the server product ({rel})"
        );
    }
}

/// Fail-closed parity (deka#5, preserved by the §11.2 fix): `deka build`
/// must still reject genuinely invalid source under app/ — files with no
/// imports validate standalone; files with imports validate through the
/// module graph. Either way the diagnostic names the offending file and no
/// dist/ output is written.
#[test]
fn build_still_fails_closed_on_invalid_source() {
    let project = tempfile::tempdir().expect("create temp project dir");
    write_fixture(project.path());

    // Import-free broken file: standalone validation path.
    fs::write(
        project.path().join("app").join("broken.dsx"),
        "export function f(): int { return\n",
    )
    .expect("write broken.dsx");
    let (success, combined) = run_build(project.path(), &test_dsc());
    assert!(
        !success,
        "deka build must exit non-zero on invalid source under app/: {combined}"
    );
    assert!(
        combined.contains("broken.dsx"),
        "build failure diagnostic should name the offending file: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "deka build should not write dist/ output when validation fails"
    );
}

#[test]
fn build_still_fails_closed_on_invalid_importing_source() {
    let project = tempfile::tempdir().expect("create temp project dir");
    write_fixture(project.path());

    // Broken file WITH imports: module-graph validation path — the same
    // graph compilation `deka serve --dev` performs.
    fs::write(
        project
            .path()
            .join("app")
            .join("broken-import.dsx"),
        "import { shared_message } from \"../lib/shared.ds\"\nexport fn Broken() {\n    return <main>{shared_message()}</main>;\n",
    )
    .expect("write broken-import.dsx");
    let (success, combined) = run_build(project.path(), &test_dsc());
    assert!(
        !success,
        "deka build must exit non-zero on a graph-invalid importing source: {combined}"
    );
    assert!(
        combined.contains("broken-import.dsx"),
        "build failure diagnostic should name the offending file: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "deka build should not write dist/ output when validation fails"
    );
}
