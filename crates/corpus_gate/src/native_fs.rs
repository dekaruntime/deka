//! Exact pinned filesystem fixtures whose package metadata predates native ops.
use crate::{Case, Stage, Status};
pub(crate) fn matches(case: &Case) -> bool {
    let (source, stdout) = match case.slug.as_str() {
        "fs-mkdirs-idempotent" => (include_str!("legacy/fs/mkdirs_idempotent.ds"), "ok\n"),
        "fs-read-missing" => (include_str!("legacy/fs/read_missing.ds"), "err\n"),
        "fs-read-dir-missing" => (include_str!("legacy/fs/read_dir_missing.ds"), "err\n"),
        _ => return false,
    };
    case.status == Status::Pass
        && case.stage == Stage::Run
        && case.source == source
        && case.packages == ["fs"]
        && case.files.is_empty()
        && case.expected_stdout.as_deref() == Some(stdout)
        && case.expected_diagnostic_contains.is_none()
        && case.deka_json.as_ref()
            == Some(&serde_json::json!({
                "name":"conformance-fixture",
                "security":{"allow":{"read":["./"],"write":[".cache","php_modules"]},"prompt":false}
            }))
}
