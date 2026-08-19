# DekaScript Runtime Execution Suite

Headless execution tests that compile DekaScript fixtures through **both** the
native CLI and the browser WASM compiler, run the emitted JS against the Deka
runtime globals, and assert on stdout / compile diagnostics.

## Why this exists

The website tour tests catch drift, but only after a runtime release has been
built and synced. This suite lives in the runtime repo so we can catch
native/WASM parity bugs and runtime regressions before a version ever reaches
the website.

## Running locally

```bash
cd runtime

# Build the native CLI
cargo build --release -p cli

# Build the browser WASM compiler
CARGO_INCREMENTAL=0 cargo build --release \
  --target wasm32-unknown-unknown -p phpx_compiler_wasm --no-default-features

# Run the suite
bun tests/runtime-suite/run.mjs
```

The harness picks the newest `deka_compiler.wasm` it can find between
`target/wasm32-unknown-unknown/release/phpx_compiler_wasm.wasm` and
`dist/deka-compiler-wasm/deka_compiler.wasm`.

## Running a subset of fixtures

```bash
# List every fixture currently registered
bun tests/runtime-suite/run.mjs --list

# Run only fixtures matching a substring
bun tests/runtime-suite/run.mjs --filter structs
bun tests/runtime-suite/run.mjs --filter option
```

## Adding a fixture

1. Create a `.ds` file in `fixtures/`.
2. Add an entry to `fixtures.json`:
   - `name` — human-readable name
   - `file` — filename in `fixtures/`
   - `expectCompile` — `true` if the fixture should compile
   - `expectStdout` — expected stdout when compilation succeeds
   - `expectError` — substring expected in diagnostics when compilation fails
   - `xfail` — optional reason the test is currently expected to fail

A good workflow when you find a weird tour example: copy the source from the
website into a new `fixtures/<name>.ds` file, set `expectCompile: true` and the
expected output, and run the suite. If it fails on `main`, you have a minimal
reproduction before the bug ever reaches the website.

## Fixture coverage

| Fixture | What it exercises | Status |
|---|---|---|
| `hello` | basic console output | ✓ |
| `typed-functions` | `fn` declarations and type annotations | ✓ |
| `structs` | struct definitions and literals | ✓ |
| `struct-methods` | methods declared on structs | ✓ |
| `embedding` | Go-style struct embedding with method promotion | ✓ |
| `enums` | enum declarations and `match` | ✓ |
| `option-and-result` | builtin `Option`/`Result` globals and unqualified `Some`/`None` | ✓ |
| `optional-field-shorthand` | `T?` optional struct fields | ✓ |
| `option-explicit-type-omitted` | explicit `Option<T>` field omitted defaults to `None` | xfail — left `undefined` at runtime |
| `match-expressions` | literal pattern matching | ✓ |
| `async-await` | async functions | ✓ |
| `jsx` | component rendering | ✓ |
| `jsx-runtime-primitive` | `deka.ui.jsx` primitive tag node factory | ✓ |
| `jsx-runtime-fragment` | `deka.ui.Fragment` node factory | ✓ |
| `jsx-runtime-function-component` | `deka.ui.jsx` with function component tag | ✓ |
| `jsx-runtime-static-children` | `deka.ui.jsxs` static multiple children | ✓ |
| `jsx-runtime-nested-children` | nested children array normalization | ✓ |
| `interfaces` | structural interface satisfaction | ✓ |
| `unsafe-runtime-helper` | `unsafe { ... }` blocks | ✓ |
| `utility-classes` | class strings on JSX elements | ✓ |
