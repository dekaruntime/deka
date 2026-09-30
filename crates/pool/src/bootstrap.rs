/// Assemble the bootstrap script, injecting the shared enum prelude and the
/// `__deka_to_result` helper (deka#582). Shapes must match dsc's emitted
/// `Result`/`Option` constructors (see `crate::prelude`).
pub fn bootstrap_source() -> String {
    let source = include_str!("bootstrap.js")
        .replace(
            "/*__DEKA_POOL_ENUM_PRELUDE__*/",
            &crate::prelude::pool_prelude(),
        )
        .replace("__DEKA_TO_RESULT__", &crate::prelude::to_result_helper())
        .replace(
            "/*__DEKA_HOST_CATALOG__*/",
            &permissions::host_bridge::js_catalog_json(),
        )
        .replace(
            "__DEKA_PERMISSION_DENIED_MARKER__",
            permissions::host_bridge::PERMISSION_DENIED_MARKER,
        )
        .replace("/*__DEKA_WINTERTC__*/", include_str!("wintertc.js"));
    // assert!, not debug_assert!: release is what ships, and a marker that
    // fails to substitute there fails silently. The prelude marker sits inside
    // a /* */ comment, so an un-replaced one simply vanishes and the isolate
    // boots with no Result/Option at all — surfacing much later as
    // `Ok is not defined` on a request. That is precisely the silent
    // divergence this change exists to remove, so the check has to run in the
    // build that matters. Two substring scans once per worker bootstrap is
    // nothing next to creating the isolate.
    assert!(
        !source.contains("__DEKA_POOL_ENUM_PRELUDE__")
            && !source.contains("__DEKA_TO_RESULT__")
            && !source.contains("__DEKA_HOST_CATALOG__")
            && !source.contains("__DEKA_PERMISSION_DENIED_MARKER__")
            && !source.contains("__DEKA_WINTERTC__"),
        "bootstrap prelude markers must all be injected"
    );
    source
}
