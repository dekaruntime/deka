//! Exact obsolete package metadata for settled native argument failures.
use crate::{Case, Stage, Status};

pub(super) fn matches(case: &Case) -> bool {
    let (source, entry, package, diagnostic) = match case.slug.as_str() {
        "time-now-no-args" => (
            include_str!("legacy/builtin_negative/now_no_args.fail.ds"),
            "now_no_args.fail.ds",
            "time",
            "argument",
        ),
        "crypto-sha256-wrong-type" => (
            include_str!("legacy/builtin_negative/sha256_wrong_type.fail.ds"),
            "sha256_wrong_type.fail.ds",
            "crypto",
            "expected argument type `bytes`, found type `string`",
        ),
        _ => return false,
    };
    case.status == Status::Fail
        && case.stage == Stage::Parse
        && case.source == source
        && case.entry_path == entry
        && case.packages == [package]
        && case.files.is_empty()
        && case.deka_json.is_none()
        && case.expected_stdout.is_none()
        && case.expected_diagnostic_contains.as_deref() == Some(diagnostic)
}
