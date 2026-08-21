# Testing Deka

This repo has two test layers: Rust unit/integration tests in `runtime/`, and the
DekaScript runtime execution suite that runs the same fixtures through both the
native CLI and the browser WASM compiler.

## Prerequisites

- Rust toolchain (`rust-toolchain` file pins the version; currently 1.96.0)
- `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- [Bun](https://bun.sh) (1.3.14 or later)

## Rust tests (`cargo test`)

Run from the `runtime/` directory:

```bash
cd runtime

# Build the crates exercised by the test suite
cargo build -p deka_http -p pool -p engine -p deka_js -p php-rs -p bundler

# Run individual crate tests
cargo test -p deka_http
cargo test -p pool
cargo test -p engine
cargo test -p deka_js        # PHPX compiler, including integration tests
cargo test -p php-rs         # parser + typechecker
cargo test -p bundler

# CLI tests must run single-threaded because some tests mutate process-global state
cargo test -p cli --lib -- --test-threads=1
```

To run everything the CI runs (excluding the WASM browser build and the runtime
execution suite):

```bash
cd runtime
cargo build -p deka_http -p pool -p engine -p deka_js -p php-rs -p bundler
cargo test -p deka_http
cargo test -p pool
cargo test -p engine
cargo test -p deka_js
cargo test -p php-rs
cargo test -p bundler
cargo test -p cli --lib -- --test-threads=1
```

## DekaScript runtime execution suite

The execution suite lives in `runtime/tests/runtime-suite/`. It compiles each
fixture through the native CLI and the browser WASM compiler, runs the emitted
JS with the Deka runtime globals, and asserts on stdout or compile diagnostics.

Build the native CLI and the browser WASM compiler, then run the suite:

```bash
cd runtime

# Native CLI (release build, as used by the harness)
cargo build --release -p cli

# Browser WASM compiler
CARGO_INCREMENTAL=0 cargo build --release \
  --target wasm32-unknown-unknown -p deka_compiler_wasm --no-default-features

# Run the full suite
bun tests/runtime-suite/run.mjs
```

The harness picks the newest `deka_compiler.wasm` it can find between
`target/wasm32-unknown-unknown/release/deka_compiler_wasm.wasm` and
`dist/deka-compiler-wasm/deka_compiler.wasm`.

### Running a subset

```bash
# List every fixture currently registered
bun tests/runtime-suite/run.mjs --list

# Run only fixtures matching a substring of their display name
bun tests/runtime-suite/run.mjs --filter structs
bun tests/runtime-suite/run.mjs --filter option
```

### Adding a fixture

1. Create a `.ds` file in `runtime/tests/runtime-suite/fixtures/`.
2. Add an entry to `runtime/tests/runtime-suite/fixtures.json`:
   - `name` — human-readable name
   - `file` — filename in `fixtures/`
   - `expectCompile` — `true` if the fixture should compile
   - `expectStdout` — expected stdout when compilation succeeds
   - `expectError` — substring expected in diagnostics when compilation fails
   - `xfail` — optional reason the test is currently expected to fail

When you hit a weird tour example, copy the source into a new fixture, set the
expected output, and run the suite. If it fails on `main`, you have a minimal
reproduction before the bug reaches the website.

## Browser compiler WASM smoke test

CI also runs a dedicated WASM build/test script:

```bash
DEKA_SKIP_DIRTY_CHECK=1 runtime/scripts/test-deka-compiler-wasm.sh
```

This builds the browser compiler and runs its own in-WASM tests.
