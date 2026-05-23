# Repository Guidelines (Deka)

## Project Layout
- `deka/`: Central monorepo with core implementation
  - `crates/`: 26+ Rust workspace crates (cli, runtime, php-rs compiler)
  - `php_modules/`: PHPX standard library and language implementation
  - `target/release/cli`: Main CLI binary (113MB ARM64)
  - `target/release/php`: PHP binary with PHPX integration
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

PHPX operations:
- `deka run <file.phpx>`: execute PHPX file directly
- `deka serve <file.phpx>`: serve PHPX web application
- `deka compile <file.phpx>`: compile PHPX to PHP (outputs to .cache/)

Note: `deka run` executes runtime modules; container commands live under `deka c ...` (or `deka container ...`).

## Local CLI Wiring (dev loop)
- The local `deka` command is wired to `deka/target/release/cli` for this repo.
- Build policy: release-only builds for this repo. Do not build or rely on `target/debug` binaries.
- After Rust changes: `cargo build --release -p cli`
- After PHPX compiler changes: 
  `cargo build -p php-rs --release --target wasm32-unknown-unknown --lib --no-default-features`
  then `cargo build --release -p cli`
- PHPX files execute directly: `deka run app.phpx`

## PHPX Module Root (lockfile)
- PHPX resolves the project root by locating `deka.lock`.
- Set `PHPX_MODULE_ROOT=/path/to/project` to override root discovery.
- For this repo, `deka.lock` and `php_modules/` live at `deka/`.

## PHPX Language
PHPX is a modern typed language that compiles to PHP:

### Core Syntax
```phpx
struct User { 
    $name: string; 
    $age: int = 0; 
}

enum Result {
    case Ok(mixed $value);
    case Err(mixed $error);
}

function divide($a: int, $b: int): Result {
    if ($b === 0) return Result::Err("Division by zero");
    return Result::Ok($a / $b);
}

match divide(10, 2) {
    Result::Ok($v) => $v,
    Result::Err($e) => handle_error($e),
}
```

### Module System
```phpx
import { str_contains } from 'string';
import { UserCard } from 'component/user';

export function create_user($name: string): User {
    return User { $name: $name, $age: 0 };
}
```

### JSX Components
```phpx
function Button($props) {
    return <button class={$props.variant}>
        {$props.children}
    </button>;
}
```

Note: Semicolons are optional in PHPX (JS-style automatic semicolon insertion).

## Service Ports (deka-cli defaults)
- `postgres`: 5432
- `redis`: 6379
- `edge` (runtime): 8506
- `t4`: 8507
- `gild-vcs`: 8508
- `deploy`: 8509

## PHPX Module System
- Core modules: `core/`, `string/`, `array/`, `json/`, `component/`
- User modules: `@user/*` namespace  
- Module resolution: via `deka.lock` in project root
- Standard library: defined in `php_modules/stdlib.json`

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
- PHPX: `deka test` (PHPX test suite), `deka test --no-rust` (skip Rust tests)

### PHPX Conformance Tests (required after runtime changes)
PHPX has a dedicated fixture suite under `deka/tests/phpx/` that guards language/runtime behavior.
Run this after any parser/compiler/VM or module-system change:
```
PHPX_BIN=target/release/cli PHPX_BIN_ARGS=run bun tests/phpx/testrunner.js
```
Each fixture must include a short `TEST:` header comment describing its intent.
See `tests/phpx/CONFORMANCE.md` for the feature-to-fixture map.

## Documentation Workflow (required)
- Keep `runtime/docs/` user-facing only. Put internal plans/task lists under `tasks/` (or `tasks/archive/`).
- After runtime/language/module changes, update docs in the same PR:
  - Language/runtime behavior: `runtime/docs/phpx/**`
  - PHP runtime behavior: `runtime/docs/php/**`
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