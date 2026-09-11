# deka

deka is a programming language and runtime for building reliable web applications.
It compiles to JavaScript and runs both natively and in the browser via WASM.

## Quick links

- **Homepage:** https://deka.gg
- **Language tour:** https://deka.gg/tour
- **Documentation:** https://deka.gg/docs
- **Test suite:** https://testsuite.deka.gg
- **Releases:** https://github.com/dekaruntime/deka/releases

## Repository layout

- `crates/` — Rust workspace with the compiler, runtime, CLI, LSP, and WASM targets.
- `assets/` — Shared assets such as the utility CSS bundle.
- `docs/` — Design notes and RFCs.
- `scripts/` — Build and test helpers.
- `tests/` — Integration tests and conformance fixtures.

## Useful docs

- [`PUBLISH.md`](./PUBLISH.md) — How to publish a new runtime release.
- [`STDLIB.md`](./STDLIB.md) — How to version and publish `@deka/*` packages to the index.
- [`TESTING.md`](./TESTING.md) — How the test suites are run.
- [`RELEASE.md`](./RELEASE.md) — Release process checklist.

## Getting started

Build the CLI:

```bash
cargo build --release -p cli
```

Run the language suite:

```bash
./run.sh
```

That builds the CLI, fetches the pinned `dekaruntime/tour` and
`dekaruntime/testsuite` checkouts (`deka self fetch`, into `./tour` and
`./testsuite`), compiles every lesson of the tour, and runs the corpus on the
native isolate. See [`TESTING.md`](./TESTING.md).

## Local package development

Link a package working tree into a consumer without changing `deka.json` or
`deka.lock`:

```bash
deka link ../my-package
deka unlink @scope/my-package
```

Links are stored in the consumer's developer-only `.deka/links.json` (keep
`.deka/` out of version control). During
bundling, a local link takes precedence over the installed copy in `ds_modules`
(and the legacy `php_modules` directory); `deka unlink` removes only that
metadata and never deletes the package working tree. The linked package must
declare a scoped `name` such as `@scope/my-package` in its `deka.json`.
