# ADHOC

Scenarios that are not snippet fixtures. `./run.sh` runs them after the
snippet grid. Dump publishes them as category **ADHOC** on testsuite.deka.gg
(cached commands + stdout).

```sh
bun tests/adhoc/run.mjs
bun tests/adhoc/run.mjs --filter serve
DEKA_WASM=path/to/deka_compiler.wasm bun tests/adhoc/run.mjs --filter wasm
```

`deka-init` checks the DekaScript app-router starter: `deka.json` (`mode: ds`),
`app/page.dsx`, root `index.html`, `public/style.css`, and `.gitignore`.
`deka-serve` runs that unmodified starter and checks the home page and stylesheet
return HTTP 200, while `/deka.json` returns 404.
