// deka#738 F3: `deka verify` recomputes the build manifest's artifact
// digests against the on-disk dist tree. Clean run exits 0; a tampered or
// missing artifact exits non-zero naming the path; a missing manifest or
// dist/ exits non-zero with a clear error.
//
// The fixture dist/ + artifact manifest are synthesized directly: the
// manifest's only producer is `deka build`, and the app-router projects that
// build emits manifests for cannot build while the framework is paused
// (deka#881). The verify contract under test is identical either way —
// `deka verify` never reads sources, only the published artifact.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn run_verify(project: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("verify")
        .current_dir(project)
        .output()
        .expect("run deka verify");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn manifest_path(project: &Path) -> PathBuf {
    // deka#762: verify is anchored on the v2 artifact manifest in dist/, not
    // the dev-cache v1 manifest.
    project.join("dist").join("build-manifest.json")
}

/// A project dir holding a published dist/ tree plus its v2 artifact
/// manifest, exactly as `deka build` would leave it.
fn built_project() -> tempfile::TempDir {
    use runtime_core::dist::{
        ARTIFACT_FORMAT, RUNTIME_ABI, ArtifactClient, ArtifactCompat, ArtifactManifestV2,
        ArtifactProducer, ArtifactServer,
    };

    let project = tempfile::tempdir().expect("create temp project dir");
    let dist = project.path().join("dist");
    fs::create_dir_all(dist.join("client")).expect("create dist/client");
    fs::write(
        dist.join("client").join("index.html"),
        "<!doctype html>\n<html><body>fixture</body></html>\n",
    )
    .expect("write dist/client/index.html");

    let mut manifest = ArtifactManifestV2 {
        format: ARTIFACT_FORMAT.to_string(),
        origin: "verify-command-fixture".to_string(),
        producer: ArtifactProducer {
            deka: "fixture".to_string(),
            dsc: "fixture".to_string(),
            plan_version: 2,
        },
        compat: ArtifactCompat {
            runtime_abi: RUNTIME_ABI,
            module_format: runtime_core::dist::MODULE_FORMAT.to_string(),
            targets: Vec::new(),
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
    manifest
        .record_payloads(&dist)
        .expect("record fixture payloads");
    manifest.write_into(&dist).expect("write fixture manifest");
    project
}

#[test]
fn verify_clean_tree_exits_zero() {
    let project = built_project();
    let (success, combined) = run_verify(project.path());
    assert!(success, "deka verify must exit 0 on a clean tree: {combined}");
}

#[test]
fn verify_tampered_artifact_fails_naming_path() {
    let project = built_project();
    let tampered = project.path().join("dist").join("client").join("index.html");
    assert!(tampered.is_file(), "fixture dist publishes client/index.html");
    let mut bytes = fs::read(&tampered).expect("read index.html");
    bytes.extend_from_slice(b"TAMPERED");
    fs::write(&tampered, bytes).expect("tamper with index.html");

    let (success, combined) = run_verify(project.path());
    assert!(
        !success,
        "deka verify must exit non-zero when an artifact no longer matches the manifest"
    );
    assert!(
        combined.contains("dist/client/index.html"),
        "the failure must name the tampered path: {combined}"
    );
}

#[test]
fn verify_deleted_artifact_fails_naming_path() {
    let project = built_project();
    let deleted = project.path().join("dist").join("client").join("index.html");
    fs::remove_file(&deleted).expect("delete index.html");

    let (success, combined) = run_verify(project.path());
    assert!(
        !success,
        "deka verify must exit non-zero when an artifact is missing"
    );
    assert!(
        combined.contains("dist/client/index.html"),
        "the failure must name the missing path: {combined}"
    );
}

#[test]
fn verify_without_manifest_fails_clearly() {
    let project = built_project();
    fs::remove_file(manifest_path(project.path())).expect("remove manifest");

    let (success, combined) = run_verify(project.path());
    assert!(
        !success,
        "deka verify must exit non-zero without a build manifest"
    );
    assert!(
        combined.contains("build-manifest.json") && combined.contains("deka build"),
        "the error should name the manifest and point at `deka build`: {combined}"
    );
}

#[test]
fn verify_without_dist_fails_clearly() {
    let project = built_project();
    fs::remove_dir_all(project.path().join("dist")).expect("remove dist");

    let (success, combined) = run_verify(project.path());
    assert!(
        !success,
        "deka verify must exit non-zero without a dist tree"
    );
    assert!(
        combined.contains("dist") && combined.contains("deka build"),
        "the error should name dist/ and point at `deka build`: {combined}"
    );
}
