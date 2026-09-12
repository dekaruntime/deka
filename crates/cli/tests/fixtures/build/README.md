# `deka build` artifact fixtures (deka#720)

Each fixture pins `deka build` failure output as a deterministic contract:
expected stderr (diagnostics) and the guarantee that no `dist/` is published.
Fixtures run a real Dsc + Deka build in a tempdir — nothing inspects a Rust
AST or snapshots internal codegen.

## Layout

```text
<name>/
  project/             deka project source; copied to a tempdir and built
  expected/
    stderr.txt         expected stderr after path normalization (the tempdir
                       path is replaced with `<project>`)
    fail               marker: the build must fail and must not publish dist/
```

The harness (`crates/cli/tests/build_fixture_harness.rs`, driven by
`crates/cli/tests/build_artifacts.rs`) compares the normalized stderr text
and asserts the build failed closed (non-zero exit, no `dist/`).

## Blessing (refreshing expected output)

Expected files are committed and change only intentionally:

```sh
DEKA_BLESS=1 cargo test -p cli --test build_artifacts
```

Blessing is an explicit local refresh step; normal test runs never rewrite
expectations. It refuses to bless when the build succeeds.

## History

Successful-build fixtures (`static-site`, `static-params`, `request-time`)
pinned JSX-rendered prerender HTML, route tables, dist tree bytes, and the
artifact manifest digests. They were removed with the paused-framework
teardown (deka#881): they pinned JSX-rendered prerender HTML, island assets,
and route CSS, none of which `deka build` emits anymore. App-router projects
still build and publish dist/ + manifests — the generated serve entry keeps
its paused `ui/*` imports, which the loader cannot resolve until the
framework returns in dsc (RFD 60) — so serving a built JSX page awaits that
return. What remains pins the pre-publication failure boundary (route
collisions). If the framework unpauses, restore the success-fixture
machinery from git history rather than recreating it.
