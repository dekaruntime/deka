# Deka Init Routing Contract

## Problem

`deka init` currently creates a project that does not serve `/` correctly out of the box.

From first principles:

- Deka is a runtime.
- It serves static assets and `.phpx` handlers.
- A served project must have one clear entry contract.
- That entry must exist, be discoverable from `deka.json`, and produce a valid handler response.

Today the runtime has drifted into two competing PHPX app models:

1. Explicit entry mode
- `deka.json` points to `serve.entry`.
- A `.phpx` entry file is loaded through the PHPX ESM path.
- The runtime expects the entry module to produce `globalThis.app`, then invokes either `app.fetch(req)` or `app(req)`.

2. Runtime-owned app directory mode
- If the resolved target is a directory containing `app/`, the runtime switches to a built-in directory router.
- That router lives inside `servePhp(...)`.
- It resolves `app/` and `api/` internally.

`deka init` is currently mixing the two models:

- It generates `main.phpx` as the explicit entry.
- That entry imports `component/router` and returns `Router()`.
- The runtime still also contains its own app-directory routing path.

This causes ambiguity about who owns routing and what the default app contract actually is.

## Findings

### 1. The runtime contract is request-handler oriented

The request executor calls a loaded handler through one of these shapes:

- `app.fetch(req, ctx)`
- `app(req, ctx)`

That is the real runtime contract.

### 2. `component/router` is framework logic, not runtime glue

`component/router.phpx` scans `app/`, builds a route manifest, resolves a route, loads page/layout modules, and renders them.

That is userland framework behavior.

### 3. Runtime-owned app-directory mode duplicates framework responsibility

`servePhp(...)` also implements directory routing for `app/` and `api/`.

That duplicates `component/router` responsibility and makes the init path ambiguous.

### 4. The current app-directory ESM bridge is also internally inconsistent

The app-directory wrapper computes app-root state, but the generated source currently calls:

```js
const app = servePhp({});
```

That does not pass the directory path `servePhp(...)` expects.

## Decision

Adopt one primary model:

- Deka serves an explicit `.phpx` entry from `deka.json`.
- The runtime remains generic and invokes the handler contract only.
- File-based routing is framework behavior implemented in userland through `component/router`.
- `app/` and `api/` are conventions consumed by `component/router`, not hidden runtime behavior.

## Target Wiring

### Runtime

- Resolve `serve.entry`.
- Load the PHPX entry through the ESM PHPX pipeline.
- Invoke the resulting handler through the standard runtime request contract.
- Do not rely on hidden app-directory routing for the default project path.

### Init Template

- Generate a real `main.phpx` entry.
- That entry must explicitly satisfy the runtime handler contract.
- The default entry should call `component/router` and pass request-derived data explicitly.
- `app/page.phpx` and `app/layout.phpx` remain user-visible framework files.

### Router

- `component/router` remains the source of truth for file-based routing.
- It owns `app/` and `api/` conventions.
- It should be exercised by the init template and covered by tests.

## Tasks

- [ ] Document this routing contract and commit it.
- [ ] Fix the default init entry so it matches the runtime request-handler contract exactly.
- [ ] Remove default dependence on runtime-owned app-directory routing.
- [ ] Add regression coverage for `deka init -> deka serve -> GET /`.
- [ ] Verify with a release CLI build.
- [ ] Summarize the final runtime/init wiring in docs or task notes.

## Implementation Notes

- Keep the explicit entry model.
- Avoid runtime magic for framework routing.
- Prefer visible userland code over hidden runtime heuristics.
- If runtime-owned app-directory mode remains temporarily for compatibility, it must not be the path used by `deka init`.
