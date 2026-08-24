# Testing Deka

One language suite. This repo owns it. The other two repos are delivery
vehicles. See [deka#292](https://github.com/dekaruntime/deka/issues/292) and
[RFD 26](https://github.com/dekaruntime/rfd/issues/26).

| Repo | Owns | Does not own |
|---|---|---|
| **deka** (this repo) | Every language test. `tests/testsuite/` (Hats folders: the public contract), `tests/tour/` (samples deka.gg displays), plus Rust / WASM / `tests/runtime-suite`. | The testsuite.deka.gg UI, tour markdown |
| **testsuite** | The website: grid, live browser playground, **CACHED RESULTS** for native-only / packages / recorded-only. | Fixture sources. After #292 steps 2–3, CI does **not** re-run the suite. |
| **website** | Lesson prose, titles, section order. CI: pinned WASM + tour sources compile, pages are not garbage. | The language. No second runtime suite. |

`tests/testsuite` **is** the suite. Display names are never keys — match by
stable id / slug. `tests/runtime-suite` should merge into `tests/testsuite` or
go away; do not add new language cases there.

## The loop

```bash
./run.sh
./run.sh --filter json
./run.sh --list
```

That is the testsuite-repo `./run.sh` equivalent for this tree. It finds bun,
builds `target/release/cli` (or uses `DEKA_NATIVE`), compiles every tour
lesson, then runs Hats natively. Same files CI runs. Fail locally. Do not
discover a language break on a Hats deploy.

Native isolate only (`deka run`). Browser WASM stays the live playground on the
site; dump-time browser results are a website concern until this repo publishes
a dump (#292 step 2). The full log lands in `.cache/report.txt`.

The bun commands below are what `./run.sh` invokes, for a subset or a rerun
when the CLI is already built.

## Pipelines

1. **deka CI** — `tests/tour` + `tests/testsuite` (native). A language PR that
   breaks a fixture is red here.
2. **deka release** *(not landed yet)* — upload the fixture tree + a results
   dump next to the compiler. Compiler and tests are one version.
3. **testsuite CI** *(still dumps both hosts today)* — after step 2: **fill in**
   tree + dump, `next build`, deploy. No second `deka run` of 620 cases. No
   Chromium for conformance.
4. **website** *(still has in-repo copies today)* — after step 4: sync tour
   sources **by id** with the compiler artifact. `curriculum.ts` has no `.ds`
   text.

Until steps 2–3 land, https://testsuite.deka.gg still dumps native + Chromium
Worker itself. That dump is not the language gate. This tree is.

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

`cargo test -p php-rs` is deliberately absent from CI. It has a large number of
pre-existing failures; do not treat its local red as a regression without
diffing failure **names** against a clean `origin/main` worktree.

To reproduce CI's exact sequence, use the list in [`CI.md`](./CI.md), not the
crate list above. CI covers `deka_compiler_wasm` via
`scripts/test-deka-compiler-wasm.sh`, then builds the CLI and runs
`tests/runtime-suite`, `tests/tour`, and `tests/testsuite`.

## Public conformance suite (`tests/testsuite`)

Hats folders. Source of truth for https://testsuite.deka.gg.

```
tests/testsuite/<category>/<name>/
  <name>.pass.ds | <name>.fail.ds
  <name>.stdout          # optional exact stdout
  <name>.code            # formatter output (website dump, not the native runner)
  <name>.json            # title, stage, hosts, diagnostics, packages
```

```bash
cargo build --release -p cli
bun tests/testsuite/run.mjs
bun tests/testsuite/run.mjs --filter json
bun tests/testsuite/run.mjs --list
bun tests/testsuite/run.mjs --jobs 4
```

Uses `target/release/cli` or `DEKA_NATIVE`. Native isolate only. The runner
matches by slug (`category-name`), never by title. Fixtures with `hosts` that
do not include `native` are skipped (today: the browser-only Worker case).

### Known native mismatches

Hats is a diagnostic grid. Pink cells on the site are host disagreement at dump
time, not a CI verdict. On this tree some fixtures still mismatch native (JSX
isolate `ui.jsx`, stale `JSON` / `deka.unsafe` samples, published packages
still using `): T`, diagnostic-text drift, a few Node traces that leaked into
`.json`). Those slugs live in `tests/testsuite/native-known-fail.json`.

- CI fails on a **new** mismatch or an **unexpected pass**.
- When you fix a fixture or the runtime, remove its slug.
- Rewrite the file with `--update-known-fail` only when the baseline itself
  should change.

### Adding a Hats fixture

1. Create `tests/testsuite/<category>/<name>/`.
2. Add `<name>.pass.ds` or `<name>.fail.ds`.
3. Add `<name>.json` with `title`, `stage` (`parse` / `typecheck` / `run`), and
   optional `hosts`, `expectedDiagnosticContains`, `packages`.
4. For a passing run test, add `<name>.stdout` with exact native stdout.
5. Run `bun tests/testsuite/run.mjs --filter <name>`.

Multi-file cases put extra `.ds` modules next to the entry file. The entry must
sit at the top of the folder (`*.pass.ds` / `*.fail.ds` with no `/` in the
relative path). Metadata is read from `<entry-basename>.json` (for
`main.pass.ds` that is `main.json`).

## Tour lessons (`tests/tour`)

Canonical DekaScript for deka.gg. The website owns prose; this directory owns
the samples. Match by `id` in `manifest.json`, never by display name.

```bash
bun tests/tour/run.mjs
bun tests/tour/run.mjs --filter structs
bun tests/tour/run.mjs --list
```

Compiles every lesson with the local CLI. A language PR that breaks a lesson
fails here. `deka_compiler_wasm` unit tests and `browser-parity.mjs` load the
same files.

### Adding a lesson

1. Pick a stable `id` (slug, not a title). Website routes may alias it later.
2. Add `tests/tour/<id>.ds`.
3. Append an entry to `tests/tour/manifest.json`:
   - `id` — the filename stem
   - `title` — display only
   - `expectCompile` — `true` / `false`
   - `expectError` — substring of the diagnostic when compile must fail
4. Run `bun tests/tour/run.mjs --filter <id>`.

An `.ds` file without a manifest row (or the reverse) is a hard error.

## DekaScript runtime execution suite (`tests/runtime-suite`)

Older native+WASM harness. Compiles each fixture through the native CLI **and**
the browser WASM compiler, runs the emitted JS against Deka runtime globals, and
asserts on stdout / compile diagnostics.

Do not add new language coverage here. Put it in `tests/testsuite` or
`tests/tour`. Keep this suite green until it is folded in or deleted.

```bash
cargo build --release -p cli

CARGO_INCREMENTAL=0 cargo build --release \
  --target wasm32-unknown-unknown -p deka_compiler_wasm --no-default-features

bun tests/runtime-suite/run.mjs
bun tests/runtime-suite/run.mjs --list
bun tests/runtime-suite/run.mjs --filter structs
```

The harness picks the newest `deka_compiler.wasm` it can find between
`target/wasm32-unknown-unknown/release/deka_compiler_wasm.wasm` and
`dist/deka-compiler-wasm/deka_compiler.wasm`.

## Browser compiler WASM smoke test

```bash
DEKA_SKIP_DIRTY_CHECK=1 scripts/test-deka-compiler-wasm.sh
```

Builds the browser compiler, runs its in-crate tests (including every
`tests/tour` lesson against the WASM ABI), then `browser-parity.mjs`.

## Dual-host dump (testsuite website, transitional)

`bun tests/testsuite/run.mjs` is the in-tree native gate. Until #292 steps 2–3
land, the published site still compiles and runs **both** hosts: native isolate
(`deka run`) and a Chromium Worker. Node is not an execution host.

That dump reports on the runtime you point it at. Failures and native/browser
divergences are findings to read, not a pass/fail verdict. Exit `0` means the
suite ran; exit `2` means the environment could not support a run.

```bash
git clone git@github.com:dekaruntime/testsuite.git
cd testsuite
./run.sh /path/to/your/deka/checkout
./run.sh                   # auto-detect ($DEKA_REPO, ../deka, ...)
./run.sh --published       # released compilers instead
```

`run.sh` installs dependencies and Chromium if missing, builds both compilers
from the checkout you name, and writes `.cache/report.txt`. Chromium is a
separate download from the npm package. Without it the browser host drops and
the suite reports zero divergences — a run that looks *healthier* than a
correct one.

### What preflight asserts, and why each one exists

| Check | The failure it prevents |
|---|---|
| `DEKA_NATIVE`/`DEKA_WASM` both set or both unset | Mixing a local host with a published one renders type-name drift (`int` vs `number`) as native/browser disagreement |
| native binary matches its own tree | A stale `target/release/cli` tests your checkout with an old compiler and invents divergences |
| playwright resolvable | Without it the browser host drops and every run reports zero divergences |
| both hosts available | A host that did not run cannot be reported on, so the harness refuses rather than publishing half a result |

Every one of these has produced a confident wrong answer in practice. A broken
environment does not look broken — it looks like a clean run with a number you
would quote.

Note `[hats build] wasm compiler version=` comes from the published CDN
manifest. When testing a local checkout it says so explicitly rather than
implying the artifact under test carries that version.
