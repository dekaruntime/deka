//! The seam gate, end to end (deka #789, tana #454).
//!
//! `CLAUDE.md` makes the seam-contract system mandatory for every boundary that
//! crosses a service, process or VM, and has reviewers auto-fail PRs that add
//! one without a registered contract. That rule is only worth anything if the
//! check actually fails on drift — an unwired gate is what produced the five
//! stacked linkhash↔gild↔runner boundary bugs, every one of which passed its
//! in-process unit test.
//!
//! So these tests run the real `cli` binary, not an in-process shortcut, and
//! the load-bearing one is the *negative*: a consumer that disagrees with its
//! producer must make `deka contract-check` exit non-zero.

use std::path::{Path, PathBuf};
use std::process::Command;

use seam_diff::{SeamErrorKind, check_consumer};

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

struct Run {
    code: i32,
    out: String,
}

fn contract_check(args: &[&str]) -> Run {
    let output = Command::new(cli_bin())
        .arg("contract-check")
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("run deka contract-check");
    let mut out = String::from_utf8_lossy(&output.stdout).into_owned();
    out.push_str(&String::from_utf8_lossy(&output.stderr));
    Run {
        code: output.status.code().unwrap_or(-1),
        out,
    }
}

/// The declared manifest is the CI gate. It must pass on a clean tree.
#[test]
fn declared_manifest_has_no_drift() {
    let run = contract_check(&["--manifest", "contracts/seams.json"]);
    assert_eq!(run.code, 0, "contract-check --manifest failed:\n{}", run.out);
    assert!(run.out.contains("all ok"), "{}", run.out);
}

/// **The test this issue exists for.** A green check against agreeing shapes is
/// not evidence the gate works. Take the real `rust:data_backend` producer,
/// rename exactly one field on the consumer side — the historical `{ok, value}`
/// vs `{ok, result}` break this contract was written to catch — and require a
/// non-zero exit naming the offending field.
#[test]
fn disagreeing_consumer_fails_the_check() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agreeing = dir.path().join("agreeing.ts");
    let drifting = dir.path().join("drifting.ts");

    std::fs::write(
        &agreeing,
        r#"
            export interface KvGetResponse {
                ok: boolean;
                value: string | null;
                error?: string;
            }
        "#,
    )
    .unwrap();
    std::fs::write(
        &drifting,
        r#"
            export interface KvGetResponse {
                ok: boolean;
                result: string | null;
                error?: string;
            }
        "#,
    )
    .unwrap();

    // Control: the same producer against a consumer that agrees passes.
    let ok = contract_check(&[
        "--producer",
        "rust:data_backend",
        "--consumer",
        &format!("ts:{}", agreeing.display()),
    ]);
    assert_eq!(ok.code, 0, "control run should pass:\n{}", ok.out);

    // The gate: one renamed field must fail the build.
    let drift = contract_check(&[
        "--producer",
        "rust:data_backend",
        "--consumer",
        &format!("ts:{}", drifting.display()),
    ]);
    assert_eq!(
        drift.code, 1,
        "drifting consumer must exit non-zero:\n{}",
        drift.out
    );
    assert!(drift.out.contains("seam drift"), "{}", drift.out);
    assert!(
        drift.out.contains("KvGetResponse.result"),
        "drift report must name the offending field:\n{}",
        drift.out
    );
}

/// One drifting seam fails the whole manifest run — the multi-seam CI gate.
#[test]
fn one_drifting_seam_fails_the_whole_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let consumer = dir.path().join("storefront_drift.ts");
    std::fs::write(
        &consumer,
        r#"
            export interface StorefrontResponse {
                status: number;
                headers: Record<string, string>;
                body: string;
                statusText: string;
            }
        "#,
    )
    .unwrap();

    let manifest = dir.path().join("seams.json");
    std::fs::write(
        &manifest,
        format!(
            r#"{{"seams":[
                {{"name":"good","producer":"rust:storefront","consumer":"ts:{root}/contracts/fixtures/storefront_consumer.ts"}},
                {{"name":"bad","producer":"rust:storefront","consumer":"ts:{drift}"}}
            ]}}"#,
            root = repo_root().display(),
            drift = consumer.display()
        ),
    )
    .unwrap();

    let run = contract_check(&["--manifest", &manifest.display().to_string()]);
    assert_eq!(run.code, 1, "manifest with drift must fail:\n{}", run.out);
    assert!(run.out.contains("✓ good"), "{}", run.out);
    assert!(run.out.contains("✗ bad"), "{}", run.out);
    assert!(
        run.out.contains("DRIFT (build should fail)"),
        "{}",
        run.out
    );
}

/// `check_consumer` is callable directly, against a real producer contract.
#[test]
fn check_consumer_is_callable_from_a_test() {
    let producer = runtime_core::data_envelope::data_backend_contract();

    let mut consumer = producer.clone();
    assert!(
        check_consumer(&producer, &consumer).is_empty(),
        "a contract must agree with itself"
    );

    for definition in &mut consumer.definitions {
        if let runtime_core::seam::SeamDefinition::Record(record) = definition
            && record.name == "KvGetResponse"
        {
            let value = record.fields.remove("value").expect("value field");
            record.fields.insert("result".to_string(), value);
        }
    }

    let errors = check_consumer(&producer, &consumer);
    assert!(
        errors.iter().any(|error| error.kind == SeamErrorKind::UnknownField
            && error.offending == "KvGetResponse.result"),
        "expected an UnknownField error for KvGetResponse.result, got {errors:?}"
    );
}

/// `deka contract-extract` emits a `seam.contract@1` document for a Rust target
/// — the `--rust` parameter that survived the migration without its command.
///
/// Note: `stdio::raw` writes to stderr, so the document arrives there rather
/// than on stdout. That is pre-existing `stdio` behaviour shared by every deka
/// command, unchanged by this recovery, so the test reads both streams.
#[test]
fn contract_extract_emits_the_rust_targets() {
    for target in ["storefront", "data_backend", "platform_env_policy"] {
        let output = Command::new(cli_bin())
            .args(["contract-extract", "--rust", target])
            .current_dir(repo_root())
            .output()
            .expect("run deka contract-extract");
        assert!(
            output.status.success(),
            "contract-extract --rust {target} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut emitted = String::from_utf8_lossy(&output.stdout).into_owned();
        emitted.push_str(&String::from_utf8_lossy(&output.stderr));
        let document: serde_json::Value =
            serde_json::from_str(emitted.trim()).expect("contract-extract emits JSON");
        assert_eq!(document["format"], "seam.contract@1");
        assert_eq!(document["name"], target);
        assert!(
            document["definitions"]
                .as_array()
                .is_some_and(|definitions| !definitions.is_empty()),
            "contract for {target} has no definitions"
        );
    }
}
