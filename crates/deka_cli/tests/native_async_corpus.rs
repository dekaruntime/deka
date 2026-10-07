//! Real CLI effects for the approved native async source migration.
use corpus_gate::{evaluate, load_cases, run_case};
use std::path::Path;

const LEGACY: &str = include_str!("../../corpus_gate/src/fixtures/async/legacy_return_promise.ds");
const NATIVE: &str = include_str!("../../corpus_gate/src/fixtures/async/native_return_promise.ds");

fn fixture(root: &Path, source: &str) {
    let directory = root.join("async/async_return_promise_unwraps");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("async_return_promise_unwraps.pass.ds"),
        source,
    )
    .unwrap();
    std::fs::write(
        directory.join("async_return_promise_unwraps.stdout"),
        "1\n2\n",
    )
    .unwrap();
    std::fs::write(directory.join("async_return_promise_unwraps.json"), r#"{"title":"Returning a Promise<T> from an async Promise<T> unwraps the value","stage":"run"}"#).unwrap();
}

#[test]
fn pinned_async_migration_and_canonical_source_execute_both_result_arms() {
    for source in [LEGACY, NATIVE] {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path(), source);
        let cases = load_cases(root.path());
        assert_eq!(cases.len(), 1);
        let case = &cases[0];
        let result = run_case(Path::new(env!("CARGO_BIN_EXE_deka")), case, root.path()).unwrap();
        assert!(evaluate(case, &result).is_empty(), "{result:?}");
        assert_eq!(case.source, NATIVE);
    }
}

#[test]
fn changed_legacy_source_and_metadata_are_never_rewritten() {
    for change in 0..6 {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path(), LEGACY);
        let directory = root.path().join("async/async_return_promise_unwraps");
        match change {
            0 => std::fs::write(directory.join("async_return_promise_unwraps.pass.ds"), format!("{LEGACY}// changed\n")).unwrap(),
            1 => std::fs::write(directory.join("async_return_promise_unwraps.stdout"), "changed\n").unwrap(),
            2 => std::fs::write(directory.join("extra.ds"), "const extra = 1\n").unwrap(),
            3 => std::fs::write(directory.join("async_return_promise_unwraps.json"), r#"{"title":"Changed","stage":"run"}"#).unwrap(),
            4 => std::fs::write(directory.join("async_return_promise_unwraps.json"), r#"{"title":"Returning a Promise<T> from an async Promise<T> unwraps the value","stage":"typecheck"}"#).unwrap(),
            5 => std::fs::write(directory.join("async_return_promise_unwraps.json"), r#"{"title":"Returning a Promise<T> from an async Promise<T> unwraps the value","stage":"run","packages":["real-package"]}"#).unwrap(),
            _ => unreachable!(),
        }
        let cases = load_cases(root.path());
        let case = &cases[0];
        assert!(case.source.starts_with(LEGACY));
        let result = run_case(Path::new(env!("CARGO_BIN_EXE_deka")), case, root.path());
        assert!(
            result.is_err() || !evaluate(case, &result.unwrap()).is_empty(),
            "changed contract {change} passed"
        );
    }
}

#[test]
fn native_error_fallback_assertion_is_effective() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), &NATIVE.replace("Err(_) => 0", "Err(_) => 99"));
    let case = load_cases(root.path()).pop().unwrap();
    let result = run_case(Path::new(env!("CARGO_BIN_EXE_deka")), &case, root.path()).unwrap();
    assert!(!evaluate(&case, &result).is_empty());
}
