# Conformance dump

Produces the dual-host Hats dump (`hats-results.json`) for
https://testsuite.deka.gg. Native isolate + Chromium Worker. The language
gate is `./run.sh` (native only). This dump is what the website fills in
(#292 step 2).

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
bundles that with `tests/tour` and `tests/testsuite` for R2.
