# Tour lessons

Canonical DekaScript for deka.gg. The website owns prose; this directory owns
the samples. Match by `id` in `manifest.json`, never by display name.

```
./run.sh
bun tests/tour/run.mjs
```

`./run.sh` is the one-command gate (tour + Hats). Compiles every lesson with
the local CLI (`target/release/cli` or `DEKA_NATIVE`). A language PR that
breaks a lesson fails here.

Every `.dsx` lesson must `export fn Page` (RFD 24 §16): a `.dsx` lesson is
`app/page.dsx`, and the website harness imports the `Page` export. `run.mjs`
asserts this contract and fails a `.dsx` lesson that ends in a bare JSX
expression instead.
