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

The native package cache stores installed modules, `deka.lock`, and the optional
`deka.grants.json` together under `.cache/deka-packages/with-grants-v1/`.
Older cache entries are bypassed because they omitted installer-issued grants.
A grant-free install may legitimately have no grants file.

`expected-failures.txt` ratchets all shared-host divergences: full diagnostic
lists, formatter output, and per-host expectation results. Listed cases remain
visibly divergent in the dump; unlisted divergences and stale entries fail the
ratchet. The c5 entries preserve native-correct assertions pending adjudication;
they do not turn browser bugs into passing expectations.
