# Repository Guidelines (Deka)

## Project Layout
- `deka/`: Central monorepo with core implementation
  - `crates/`: 26+ Rust workspace crates (cli, runtime, php-rs compiler)
  - `ds_modules/`: consumer package install directory (`deka add` / `deka install`). `php_modules/` is the legacy name and is still resolved if present.
  - `target/release/cli`: Main CLI binary (113MB ARM64)
  - `target/release/php`: PHP binary
- `deka-runtime/`: Rust runtime binary (`deka-runtime`) + JS/TS bootstrap modules.
- `deka-cli/`: Bun-based CLI (packaged as `deka`).
- `deka-rs/`: Cargo workspace for Deka services (crates under `deka-rs/crates/`).
- `deka-stdio/`: Shared logging/stdio formatting crate (used for runtime logging).
- Published validation crate: Shared error formatting used by runtime.
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

Stdlib packages (`@deka/*`) additionally reach the closed `deka.*` catalog
through `safe { deka.kind.method(...) }` (non-throwing, declared type) and
`unsafe { deka.kind.method(...) }` (`Result<T, string>`); application code
may not. The catalog is validated and lowered by the loader before dsc
compiles — see `docs/dekascript/runtime-bridge.mdx`.

## Service Ports (deka-cli defaults)
- `postgres`: 5432
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
- Built-in modules: `deka/postgres`, `deka/sqlite`, `deka/docker`, `deka/t4`, `deka/jsx-runtime`, etc.
- Node `ws` compatibility: `globalThis.__dekaWs` now re-exports the vendored [`ws`](deka-runtime/src-ts/runtime/vendor/ws) package so framework HMR servers (Vite, etc.) can run without bundling their own `ws`.

## Testing
- Rust: `cargo test` (in `deka-runtime/` or `deka-rs/`).
- Runtime compat suite: `scripts/compat.sh` → `tests/compat/REPORT.md`.
- CLI: `bun test` if present; build via `bun run build`.
- DekaScript compiler core: run the owning parser/emitter crate tests for the
  implemented syntax slice; do not claim a CLI conformance suite yet.

## Documentation Workflow (required)
- Keep `docs/` user-facing only. Put internal plans/task lists under `tasks/` (or `tasks/archive/`).
- After runtime/language/module changes, update docs in the same PR:
  - Language behavior: `docs/dekascript/**`
  - Keep examples current and include expected output for non-trivial features.
- Publish docs from the repo root with:
  ```sh
  node scripts/publish-docs.js --scan . --out ../website/content/docs --force
  ```
- The publish script also runs website runtime-doc bundling automatically.
- Treat documentation as part of feature completeness: if behavior changes, document it before closing the task.


## Conventions & Expectations
- Use `deka-stdio` for runtime logs.
- Prefer helpful validation errors (see the published validation crate).
- Keep imports explicit in JS/TS examples.
- Follow Bun-like ergonomics for runtime APIs where possible (serve/build/run behavior).

## Workspace hygiene (issue-based work)

To avoid worktree/branch pollution in the main project folders, every issue gets
its own fresh clone and a branch named after the issue.

1. Create a working directory named after the repo and issue:
   ```sh
   mkdir -p ~/Projects/work
   cd ~/Projects/work
   git clone git@github.com:dekaruntime/<repo>.git <repo>-issue-<number>
   cd <repo>-issue-<number>
   ```
2. Create a branch with the same name for context:
   ```sh
   git checkout -b agent/ava/issue-<number>
   ```
3. Do the work, commit, push, and open a PR.
4. After the PR merges, delete the directory:
   ```sh
   rm -rf ~/Projects/work/<repo>-issue-<number>
   ```

Do not use `git worktree` inside `~/Projects/deka/` or other canonical repos.
The canonical repos (`~/Projects/deka/`, `~/Projects/deka/website`, etc.) should
stay on `main` and remain clean between tasks.
