# Public conformance suite

Hats layout. This is the source of truth for https://testsuite.deka.gg.

```
tests/testsuite/<category>/<name>/
  <name>.pass.ds | <name>.fail.ds
  <name>.stdout          # optional exact stdout
  <name>.code            # optional formatter output
  <name>.json            # title, stage, hosts, diagnostics, packages
```

The testsuite **website** repo does not own these files. It displays them
(live browser playground; CACHED RESULTS for native-only cases).

## Run locally

```sh
cargo build --release -p cli
bun tests/testsuite/run.mjs
bun tests/testsuite/run.mjs --filter json
```

Uses `target/release/cli` or `DEKA_NATIVE`. Native isolate only (`deka run`).
Browser/WASM remains the live playground on the site; dump-time browser
results are produced in runtime CI when we publish the results JSON.

Slugs in `native-known-fail.json` are current native mismatches. CI fails on a
new mismatch or an unexpected pass. Rewrite that file with
`--update-known-fail` only when the baseline itself should change.

```
bun tests/testsuite/run.mjs --list
bun tests/testsuite/run.mjs --filter json
bun tests/testsuite/run.mjs --jobs 4
```

See deka#292.
