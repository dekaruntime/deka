// deka#719: the validated build manifest is the pre-execution validation
// boundary (plan version, slot shape, route disposition, output collisions):
// invalid plans fail before any build entry executes and never publish dist/.
//
// The successful-build tests (route table, staticParams expansion, prerendered
// HTML, transactional publish, cross-root determinism) were removed with the
// paused-framework teardown (deka#881): they pinned JSX-rendered app-router
// output, which no longer exists. What remains pins the failure boundary.
//
// Real dsc required (the suite's other build tests assume it too).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Scaffolds a fresh web project into `dir` via `deka init`, exactly the way
/// a human would, and asserts the scaffold succeeded.
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
    assert!(
        dir.join("app").join("page.dsx").is_file(),
        "deka init should scaffold app/page.dsx"
    );
}

fn run_build(dir: &Path) -> (bool, String) {
    run_build_with_env(dir, &[])
}

fn run_build_with_env(dir: &Path, env: &[(&str, &str)]) -> (bool, String) {
    let mut command = Command::new(cli_bin());
    command.arg("build").current_dir(dir);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("run deka build");
    // stdio emits to stderr (repo convention); table and diagnostics share it.
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn real_dsc() -> PathBuf {
    if let Ok(path) = std::env::var("DEKA_DSC") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let output = Command::new("which")
        .arg("dsc")
        .output()
        .expect("run which dsc");
    assert!(
        output.status.success(),
        "tests require dsc (set DEKA_DSC or put dsc on PATH)"
    );
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
}

#[test]
fn dynamic_route_without_disposition_fails() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let id_dir = project.path().join("app").join("[id]");
    fs::create_dir_all(&id_dir).expect("mkdir [id]");
    fs::write(
        id_dir.join("page.dsx"),
        "interface PageProps { id: string }\nexport fn Page(props: PageProps) {\n    return <article>{props.id}</article>;\n}\n",
    )
    .expect("write id page");

    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must fail when a dynamic route declares no disposition"
    );
    assert!(
        combined.contains("neither staticParams nor prerender = false"),
        "failure should name the missing disposition: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "a failed build must not publish dist/: {combined}"
    );
}

#[test]
fn route_collision_fails() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let slug_dir = project.path().join("app").join("posts").join("[slug]");
    fs::create_dir_all(&slug_dir).expect("mkdir [slug]");
    fs::write(
        slug_dir.join("page.dsx"),
        "interface PageProps { slug: string }\nstruct PostParam { slug: string }\nexport const staticParams: Array<PostParam> = build {\n    return Ok([PostParam{slug:\"hello\"}])\n}\nexport fn Page(props: PageProps) {\n    return <article>{props.slug}</article>;\n}\n",
    )
    .expect("write slug page");
    let concrete_dir = project.path().join("app").join("posts").join("hello");
    fs::create_dir_all(&concrete_dir).expect("mkdir posts/hello");
    fs::write(
        concrete_dir.join("page.dsx"),
        "export fn Page() {\n    return <article>concrete hello</article>;\n}\n",
    )
    .expect("write concrete page");

    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must fail when a staticParams instance collides with a concrete page"
    );
    assert!(
        combined.contains("route collision") && combined.contains("/posts/hello"),
        "failure should name the collision: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "a failed build must not publish dist/: {combined}"
    );
}

#[test]
fn stale_plan_rejected() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());

    // Fake dsc: stale plan output for `plan`, everything else delegated to
    // the real binary. Exercises the pre-execution validation boundary.
    let real = real_dsc();
    let fake = project.path().join("fake-dsc.sh");
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"plan\" ]; then\n  echo '{{\"version\": 99, \"slots\": []}}'\n  exit 0\nfi\nexec '{}' \"$@\"\n",
            real.display()
        ),
    )
    .expect("write fake dsc");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fake).expect("stat fake").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake, perms).expect("chmod fake");
    }

    let (success, combined) = run_build_with_env(
        project.path(),
        &[("DEKA_DSC", fake.to_str().expect("fake path utf8"))],
    );
    assert!(
        !success,
        "deka build must reject an unsupported dsc build plan version"
    );
    assert!(
        combined.contains("unsupported dsc build plan version 99"),
        "failure should name the stale plan version: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "a rejected plan must not publish dist/: {combined}"
    );
}
