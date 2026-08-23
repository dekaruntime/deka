# Testing Deka

This repo has two in-tree test layers: Rust unit/integration tests, and
`tests/runtime-suite/` (fixtures through the local CLI and WASM compiler).

The **public diagnostic suite** is `dekaruntime/testsuite` (https://testsuite.deka.gg).
It runs fixtures on the native isolate (`deka run`) and in a Chromium Worker.
That is not Node. See [RFD 26](https://github.com/dekaruntime/rfd/issues/26).

## Prerequisites

- Rust toolchain (`rust-toolchain` file pins the version; currently 1.96.0)
- `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- [Bun](https://bun.sh) (1.3.14 or later)

## Rust tests (`cargo test`)

Run from the repo root:

```bash
# Build the crates exercised by the test suite
cargo build -p deka_http -p pool -p engine -p deka_js -p php-rs -p bundler

# Run individual crate tests
cargo test -p deka_http
cargo test -p pool
cargo test -p engine
cargo test -p deka_js        # PHPX compiler, including integration tests
cargo test -p php-rs         # parser + typechecker
cargo test -p bundler
cargo test -p deka_compiler_wasm   # browser compiler; CI runs this via scripts/test-deka-compiler-wasm.sh

# CLI tests must run single-threaded because some tests mutate process-global state
cargo test -p cli --lib -- --test-threads=1
```

To run everything the CI runs (excluding the WASM browser build and the runtime
execution suite):

```bash
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

The execution suite lives in `tests/runtime-suite/`. It compiles each
fixture through the native CLI and the browser WASM compiler, runs the emitted
JS with the Deka runtime globals, and asserts on stdout or compile diagnostics.

Build the native CLI and the browser WASM compiler, then run the suite:

```bash
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

1. Create a `.ds` file in `tests/runtime-suite/fixtures/`.
2. Add an entry to `tests/runtime-suite/fixtures.json`:
   - `name` — human-readable name
   - `file` — filename in `fixtures/`
   - `expectCompile` — `true` if the fixture should compile
   - `expectStdout` — expected stdout when compilation succeeds
   - `expectError` — substring expected in diagnostics when compilation fails
   - `xfail` — optional reason the test is currently expected to fail

When you hit a weird tour example, copy the source into a new fixture, set the
expected output, and run the suite. If it fails on `main`, you have a minimal
reproduction before the bug reaches the website.

## Before merging a runtime change: the conformance gate

The in-tree suites above test this repo against itself. They cannot tell you
whether a compiler change breaks the **published** surface — the native isolate
and the browser Worker running the same fixture. That is what
`dekaruntime/testsuite` is for, and it is runnable locally in one command.

Run it for any change to the compiler, the typechecker, diagnostics, or the
emitted JS.

### One-time setup

```bash
git clone git@github.com:dekaruntime/testsuite.git
cd testsuite
bun install
bunx playwright install chromium    # the browser host; without it you get a
                                    # native-only run that still looks healthy
```

That Chromium step is not optional and is easy to miss — CI installs it
explicitly in its deploy workflow, so a local checkout is the only place the
browser host silently goes absent.

### Running it

Build both hosts **from the same commit**, then run the gate:

```bash
# in deka/
cargo build --release -p cli
CARGO_INCREMENTAL=0 cargo build --release \
  --target wasm32-unknown-unknown -p deka_compiler_wasm --no-default-features

# in testsuite/
DEKA_NATIVE=../deka/target/release/cli \
DEKA_WASM=../deka/target/wasm32-unknown-unknown/release/deka_compiler_wasm.wasm \
  bun scripts/run-tests.mjs --gate
```

The gate does four things: verifies the environment, runs the native host, runs
the browser host, and writes a report to `.cache/gate-report.txt`.

Exit codes: `0` clean, `1` a new failure, `2` the environment is not fit to
grade (fix it and re-run — a `2` is never a verdict on your change).

### What preflight asserts, and why each one exists

| Check | The failure it prevents |
|---|---|
| `DEKA_NATIVE`/`DEKA_WASM` both set or both unset | Pairing a local host against a published one renders type-name drift (`int` vs `number`) as native/browser disagreement |
| native binary matches its own tree | A stale `target/release/cli` grades your change with an old compiler and invents divergences |
| playwright resolvable | A missing dev dependency drops the browser host; the run then reports zero divergences no matter what the browser compiler does |
| both hosts available | A host that did not run cannot be graded, so the gate refuses rather than half-grading |

Every one of these has produced a confident wrong answer in practice. The point
of preflight is that a broken environment does not look broken — it looks like a
clean run with a number you would quote.

Note `[hats build] wasm compiler version=` is read from the published CDN
manifest, not from the compiler actually loaded. With `DEKA_WASM` set it does
**not** describe your artifact. Trust preflight's version line instead.

### The baseline

The suite carries known failures, so a raw pass/fail count carries no signal.
`tests/baseline.txt` holds the known-failing and known-divergent fixture ids,
and the gate gates on **new names**, not on zero failures.

```bash
bun scripts/run-tests.mjs --write-baseline   # accept current state
```

Only regenerate after reading the diff. A fixture that starts failing is either
a regression or an intended change, and only a human can tell those apart. A
baseline recorded without both hosts is invalid — it bakes in a single-host view
and masks every browser-side regression.

## Browser compiler WASM smoke test

CI also runs a dedicated WASM build/test script:

```bash
DEKA_SKIP_DIRTY_CHECK=1 scripts/test-deka-compiler-wasm.sh
```

This builds the browser compiler and runs its own in-WASM tests.
