use corpus_gate::{Stage, Status, evaluate, load_cases, run_case};
use std::path::Path;

fn fixtures(root: &Path) {
    for (category, name, source, metadata) in [
        (
            "time",
            "now_no_args",
            include_str!("../../corpus_gate/src/legacy/builtin_negative/now_no_args.fail.ds"),
            include_str!("../../corpus_gate/src/legacy/builtin_negative/now_no_args.json"),
        ),
        (
            "crypto",
            "sha256_wrong_type",
            include_str!("../../corpus_gate/src/legacy/builtin_negative/sha256_wrong_type.fail.ds"),
            include_str!("../../corpus_gate/src/legacy/builtin_negative/sha256_wrong_type.json"),
        ),
    ] {
        let directory = root.join(category).join(name);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(format!("{name}.fail.ds")), source).unwrap();
        std::fs::write(directory.join(format!("{name}.json")), metadata).unwrap();
    }
}

#[test]
fn archived_argument_failures_reach_the_actual_native_checker_offline() {
    let corpus = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    fixtures(corpus.path());
    let cases = load_cases(corpus.path());
    assert_eq!(cases.len(), 2);
    for case in cases {
        assert_eq!(case.status, Status::Fail);
        assert_eq!(case.stage, Stage::Parse);
        let result =
            run_case(Path::new(env!("CARGO_BIN_EXE_deka")), &case, scratch.path()).unwrap();
        assert!(!result.ok, "{} accepted invalid arguments", case.slug);
        assert!(
            result.transpile_failed,
            "{} failed outside check",
            case.slug
        );
        assert!(result.stdout.is_empty());
        // echo(r) has a secondary error in both archived programs. Require
        // the targeted call failure, not merely any argument diagnostic.
        let primary = match case.slug.as_str() {
            "time-now-no-args" => "[check] 3:11: expected 0 arguments, found 1",
            "crypto-sha256-wrong-type" => {
                "[check] 3:18: expected argument type `bytes`, found type `string`"
            }
            _ => unreachable!(),
        };
        assert_eq!(result.stderr.lines().nth(1), Some(primary));
        assert!(
            evaluate(&case, &result).is_empty(),
            "{}: {:?}",
            case.slug,
            result.diagnostics
        );
        assert!(
            !result.stderr.contains("unknown identifier"),
            "{}",
            result.stderr
        );
        assert!(
            !result.stderr.contains("unknown built-in"),
            "{}",
            result.stderr
        );
        assert!(!scratch.path().join(&case.slug).join("ds_modules").exists());
    }
}

#[test]
fn changed_archived_contracts_and_legacy_sleep_ms_stay_offline() {
    let corpus = tempfile::tempdir().unwrap();
    fixtures(corpus.path());
    for original in load_cases(corpus.path()) {
        for change in 0..10 {
            let scratch = tempfile::tempdir().unwrap();
            let mut case = load_cases(corpus.path())
                .into_iter()
                .find(|c| c.slug == original.slug)
                .unwrap();
            match change {
                0 => case.source.push_str("// changed source\n"),
                1 => case.packages.push("real-package".into()),
                2 => case.stage = Stage::Typecheck,
                3 => case.status = Status::Pass,
                4 => case.expected_diagnostic_contains = Some("new contract".into()),
                5 => case.expected_stdout = Some("output\n".into()),
                6 => case.deka_json = Some(serde_json::json!({"name": "new contract"})),
                7 => case
                    .files
                    .push(("extra.ds".into(), "const value = 1".into())),
                8 => case.slug = "time-sleep-ms-arity".into(),
                9 => case.entry_path = "renamed.fail.ds".into(),
                _ => unreachable!(),
            }
            let error =
                run_case(Path::new(env!("CARGO_BIN_EXE_deka")), &case, scratch.path()).unwrap_err();
            assert!(error.contains("offline"), "{change}: {error}");
            assert!(
                !scratch.path().join(&case.slug).exists(),
                "changed case was staged"
            );
        }
    }
}
