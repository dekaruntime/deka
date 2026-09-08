use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn init_project(dir: &Path) {
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(dir)
        .output()
        .expect("run deka init");
    assert!(
        output.status.success(),
        "deka init failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_build(dir: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("build")
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

#[test]
fn build_uses_separate_static_and_request_entries() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::create_dir_all(project.path().join("app/about")).expect("create about route");
    fs::write(
        project.path().join("app/about/page.dsx"),
        "export fn Page() { return <main><h1>About</h1></main>; }\n",
    )
    .expect("write about page");

    let (success, combined) = run_build(project.path());
    assert!(success, "deka build should succeed: {combined}");
    assert!(
        project.path().join("dist/client/about/index.html").is_file(),
        "each concrete route must produce an HTML artifact"
    );
    let cache = project.path().join(".cache/dekascript");
    assert!(
        !cache.join("serve-entry.dsx").exists(),
        "deka build must not generate the generic request router"
    );

    let static_entry = fs::read_to_string(cache.join("static-root-entry.dsx"))
        .expect("read generated static entry");
    assert!(
        static_entry.contains("renderToStringAsync") && static_entry.contains("StaticRender"),
        "static entry must own static rendering: {static_entry}"
    );
    assert!(
        !static_entry.contains("App(request)")
            && !static_entry.contains("text/x-deka-static")
            && !static_entry.contains("request.pathname"),
        "static entry must not contain request-router behavior: {static_entry}"
    );

    let about_entry = fs::read_to_string(cache.join("static-about-entry.dsx"))
        .expect("read generated about static entry");
    assert!(about_entry.contains("app/about/page.dsx"), "{about_entry}");
    assert!(
        !about_entry.contains("app/page.dsx"),
        "about's static entry must not import the root page: {about_entry}"
    );
}
