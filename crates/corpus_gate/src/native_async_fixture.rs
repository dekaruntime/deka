//! Human-approved migration of one exact pre-native async source fixture.
use crate::{Case, Stage, Status};

pub(crate) fn migrate(case: &mut Case, metadata: &serde_json::Value) {
    if case.slug == "async-async-return-promise-unwraps"
        && case.status == Status::Pass
        && case.stage == Stage::Run
        && case.entry_path == "async_return_promise_unwraps.pass.ds"
        && case.source == include_str!("fixtures/async/legacy_return_promise.ds")
        && case.expected_stdout.as_deref() == Some("1\n2\n")
        && case.expected_diagnostic_contains.is_none()
        && case.files.is_empty()
        && case.packages.is_empty()
        && case.deka_json.is_none()
        && metadata
            == &serde_json::json!({
                "title": "Returning a Promise<T> from an async Promise<T> unwraps the value",
                "stage": "run"
            })
    {
        case.source = include_str!("fixtures/async/native_return_promise.ds").into();
    }
}
