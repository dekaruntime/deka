// deka#720: `deka build` as a deterministic artifact contract. Each fixture
// runs a real Dsc + Deka build flow in a tempdir and compares expected
// stderr (route table + diagnostics), the published dist/ tree, and the
// exact bytes of declared outputs — twice, asserting byte-for-byte
// determinism. See fixtures/build/README.md for the layout and the
// DEKA_BLESS=1 refresh workflow.
//
// Coverage (deka#720 initial):
// - static-site: a concrete static page + layout emit the expected HTML;
// - static-params: staticParams expands deterministic concrete paths with ●;
// - request-time: prerender = false publishes no static HTML and stays a ƒ
//   request route (gated on dsc plan v2; on v1 dsc the same fixture asserts
//   the fail-closed upgrade-dsc diagnostic instead);
// - collision-duplicate-slug / collision-concrete-page: normalized-colliding
//   parameters fail before publication with the collision diagnostic.
//
// A `server:defer` ◐ fixture waits for deka#718 phase A (the route table has
// no partial classification yet). A failed-renderer fixture is not reachable
// in the current compiler slice: a render-time throw is either a dsc type
// error (build fails before render) or tree-shaken before prerender runs.
//
// Real dsc required, like the other build suites.

#[path = "build_fixture_harness.rs"]
mod build_fixture_harness;

#[test]
fn static_site_page_and_layout_emit_expected_html() {
    build_fixture_harness::check_fixture("static-site");
}

#[test]
fn static_params_expand_concrete_paths() {
    build_fixture_harness::check_fixture("static-params");
}

#[test]
fn prerender_false_stays_a_request_route() {
    build_fixture_harness::check_fixture("request-time");
}

#[test]
fn duplicate_static_params_slug_fails() {
    build_fixture_harness::check_fixture("collision-duplicate-slug");
}

#[test]
fn static_params_instance_colliding_with_concrete_page_fails() {
    build_fixture_harness::check_fixture("collision-concrete-page");
}
