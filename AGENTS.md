# Repository Guidelines (Deka)

## Project Layout
- `deka/`: Central monorepo with core implementation
  - `crates/`: 26+ Rust workspace crates (cli, runtime, php-rs compiler)
  - `php_modules/`: legacy implementation substrate; do not document it as a public language surface
  - `target/release/cli`: Main CLI binary (113MB ARM64)
  - `target/release/php`: PHP binary
- `deka-runtime/`: Rust runtime binary (`deka-runtime`) + JS/TS bootstrap modules.
- `deka-cli/`: Bun-based CLI (packaged as `deka`).
- `deka-rs/`: Cargo workspace for Deka services (crates under `deka-rs/crates/`).
- `deka-stdio/`: Shared logging/stdio formatting crate (used for runtime logging).
- `deka-validation/`: Validation/error formatting shared by runtime.
- `deka-dashboard/`, `deka-website/`: UI apps.

## Runtime Entry Points (deka-runtime)
- **Run once (default)**: `deka <file>` or `deka-runtime run <file>` executes a TS/JS module and exits.
- **Serve**: `deka serve <file>` starts the HTTP server and routes requests to the handler.
- **Build**: `deka --build <entry> --outdir <dir>` bundles frontend assets.

Default HTTP port: `3000` (override via `PORT` env variable).

## CLI Commands (deka-cli)
Core ops (from `deka-cli/README.md`):
- `deka setup`, `deka start`, `deka stop`, `deka status`, `deka logs`, `deka restart`, `deka check`, `deka update`, `deka upgrade`, `deka monitor start`.
Container ops:
- `deka c ps`, `deka c run`, `deka c exec`, `deka c attach`, `deka c rm`.

Runtime helpers:
- `deka run <file>`: execute a runtime module and exit.
- `deka serve <file>`: start a runtime server.
- `deka output <file>`: run a handler once and print response body.
- `deka build <entry>`: bundle frontend assets.
- `deka test [files...]`: run runtime tests (optionally `--no-rust`).
- `deka introspect`: inspect runtime scheduler state.

DekaScript compiler-core status:
- `.ds` is the only public source extension.
- The currently stacked compiler slice covers parsing and JS emission only.
- Do not document or rely on public `run`, `serve`, `build`, routing, editor,
  Wasm, import, or broad-stdlib behavior until its owning lane lands it.

Note: `deka run` executes runtime modules; container commands live under `deka c ...` (or `deka container ...`).

## Local CLI Wiring (dev loop)
- The local `deka` command is wired to `deka/target/release/cli` for this repo.
- Build policy: release-only builds for this repo. Do not build or rely on `target/debug` binaries.
- After Rust changes: `cargo build --release -p cli`
## DekaScript compiler-core syntax

```ts
export function initials(parts: Array<string>): string {
  let output = "";
  for (const part of parts) output += part.slice(0, 1);
  return `${parts[0]}:${output}`;
}
```

This is the supported compiler-core slice: typed bare parameters, `const`/`let`,
objects/lists, property/index access, string methods, template strings, and
`for (const item of items)`. PHP-derived syntax is rejected. The module and
runtime contracts are not available from this slice.

## Service Ports (deka-cli defaults)
- `postgres`: 5432
- `redis`: 6379
- `edge` (runtime): 8506
- `t4`: 8507
- `gild-vcs`: 8508
- `deploy`: 8509

## DekaScript module system

No public module-resolution or standard-library contract is available in the
current compiler-core stack. Do not add examples or compatibility aliases until
the owning runtime/stdlib lanes define and validate one.

## Runtime Features (deka-runtime)
- V8 isolate pool with warm caching.
- JS/TS module loader + SWC transforms.
- User-land router (`deka/router`) + `serve()` API.
- Introspection endpoints exposed via `serve({ introspect: true })` (default prefix `/_introspect`).
- Built-in modules: `deka/postgres`, `deka/sqlite`, `deka/docker`, `deka/t4`, `deka/redis`, `deka/jsx-runtime`, etc.
- Node `ws` compatibility: `globalThis.__dekaWs` now re-exports the vendored [`ws`](deka-runtime/src-ts/runtime/vendor/ws) package so framework HMR servers (Vite, etc.) can run without bundling their own `ws`.

## Testing
- Rust: `cargo test` (in `deka-runtime/` or `deka-rs/`).
- Runtime compat suite: `deka-runtime/scripts/compat.sh` → `deka-runtime/test/compat/REPORT.md`.
- CLI: `bun test` if present; build via `bun run build`.
- DekaScript compiler core: run the owning parser/emitter crate tests for the
  implemented syntax slice; do not claim a CLI conformance suite yet.

## Documentation Workflow (required)
- Keep `runtime/docs/` user-facing only. Put internal plans/task lists under `tasks/` (or `tasks/archive/`).
- After runtime/language/module changes, update docs in the same PR:
  - Language behavior: `runtime/docs/dekascript/**`
  - Keep examples current and include expected output for non-trivial features.
- Publish docs from `runtime/` with:
  ```sh
  node runtime/scripts/publish-docs.js --scan . --out ../website/content/docs --force
  ```
- The publish script also runs website runtime-doc bundling automatically.
- Treat documentation as part of feature completeness: if behavior changes, document it before closing the task.


## Conventions & Expectations
- Use `deka-stdio` for runtime logs.
- Prefer helpful validation errors (see `deka-validation`).
- Keep imports explicit in JS/TS examples.
- Follow Bun-like ergonomics for runtime APIs where possible (serve/build/run behavior).
