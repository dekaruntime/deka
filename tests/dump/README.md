# Conformance dump

Produces the dual-host Hats dump (`hats-results.json`) for
https://testsuite.deka.gg. Native isolate + Chromium Worker. The language
gate is `./run.sh` (tour + snippets + ADHOC). This dump is what the website
fills in (#292 step 2), including the **ADHOC** category.

```sh
cargo build --release -p cli
CARGO_INCREMENTAL=0 cargo build --release \
  --target wasm32-unknown-unknown -p deka_compiler_wasm --no-default-features

cd tests/dump
bun install
bunx playwright install chromium

DEKA_NATIVE=../../target/release/cli \
DEKA_WASM=../../target/wasm32-unknown-unknown/release/deka_compiler_wasm.wasm \
  bun scripts/dump-results.mjs
```

Writes `dist/conformance/hats-results.json`. `scripts/pack-conformance.sh`
bundles that with the pinned tour checkout (`./tour`, from
`deka self fetch tour`) and the pinned testsuite corpus (`./testsuite`) for
R2.

Corpus `.code` sidecars follow dsc's Hats runner: a file whose trimmed body is
a decimal integer is a process exit code, not formatted source (deka#929).
The collector copies fixture-local `.ds`/`.dsx`/`.css`/`.mjs` into the native
tmpdir the way that runner does, so summon fixtures keep their sibling
`foreign.mjs` (deka#930). Wasm still cannot verify summon; those rows stay
listed host gaps.

The native package cache stores installed modules, `deka.lock`, and the optional
`deka.grants.json` together under `.cache/deka-packages/with-grants-v1/`.
Older cache entries are bypassed because they omitted installer-issued grants.
A grant-free install may legitimately have no grants file. The browser host
forwards `envGranted` to web-ide-kit only when the fixture's `deka.json` lists
a non-empty `security.allow.env` (deka#378 / deka#904); otherwise `process` is
absent, matching native.

`expected-failures.txt` ratchets all shared-host divergences: full diagnostic
lists, formatter output, and per-host expectation results. Listed cases remain
visibly divergent in the dump; unlisted divergences and stale entries fail the
ratchet. web-ide-kit 0.3.4 also strips plain `export function`, so
`modules-export-async-fn` now agrees and has been removed from the ratchet.
The remaining deka#904 row is `modules-import-non-relative-001` (documented
per-host string difference; see `docs/dekascript/missing-module-diagnostics.mdx`).
Native auto-installs `io` when the source imports it; wasm project-compiles the
same fixtures against `stdlib-stubs/io.ds` (`echo(message: string)`) so both
hosts record the same diagnostic SET. Dump comparison uses that shared parser
(order-insensitive); the synthesized wasm `error` slot is logging only.
