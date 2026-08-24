# Testing Deka

This repo owns the language tests. Layout:

- Rust unit/integration tests in `crates/`
- `tests/testsuite/` — public Hats fixtures (the contract behind https://testsuite.deka.gg)
- `tests/tour/` — canonical DekaScript samples for deka.gg, matched by `id`
- `tests/runtime-suite/` — smaller native+WASM execution suite (merge into `tests/testsuite` or delete; deka#292)

The **testsuite website** (`dekaruntime/testsuite`) displays these fixtures. It
does not own them. Live browser edit/run stays on that site; native-only /
packages / recorded-only cases show CACHED RESULTS from the last dump.
See [RFD 26](https://github.com/dekaruntime/rfd/issues/26) and deka#292.

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

## Public conformance suite (`tests/testsuite`)

Hats folders. This is what a language PR must not break on the native isolate.

```
tests/testsuite/<category>/<name>/
  <name>.pass.ds | <name>.fail.ds
  <name>.stdout          # optional exact stdout
  <name>.code            # formatter output (checked on the website dump, not here)
  <name>.json            # title, stage, hosts, diagnostics, packages
```

```bash
cargo build --release -p cli
bun tests/testsuite/run.mjs
bun tests/testsuite/run.mjs --filter json
bun tests/testsuite/run.mjs --list
```

Uses `target/release/cli` or `DEKA_NATIVE`. Native isolate only (`deka run`).
Display names are never keys; the runner matches by slug (`category-name`).

Some fixtures still mismatch native on current main (JSX isolate, stale
`JSON` / `deka.unsafe` samples, published package sources, …). Those slugs live
in `tests/testsuite/native-known-fail.json`. CI fails on a **new** mismatch or
an unexpected pass. When you fix a fixture or the runtime, remove its slug.

## Tour lessons (`tests/tour`)

Canonical DekaScript for deka.gg. The website owns prose; this directory owns
the samples. Match by `id` in `manifest.json`, never by display name.

```bash
bun tests/tour/run.mjs
bun tests/tour/run.mjs --filter structs
```

Compiles every lesson with the local CLI. A language PR that breaks a lesson
fails here (and in `deka_compiler_wasm` tests, which load the same files).

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

New language tests belong in `tests/testsuite/` (Hats) or `tests/tour/`
(website samples). Add a `runtime-suite` fixture only if you need native+WASM
parity on a case that is not in the public suite yet.

## Testing a runtime checkout against the public suite website

`bun tests/testsuite/run.mjs` is the in-tree native gate. The website at
https://testsuite.deka.gg still dumps **both** hosts (native isolate and
Chromium Worker) until deka#292 steps 2–3 land (release uploads the dump;
testsuite CI only fills in). To reproduce that dual-host dump from a
`dekaruntime/testsuite` checkout:

### Running it

```bash
git clone git@github.com:dekaruntime/testsuite.git
cd testsuite
./run.sh /path/to/your/deka/checkout
```

That is the whole procedure. `run.sh` installs dependencies and Chromium if
missing, builds the native CLI and the wasm compiler from the checkout you name,
runs every fixture against both hosts, and writes `.cache/report.txt`.

```bash
./run.sh ~/Projects/deka   # test that checkout
./run.sh                   # same, auto-detecting ($DEKA_REPO, ../deka, ...)
./run.sh --published       # test the released compilers instead
```

It reports on the runtime you point it at. Failures and native/browser
divergences are findings to read, not a pass/fail verdict. Exit `0` means the
suite ran; exit `2` means the environment could not support a run, which is
never a statement about your runtime.

The setup is deliberately not left to the reader. Chromium in particular is a
separate download from the npm package, and without it the browser host drops
and the suite reports zero divergences no matter what the browser compiler does
-- a run that looks *healthier* than a correct one. Both compilers are rebuilt
every run because cargo is incremental and it is the only way to guarantee the
two hosts came from one source.

### What preflight asserts, and why each one exists

| Check | The failure it prevents |
|---|---|
| `DEKA_NATIVE`/`DEKA_WASM` both set or both unset | Mixing a local host with a published one renders type-name drift (`int` vs `number`) as native/browser disagreement |
| native binary matches its own tree | A stale `target/release/cli` tests your checkout with an old compiler and invents divergences |
| playwright resolvable | Without it the browser host drops and every run reports zero divergences |
| both hosts available | A host that did not run cannot be reported on, so the harness refuses rather than publishing half a result |

Every one of these has produced a confident wrong answer in practice. A broken
environment does not look broken -- it looks like a clean run with a number you
would quote.

Note `[hats build] wasm compiler version=` comes from the published CDN
manifest. When testing a local checkout it says so explicitly rather than
implying the artifact under test carries that version.

## Browser compiler WASM smoke test

CI also runs a dedicated WASM build/test script:

```bash
DEKA_SKIP_DIRTY_CHECK=1 scripts/test-deka-compiler-wasm.sh
```

This builds the browser compiler and runs its own in-WASM tests.
