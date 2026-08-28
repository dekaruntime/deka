# Deka monorepo — developer guide

This is the day-to-day guide for working in `dekaruntime/deka`. For release processes see `PUBLISH.md` and `RELEASE.md`; for test details see `TESTING.md`; for CI and self-hosted runner debugging see `CI.md`; for agent workspace rules see `AGENTS.md`.

## What this repo is

DekaScript compiler, native runtime, CLI, LSP, and WASM compiler. It compiles `.ds` files to JavaScript and runs them natively (Deno/V8) or in the browser (WASM).

Downstream repos you will touch regularly:

| Repo | Purpose | When you change the runtime here |
|---|---|---|
| `dekaruntime/website` | `deka.gg` homepage + tour | Update WASM artifacts and redeploy |
| `dekaruntime/testsuite` | `testsuite.deka.gg` diagnostic grid (live browser playground) | Fixtures live in `tests/testsuite/` here. Until deka#292 step 3, the site still dumps both hosts itself. |
| `dekaruntime/web-ide-kit` | Shared editor/runtime components used by both sites | Publish to npm, bump consumers |

## Quick start

```bash
# Build the native CLI (release builds are the canonical dev binary)
cargo build --release -p cli

# Check the main crates
cargo check -p deka_compile -p deka_emit -p modules_php -p php-rs -p deka-fmt -p runtime

# Run the main Rust tests
cargo test -p deka_compile -p deka_emit -p modules_php -p php-rs -p deka-fmt -p runtime
```

## Repository layout

```
Cargo.toml              # workspace root
crates/
  cli/                  # native CLI (`deka run`, `deka build`, `deka transpile`)
  deka_compile/         # compiler v2 driver (parse + typecheck + module graph)
  deka_emit/            # JS emitter (DekaScript → JS)
  deka_compiler_wasm/   # browser compiler WASM target
  deka-fmt/             # DekaScript formatter
  deka_lsp/             # native LSP
  deka-validation/      # error formatting / diagnostics
  modules_php/          # parser + validation (shared PHPX/DS parser)
  php-rs/               # typechecker (`phpx/typeck/check/`)
  runtime/              # native runtime (Deno/V8 isolate execution)
  runtime_core/         # module resolution, security policy
docs/                   # RFCs and design notes
tests/                  # integration tests
scripts/                # build/test helpers
assets/                 # shared CSS bundle
```

## Key crates for language work

| If you are changing... | Look here |
|---|---|
| Parser / syntax | `crates/modules_php/src/parser/`, `crates/php-rs/src/parser/` |
| Typechecker | `crates/php-rs/src/phpx/typeck/check/` |
| JS emission | `crates/deka_emit/src/emit.rs` |
| Formatter | `crates/deka-fmt/src/ds.rs`, `crates/deka-fmt/src/js.rs` |
| Validation / diagnostics | `crates/modules_php/src/validation/`, `crates/deka-validation/` |
| Native execution | `crates/runtime/src/`, `crates/cli/src/` |
| WASM compiler | `crates/deka_compiler_wasm/src/` |
| Module resolution | `crates/runtime_core/src/modules.rs`, `crates/modules_php/src/validation/modules.rs` |

## Common commands

### Native CLI

```bash
# Run a .ds file with the native runtime
./target/release/cli run path/to/file.ds

# Transpile to stdout
./target/release/cli transpile path/to/file.ds

# Typecheck only
./target/release/cli check path/to/file.ds
```

### WASM compiler

```bash
# Build the browser compiler (this is what the tour/testsuite download)
CARGO_INCREMENTAL=0 cargo build --release \
  --target wasm32-unknown-unknown \
  -p deka_compiler_wasm \
  --no-default-features

# Smoke test the WASM build
DEKA_SKIP_DIRTY_CHECK=1 scripts/test-deka-compiler-wasm.sh
```

### Runtime execution suite

```bash
# Language gate: tour lessons + Hats (native isolate). Builds the CLI.
./run.sh
./run.sh --filter structs
```

### Formatter

```bash
# Format a .ds file (uses the deka-fmt crate via cargo run)
cargo run --release -p deka-fmt -- path/to/file.ds
```

## End-to-end workflow for a language change

1. **Make the change** in the relevant crate(s).
2. **Run Rust tests:**
   ```bash
   cargo test -p deka_compile -p deka_emit -p modules_php -p php-rs -p deka-fmt -p runtime
   cargo test -p cli --lib -- --test-threads=1
   ```
3. **Run the in-tree language suite**, then WASM parity if you touched emit:
   ```bash
   ./run.sh
   CARGO_INCREMENTAL=0 cargo build --release --target wasm32-unknown-unknown -p deka_compiler_wasm --no-default-features
   DEKA_SKIP_DIRTY_CHECK=1 scripts/test-deka-compiler-wasm.sh
   ```
4. **Bump crate versions** and open a PR if the change is user-facing.
5. **After merge**, cut a release tag to push artifacts to R2 and trigger downstream site rebuilds (see `PUBLISH.md`). `@deka/*` packages are a different pipeline (`STDLIB.md`): merge does not publish them.
6. **Language fixtures** are in `tests/testsuite/` and `tests/tour/` in this repo. The dual-host dump is `tests/dump`; a release uploads it. Testsuite and website ingest that pack.

## How downstream sites consume the runtime

The release workflow publishes to:

- `https://wasm.deka.gg/latest/deka_compiler.wasm`
- `https://wasm.deka.gg/latest/deka-compiler-artifact.json`
- `https://wasm.deka.gg/latest/deka_diagnostics.wasm`
- `https://wasm.deka.gg/latest/deka-diagnostics-artifact.json`
- `https://releases.deka.gg/latest.json`

The website and testsuite download these at build time. If you need a site to pick up a runtime change before a release, you can manually dispatch:

```bash
gh workflow run sync-deka-compiler.yml --repo dekaruntime/website --ref main
gh workflow run "Deploy deka test suite" --repo dekaruntime/testsuite --ref main
```

## Useful checks before opening a PR

```bash
# Fast compile check of the whole language stack
cargo check -p deka_compile -p deka_emit -p modules_php -p php-rs -p deka-fmt -p runtime -p cli

# Full Rust test stack (excluding WASM browser build)
cargo build -p deka_http -p pool -p engine -p php-rs -p bundler
cargo test -p deka_http -p pool -p engine -p php-rs -p bundler
cargo test -p cli --lib -- --test-threads=1
```

## Troubleshooting

### `cargo check` fails with workspace profile warnings

```
warning: profiles for the non root package will be ignored
```

This is benign; profiles must be defined at the workspace root. Do not add profiles to individual crate `Cargo.toml` files.

### CLI tests fail only when run in parallel

CLI tests mutate process-global state. Run them single-threaded:

```bash
cargo test -p cli --lib -- --test-threads=1
```

### Testsuite shows `nativeAvailable: false` or `browserAvailable: false`

The public suite dumps two Deka hosts: `deka run` (isolate) and a Chromium
Worker. If either flag is false, that column was skipped — not compared in
Node. See `PUBLISH.md`. Branch dumps must set **both** `DEKA_NATIVE` and
`DEKA_WASM` to the same build or type-name drift will look like host drift.

### WASM compiler tests fail with "dirty" check

The WASM smoke test refuses to run if the working tree is dirty. Use:

```bash
DEKA_SKIP_DIRTY_CHECK=1 scripts/test-deka-compiler-wasm.sh
```

### Formatter changes behave differently in testsuite vs tour

Both sites use `@dekaruntime/web-ide-kit`. The formatter is invoked through that package. If you change `deka-fmt`, you must bump `web-ide-kit`, publish it, and bump the consumer sites for the change to appear online.

## Standards and conventions

- `.ds` is the only public source extension. PHPX (`.phpx`) is legacy and being removed.
- Release builds are canonical; do not rely on `target/debug` binaries.
- Keep error messages helpful; `deka-validation` owns formatting.
- Document language behavior in `docs/dekascript/` when it changes.
