//! Seam gate for the linkha.sh -> git.tana.gg registry boundary (tana #454, #901).
//!
//! linkha.sh (PHPX, jynx) reads its package/version/release data across the
//! network from the git.tana.gg registry (gild-vcs, Rust/axum, demon). That is a
//! cross-service boundary, so it is modelled in the seam-contract system rather
//! than hand-shaken. Three layers are exercised here, and each closes a
//! different hole:
//!
//!   1. `manifest_gate_passes_for_every_declared_seam` — runs the REAL
//!      `deka contract-check --manifest contracts/seams.json` binary and asserts
//!      exit 0. Previously the manifest was advisory: nothing ran it, which is
//!      exactly the "gate is advisory only where it isn't wired" failure mode
//!      CLAUDE.md calls out. Now a drifting seam fails `cargo test`.
//!
//!   2. `producer_field_rename_is_caught_by_the_gate` — proves the gate bites.
//!      Renaming `latest` on the producer must fail the consumer check, which is
//!      the precise regression this boundary was flagged for: a silent rename
//!      would otherwise degrade every consumer field to its fallback.
//!
//!   3. `live_registry_matches_the_registered_producer_contracts` — fetches the
//!      real payloads from the live registry and asserts the registered producer
//!      contracts still describe them (field-for-field, including nullability).
//!      Layers 1 and 2 only compare fixtures to fixtures; without this layer a
//!      stale producer fixture would keep the gate green while the real service
//!      had already drifted.

extern crate std as core;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

const REGISTRY_SEAMS: [&str; 3] = [
    "linkhash.registry_packages",
    "linkhash.registry_versions",
    "linkhash.registry_release",
];

fn runtime_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("resolve runtime root")
}

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

/// Run the real `contract-check` command the same way CI and a developer would.
fn contract_check(args: &[&str]) -> (i32, String) {
    let output = Command::new(cli_bin())
        .args(args)
        .current_dir(runtime_root())
        .output()
        .expect("run deka contract-check");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), combined)
}

// ---------------------------------------------------------------------------
// Layer 1 — the declared manifest is an enforced gate, not a suggestion.
// ---------------------------------------------------------------------------

#[test]
fn manifest_gate_passes_for_every_declared_seam() {
    let (code, report) = contract_check(&["contract-check", "--manifest", "contracts/seams.json"]);

    assert_eq!(
        code, 0,
        "seam manifest reported drift (exit {code}):\n{report}"
    );

    // The registry seams must actually be part of the checked set — a typo in
    // seams.json would otherwise leave this boundary silently unguarded.
    for seam in REGISTRY_SEAMS {
        assert!(
            report.contains(&format!("✓ {seam}")),
            "seam '{seam}' was not checked by the manifest run:\n{report}"
        );
    }
}

// ---------------------------------------------------------------------------
// Layer 2 — the gate bites on the exact drift this boundary was flagged for.
// ---------------------------------------------------------------------------

#[test]
fn producer_field_rename_is_caught_by_the_gate() {
    let root = runtime_root();
    let producer = root.join("contracts/fixtures/linkhash/registry/packages_producer.phpx");
    let source = std::fs::read_to_string(&producer).expect("read packages producer fixture");

    // Simulate gild-vcs renaming `latest` -> `latest_version`.
    let drifted = source.replace("$latest: string;", "$latest_version: string;");
    assert_ne!(drifted, source, "fixture no longer declares `$latest`");

    let dir = tempfile::tempdir().expect("tempdir");
    let drifted_path = dir.path().join("packages_producer_drifted.phpx");
    std::fs::write(&drifted_path, drifted).expect("write drifted producer");

    let (code, report) = contract_check(&[
        "contract-check",
        "--producer",
        &format!("phpx:{}", drifted_path.display()),
        "--consumer",
        "phpx:contracts/fixtures/linkhash/registry/packages_consumer.phpx",
    ]);

    assert_eq!(code, 1, "renamed producer field did not fail the gate:\n{report}");
    assert!(
        report.contains("UnknownField") && report.contains("RegistryPackageEntry.latest"),
        "gate failed but not for the renamed field:\n{report}"
    );
}

// ---------------------------------------------------------------------------
// Layer 3 — the registered producer contracts still describe the live service.
// ---------------------------------------------------------------------------

/// Extract a registered producer contract through the real CLI.
///
/// `stdio::raw` emits on stderr, so take whichever stream carries the payload
/// rather than assuming stdout.
fn extract_producer(fixture: &str) -> Value {
    let output = Command::new(cli_bin())
        .args(["contract-extract", &format!("phpx:{fixture}")])
        .current_dir(runtime_root())
        .output()
        .expect("run deka contract-extract");
    assert!(
        output.status.success(),
        "contract-extract failed for {fixture}: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let payload = if stdout.trim_start().starts_with('{') {
        stdout
    } else {
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    serde_json::from_str(payload.trim())
        .unwrap_or_else(|err| panic!("parse extracted contract for {fixture}: {err}\n{payload}"))
}

/// The named record's fields, as `name -> seam type node`.
fn record_fields<'a>(contract: &'a Value, record: &str) -> &'a serde_json::Map<String, Value> {
    contract["definitions"]
        .as_array()
        .expect("definitions array")
        .iter()
        .find(|definition| definition["name"] == record)
        .unwrap_or_else(|| panic!("contract has no record '{record}'"))
        ["fields"]
        .as_object()
        .expect("fields object")
}

/// Does a live JSON value satisfy a `seam.contract@1` type node?
fn value_matches(seam_type: &Value, value: &Value) -> bool {
    match seam_type["kind"].as_str() {
        Some("option") => value.is_null() || value_matches(&seam_type["item"], value),
        Some("list") => value
            .as_array()
            .is_some_and(|items| items.iter().all(|item| value_matches(&seam_type["item"], item))),
        Some("primitive") => match seam_type["name"].as_str() {
            Some("String") => value.is_string(),
            Some("Int") => value.is_i64() || value.is_u64(),
            Some("Bool") => value.is_boolean(),
            Some("Bytes") => value.is_string() || value.is_array(),
            _ => false,
        },
        // Named records are compared separately against their own live object.
        Some("named") => value.is_object() || value.is_array(),
        _ => false,
    }
}

/// Assert a live JSON object matches a producer record field-for-field: no
/// field the contract promises is missing, no field the service returns is
/// undeclared, and every value satisfies its declared type (nullable included).
fn assert_object_matches_record(contract: &Value, record: &str, live: &Value, label: &str) {
    let fields = record_fields(contract, record);
    let live_object = live
        .as_object()
        .unwrap_or_else(|| panic!("{label}: live payload is not a JSON object: {live}"));

    let declared: BTreeSet<&str> = fields.keys().map(String::as_str).collect();
    let actual: BTreeSet<&str> = live_object.keys().map(String::as_str).collect();

    let missing: Vec<&&str> = declared.difference(&actual).collect();
    assert!(
        missing.is_empty(),
        "{label}: producer contract '{record}' promises field(s) {missing:?} that the live \
         registry no longer returns — the contract is stale, update it and re-check consumers"
    );

    let undeclared: Vec<&&str> = actual.difference(&declared).collect();
    assert!(
        undeclared.is_empty(),
        "{label}: live registry returns field(s) {undeclared:?} not declared by producer \
         contract '{record}' — the contract is stale, update it"
    );

    for (name, seam_type) in fields {
        let value = &live_object[name];
        assert!(
            value_matches(seam_type, value),
            "{label}: field '{name}' does not match its declared seam type \
             {seam_type} (live value: {value})"
        );
    }
}

fn registry_base() -> String {
    std::env::var("LINKHASH_GIT_API_URL")
        .unwrap_or_else(|_| "https://git.tana.gg".to_string())
}

// ---------------------------------------------------------------------------
// Layer 4 — the consumer's own visibility handling actually fails closed.
// ---------------------------------------------------------------------------

/// Runs linkha.sh's fail-closed visibility suite through the real PHPX runtime.
///
/// The seam layers above pin the *shape* of the boundary; this pins the
/// *behaviour* on the consumer side. `visibility` decides what an anonymous
/// visitor is shown, and linkha.sh filters it locally after reading the
/// registry with a privileged token — so a missing or renamed value must
/// withhold the row rather than publish it.
///
/// The suite exercises `src/services/registry_api.phpx` itself (not a Rust
/// re-implementation) and exits non-zero on any failed assertion.
#[test]
fn linkhash_visibility_fails_closed_under_the_real_phpx_runtime() {
    let project = runtime_root().join("../linkhash/phpx");
    let suite = project.join("tests/visibility_fail_closed.phpx");
    assert!(
        suite.exists(),
        "missing visibility suite at {}",
        suite.display()
    );

    let output = Command::new(cli_bin())
        .args(["run", "tests/visibility_fail_closed.phpx"])
        .current_dir(&project)
        .output()
        .expect("run deka run");

    let mut report = String::from_utf8_lossy(&output.stdout).into_owned();
    report.push_str(&String::from_utf8_lossy(&output.stderr));

    assert!(
        output.status.success(),
        "visibility suite failed (exit {:?}):\n{report}",
        output.status.code()
    );
    assert!(
        report.contains("RESULT OK"),
        "visibility suite did not report success:\n{report}"
    );
    assert!(
        !report.contains("FAIL "),
        "visibility suite reported failing assertions:\n{report}"
    );
}

#[test]
fn live_registry_matches_the_registered_producer_contracts() {
    let base = registry_base();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("build http client");

    let fetch = |path: &str| -> Option<Value> {
        let url = format!("{base}{path}");
        match client.get(&url).send() {
            Ok(response) if response.status().is_success() => {
                Some(response.json().expect("registry returned non-JSON"))
            }
            Ok(response) => panic!("{url} returned HTTP {}", response.status()),
            Err(err) => {
                eprintln!("SKIP: registry {base} unreachable ({err}); live seam conformance not verified");
                None
            }
        }
    };

    // --- GET /api/packages -------------------------------------------------
    let Some(packages_payload) = fetch("/api/packages") else {
        return;
    };
    let packages_contract =
        extract_producer("contracts/fixtures/linkhash/registry/packages_producer.phpx");
    assert_object_matches_record(
        &packages_contract,
        "RegistryPackagesResponse",
        &packages_payload,
        "GET /api/packages",
    );

    let entries = packages_payload["packages"]
        .as_array()
        .expect("packages array");
    assert!(
        !entries.is_empty(),
        "live registry returned no packages; cannot verify the entry shape"
    );
    for entry in entries {
        assert_object_matches_record(
            &packages_contract,
            "RegistryPackageEntry",
            entry,
            "GET /api/packages entry",
        );
    }

    // Pick a real package to drive the per-package endpoints.
    let sample = &entries[0];
    let full_name = sample["name"].as_str().expect("package name");
    let version = sample["latest"].as_str().expect("package latest");
    let (scope, name) = full_name
        .trim_start_matches('@')
        .split_once('/')
        .unwrap_or_else(|| panic!("unscoped package name '{full_name}'"));

    // --- GET /api/scoped-packages/{scope}/{name}/versions ------------------
    let Some(versions_payload) = fetch(&format!("/api/scoped-packages/{scope}/{name}/versions"))
    else {
        return;
    };
    let versions_contract =
        extract_producer("contracts/fixtures/linkhash/registry/versions_producer.phpx");
    assert_object_matches_record(
        &versions_contract,
        "RegistryVersionsResponse",
        &versions_payload,
        "GET /api/scoped-packages/{scope}/{name}/versions",
    );

    // --- GET /api/scoped-packages/{scope}/{name}/{version} -----------------
    let Some(release_payload) =
        fetch(&format!("/api/scoped-packages/{scope}/{name}/{version}"))
    else {
        return;
    };
    let release_contract =
        extract_producer("contracts/fixtures/linkhash/registry/release_producer.phpx");
    assert_object_matches_record(
        &release_contract,
        "RegistryReleaseResponse",
        &release_payload,
        "GET /api/scoped-packages/{scope}/{name}/{version}",
    );
}
