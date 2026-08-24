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

That builds the CLI, compiles every `tests/tour` lesson, and runs
`tests/testsuite` on the native isolate. See [`TESTING.md`](./TESTING.md).
