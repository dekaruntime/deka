// deka#720/#728: `deka build` as a deterministic artifact contract. Each
// fixture runs a real Dsc + Deka build flow in a tempdir and compares
// expected stderr (route table + diagnostics) and the absence of a published
// dist/. See fixtures/build/README.md for the layout.
//
// The successful-build fixtures (static-site, static-params, request-time)
// were removed with the paused-framework teardown (deka#881): they pinned
// JSX-rendered prerender HTML, island assets, and route CSS, none of which
// `deka build` emits anymore. (App-router projects still build and publish
// dist/ + manifests; the paused `ui/*` imports in the serve entry stay
// unresolved at serve time until the framework returns in dsc, RFD 60.)
// What remains pins the pre-publication failure boundary.
//
// Coverage (deka#720 initial):
// - collision-duplicate-slug / collision-concrete-page: normalized-colliding
//   parameters fail before publication with the collision diagnostic.
//
// Real dsc required, like the other build suites.

#[path = "build_fixture_harness.rs"]
mod build_fixture_harness;

#[test]
fn duplicate_static_params_slug_fails() {
    build_fixture_harness::check_fixture("collision-duplicate-slug");
}

#[test]
fn static_params_instance_colliding_with_concrete_page_fails() {
    build_fixture_harness::check_fixture("collision-concrete-page");
}
