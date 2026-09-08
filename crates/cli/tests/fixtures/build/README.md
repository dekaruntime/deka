# `deka build` artifact fixtures (deka#720)

Each fixture pins `deka build` output as a deterministic contract: expected
stderr (route table + diagnostics), the published `dist/` file tree, and the
exact bytes of every declared output. Fixtures run a real Dsc + Deka build in
a tempdir — nothing inspects a Rust AST or snapshots internal codegen.

## Layout

```text
<name>/
  project/             deka project source; copied to a tempdir and built
  expected/
    stderr.txt         expected stderr after path normalization (the tempdir
                       path is replaced with `<project>`); includes the route
                       table and any diagnostics
    stderr.v1.txt      optional: expected stderr when dsc emits build plan
                       version 1 (dsc 0.6.x); selected automatically on v1
    fail               optional marker: the build must fail and must not
                       publish dist/
    v1-fail            optional marker: under plan v1 the build must fail
                       (pairs with stderr.v1.txt; used by request-time,
                       whose prerender = false needs plan v2 / dsc#54)
    tree.txt           canonical sorted dist/ relative paths (success only)
    files/             mirror of dist/ holding the expected BYTES of every
                       path in tree.txt (success only)
```

The harness (`crates/cli/tests/build_fixture_harness.rs`, driven by
`crates/cli/tests/build_artifacts.rs`) compares:

1. **stderr** — full normalized text; the route table's row ordering is part
   of the contract (the table derives from the build manifest only).
2. **file tree** — `tree.txt` vs the built `dist/`; a mismatch lists missing
   and unexpected paths.
3. **bytes** — every path in `tree.txt` is compared against the `files/`
   mirror; text files get a unified diff, binary files a size note.

For successful fixtures it additionally asserts, from
`.cache/dekascript/build-manifest.json`: the manifest parses, its routes
match the printed table exactly (glyph + path, in order), its artifact list
covers exactly the published tree, **and every artifact digest is recomputed
from the published bytes** — a manifest with correct paths but wrong digests
fails, naming the mismatched path. Then the fixture builds twice more: a
**same-root rerun** (dist tree, full stderr, and the manifest must be
byte-identical across runs) and a **cross-root build** (the same project
from a second temporary root must produce RAW, byte-identical dist
artifacts).

## Blessing (refreshing expected output)

Expected files are committed and change only intentionally:

```sh
DEKA_BLESS=1 cargo test -p cli --test build_artifacts
```

Blessing rewrites `stderr.txt` (or `stderr.v1.txt`, matching the installed
dsc) and, for successful fixtures, `tree.txt` and the `files/` mirror. It
refuses to bless when the build's success/failure disagrees with the
fixture's markers. A fixture with both `stderr.txt` and `stderr.v1.txt`
must be blessed once per dsc plan generation (CI installs dsc 0.6.0 → v1).

The mirror holds **raw** bytes. Since dsc PR #62 (deka#728) slot ids are
project-relative, so raw bytes are stable across build roots; the fixtures
were re-blessed with that dsc. An older dsc whose slot ids hash absolute
paths will fail the `static-params` byte comparison and the cross-root
check — that is the contract working, not flakiness.

## Coverage

| fixture | issue item | dsc plan |
|---|---|---|
| `static-site` | concrete static page + layout emit expected HTML | v1 |
| `static-params` | staticParams expands ● concrete paths | v1 |
| `request-time` | prerender = false → no static HTML, stays a ƒ route | v2 (v1 asserts the fail-closed upgrade-dsc diagnostic) |
| `collision-duplicate-slug` | duplicate slug fails pre-publish | v1 |
| `collision-concrete-page` | instance vs concrete page collision fails | v1 |

Deferred on purpose:

- **`server:defer` ◐ classification** waits for deka#718 phase A; the route
  table has no partial mode yet. When it lands, note that the defer secret
  (`.cache/dekascript/defer.key`) is per-project persistent: commit one into
  the fixture's `project/` so encrypted-props bytes stay stable across
  machines (gitignore only accepts it when explicitly added with `git add -f`
  — `.cache/` is a repo-wide ignore pattern).
- **failed renderer** is not reachable in the current compiler slice: a
  render-time throw is either rejected by dsc as a type error (build fails
  before render) or tree-shaken before prerender runs.

## Determinism notes (verified, not assumed)

- Content-hashed asset names are stable across runs when inputs are stable
  (the rerun and cross-root builds in every successful fixture would catch
  rotation).
- **dsc build-slot ids are project-relative since dsc PR #62** (deka#728);
  previously they hashed the source file's absolute path, which made the
  `deka:dev/<id>` import embedded in emitted JS a function of the build
  root. The harness now compares **raw bytes** with no normalization: every
  successful fixture builds from a second temporary root and the dist trees
  must match byte-for-byte. If that check ever fails, investigate which
  bytes embed the root before considering any normalization — and if one is
  truly unavoidable, narrow it to that specific case with a comment here.
- The build manifest embeds absolute paths (slot files, route sources), so
  it is byte-compared only across same-root rebuilds; across roots the
  artifacts' bytes are the contract, plus per-root manifest verification.
- No volatile file needed exclusion from byte comparison; the whole `dist/`
  tree is mirrored in `files/`. If a future fixture needs an exclusion
  (e.g. timestamps), encode it as an explicit rule in the harness and
  document it here — never exclude ad hoc.
