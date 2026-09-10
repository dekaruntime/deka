use super::*;
use std::fs;

fn payload(path: &str, body: &str) -> ArtifactPayload {
    ArtifactPayload {
        path: path.to_string(),
        role: payload_role(path),
        bytes: body.len() as u64,
        digest: artifact_digest(body.as_bytes()),
    }
}

fn sample_manifest() -> ArtifactManifestV2 {
    ArtifactManifestV2 {
        format: ARTIFACT_FORMAT.to_string(),
        origin: "authored".to_string(),
        producer: ArtifactProducer {
            deka: "0.0.0-test".to_string(),
            dsc: "dsc [version 0.0.0-test]".to_string(),
            plan_version: 2,
        },
        compat: ArtifactCompat {
            runtime_abi: RUNTIME_ABI,
            module_format: MODULE_FORMAT.to_string(),
            targets: vec!["native".to_string(), "worker".to_string()],
            host_imports: Vec::new(),
        },
        client: ArtifactClient {
            root: "client".to_string(),
            index: Some("client/index.html".to_string()),
            trailing_slash: false,
        },
        server: ArtifactServer {
            root: "server".to_string(),
            entries: vec![ServerEntry {
                id: "page:/".to_string(),
                kind: ServerEntryKind::Page,
                module: "server/app/page.js".to_string(),
                export: "default".to_string(),
                methods: Vec::new(),
            }],
        },
        worker: None,
        routes: vec![ArtifactRoute {
            template: "/".to_string(),
            mode: RouteMode::Static,
            instances: Vec::new(),
            outputs: vec!["client/index.html".to_string()],
            entry: None,
            slot: None,
            deferred: Vec::new(),
            source_file: "app/page.dsx".to_string(),
        }],
        slots: Vec::new(),
        payloads: vec![
            payload("client/index.html", "<html></html>"),
            payload("server/app/page.js", "export default function Page() {}"),
        ],
        payload_root: String::new(),
    }
}

#[test]
fn payload_root_hashes_path_and_digest_lines() {
    use sha2::Digest;
    let payloads = vec![
        payload("client/index.html", "a"),
        payload("server/app/page.js", "b"),
    ];
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"client/index.html\0");
    hasher.update(payloads[0].digest.as_bytes());
    hasher.update(b"\n");
    hasher.update(b"server/app/page.js\0");
    hasher.update(payloads[1].digest.as_bytes());
    hasher.update(b"\n");
    assert_eq!(
        compute_payload_root(&payloads),
        artifact_digest(&hasher.finalize())
    );
}

#[test]
fn serialization_is_compact_deterministic_and_sorted() {
    let mut manifest = sample_manifest();
    manifest.record_payloads(Path::new("/nonexistent-dist")).unwrap();
    let first = format!("{}\n", manifest.canonical_json().unwrap());
    let second = format!("{}\n", manifest.canonical_json().unwrap());
    assert_eq!(first, second);
    assert!(!first.contains('\n') || first.trim_end_matches('\n').find('\n').is_none());
    assert!(first.ends_with("\n"));
    // No incidental whitespace: `":` and `","` never appear with a space.
    assert!(!first.contains(": "), "{first}");
    assert!(!first.contains(", "), "{first}");
}

#[test]
fn record_payloads_hashes_everything_except_descriptors_and_rejects_symlinks() {
    let dist = tempfile::tempdir().unwrap();
    fs::write(dist.path().join("build-manifest.json"), "{}").unwrap();
    fs::write(dist.path().join("build-manifest.sha256"), "anchor").unwrap();
    fs::create_dir_all(dist.path().join("server/app")).unwrap();
    fs::write(dist.path().join("server/app/page.js"), "page").unwrap();
    fs::create_dir_all(dist.path().join("client")).unwrap();
    fs::write(dist.path().join("client/index.html"), "html").unwrap();

    let mut manifest = sample_manifest();
    manifest.record_payloads(dist.path()).unwrap();
    let paths: Vec<&str> = manifest.payloads.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(paths, vec!["client/index.html", "server/app/page.js"]);
    assert_eq!(
        manifest.payloads[1].digest,
        artifact_digest(b"page")
    );
    assert_eq!(manifest.payloads[1].role, PayloadRole::Server);
    assert_eq!(manifest.payloads[0].role, PayloadRole::Client);
    assert_eq!(
        manifest.payload_root,
        compute_payload_root(&manifest.payloads)
    );

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            dist.path().join("client/index.html"),
            dist.path().join("client/link.html"),
        )
        .unwrap();
        let mut manifest = sample_manifest();
        let err = manifest
            .record_payloads(dist.path())
            .expect_err("symlinks must fail the build");
        assert!(err.contains("symlink"), "{err}");
    }
}

#[test]
fn write_into_and_load_verified_round_trip() {
    let dist = tempfile::tempdir().unwrap();
    fs::create_dir_all(dist.path().join("client")).unwrap();
    fs::create_dir_all(dist.path().join("server/app")).unwrap();
    fs::write(dist.path().join("client/index.html"), "<html></html>").unwrap();
    fs::write(
        dist.path().join("server/app/page.js"),
        "export default function Page() {}",
    )
    .unwrap();
    let mut manifest = sample_manifest();
    manifest.record_payloads(dist.path()).unwrap();
    manifest.write_into(dist.path()).unwrap();

    let loaded = ArtifactManifestV2::load_verified(dist.path()).unwrap();
    assert_eq!(loaded, manifest);
}

#[test]
fn load_verified_detects_tampering_and_incompatibility() {
    let dist = tempfile::tempdir().unwrap();
    let mut manifest = sample_manifest();
    manifest.record_payloads(dist.path()).unwrap();
    manifest.write_into(dist.path()).unwrap();

    // Tampered manifest bytes -> sidecar mismatch.
    fs::write(dist.path().join("build-manifest.json"), "{}\n").unwrap();
    let err = ArtifactManifestV2::load_verified(dist.path()).unwrap_err();
    assert!(err.contains("integrity anchor"), "{err}");

    // Missing sidecar -> incomplete.
    fs::remove_file(dist.path().join("build-manifest.sha256")).unwrap();
    let mut tampered = sample_manifest();
    tampered.record_payloads(dist.path()).unwrap();
    tampered.write_into(dist.path()).unwrap();
    fs::remove_file(dist.path().join("build-manifest.sha256")).unwrap();
    let err = ArtifactManifestV2::load_verified(dist.path()).unwrap_err();
    assert!(err.contains("incomplete"), "{err}");

    // Wrong runtime_abi -> incompatible with the way out named.
    let mut incompatible = sample_manifest();
    incompatible.compat.runtime_abi = RUNTIME_ABI + 1;
    incompatible.record_payloads(dist.path()).unwrap();
    incompatible.write_into(dist.path()).unwrap();
    let err = ArtifactManifestV2::load_verified(dist.path()).unwrap_err();
    assert!(err.contains("runtime_abi"), "{err}");
    assert!(err.contains("deka build"), "{err}");
}

#[test]
fn verify_reports_digest_and_size_mismatches() {
    let dist = tempfile::tempdir().unwrap();
    let mut manifest = sample_manifest();
    fs::create_dir_all(dist.path().join("client")).unwrap();
    fs::create_dir_all(dist.path().join("server/app")).unwrap();
    fs::write(dist.path().join("client/index.html"), "<html></html>").unwrap();
    fs::write(
        dist.path().join("server/app/page.js"),
        "export default function Page() {}",
    )
    .unwrap();
    manifest.record_payloads(dist.path()).unwrap();
    assert!(manifest.verify(dist.path()).is_empty());

    fs::write(dist.path().join("server/app/page.js"), "mutated").unwrap();
    let problems = manifest.verify(dist.path());
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("server/app/page.js"), "{problems:?}");
    assert!(
        problems.iter().any(|p| p.contains("digest mismatch")),
        "{problems:?}"
    );
}

#[test]
fn client_output_path_maps_routes_like_prerender() {
    assert_eq!(client_output_path("/"), "client/index.html");
    assert_eq!(client_output_path("/about"), "client/about/index.html");
    assert_eq!(
        client_output_path("/posts/hello"),
        "client/posts/hello/index.html"
    );
}

#[test]
fn source_to_server_module_maps_compiled_paths() {
    assert_eq!(
        source_to_server_module("app/posts/[slug]/page.dsx").unwrap(),
        "server/app/posts/[slug]/page.js"
    );
    assert_eq!(
        source_to_server_module("./api/hello/route.ds").unwrap(),
        "server/api/hello/route.js"
    );
    assert!(source_to_server_module("app/readme.md").is_err());
}
