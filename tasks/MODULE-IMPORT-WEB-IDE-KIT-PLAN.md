# Plan: DekaScript modules in the browser and test suite

## Goal

Make it possible to write multi-file DekaScript in the tour, the test suite,
and any web-ide-kit consumer:

1. Import items from the `@deka/*` standard library hosted on the deka package
   index.
2. Import raw `.ds` files from the deka package registry; web-ide-kit compiles
   the dependency at the same time as the user code, with caching.
3. Lay the ground work for a large conformance surface around imports and
   exports.

This plan is focused on the browser / web-ide-kit path. The native CLI already
has lockfile-driven `php_modules` resolution; that work is documented in
`MVP-RUNTIME-MODULE-RESOLUTION-PLAN.md`.

## Inspiration

The Gleam playground solves the same problem in a way that fits our existing
architecture:

- The WASM compiler exposes a *project* with a virtual file system:
  `writeModule(name, code)`, `compilePackage(target)`,
  `readCompiledJavaScript(name)`.
- The standard library is pre-compiled and shipped as static JS modules.
- The worker rewrites relative stdlib imports to point at those pre-compiled
  modules and evaluates the entry module with `data:text/javascript;base64,…`.

We can do the same for DekaScript: give the browser compiler a virtual project,
resolve registry imports in TypeScript, compile dependencies through the same
WASM compiler, and evaluate the resulting JS with our existing sandbox.

## Design decisions

1. **Module syntax is JS/TS-like.**
   ```dekascript
   import { reverse, length } from "@deka/string";
   import { add } from "./math.ds";
   import { default as d } from "https://pkg.deka.gg/raw/@deka/string/0.1.0/src/index.ds";

   export fn greet(name: string): string {
     return "Hello, " + name;
   }

   export const version = "1.0.0";
   ```

2. **Specifier kinds.**
   - `@scope/name` and `@scope/name/subpath` — registry packages.
   - `./foo.ds`, `../foo.ds` — relative imports inside a virtual project.
   - Absolute `https://…` URLs — raw source files, compiled on demand.

3. **Per-module ES-module output.** The compiler emits one JS string per
   DekaScript source file with `export const …` / `import { … } from "…"`.
   The browser loader links these modules, not the compiler.

4. **Browser evaluation stays in our sandbox.** We do *not* rely on the native
   browser ES module loader for arbitrary URLs (CORS and integrity become a
   nightmare). Instead we wrap each compiled module in a factory function that
   receives an `imports` map and returns an `exports` object, then evaluate with
   `new Function` inside the existing Worker/direct runner.

5. **Standard library is pre-compiled.** `@deka/*` packages are shipped as
   static `.mjs` files with the website/test suite. The compiler only needs to
   compile user code and non-stdlib registry packages.

6. **Caching is per-module and keyed by source SHA-256.** A module compiled
   from a registry URL or raw URL is cached in memory and optionally in
   IndexedDB so repeated tour runs do not re-compile unchanged dependencies.

## What needs to change

### 1. Compiler WASM ABI

Add a project-mode API alongside the existing single-file
`deka_compiler_compile`. Single-file mode stays untouched for backward
compatibility.

Proposed exports:

```ts
// Create/destroy a project (virtual file system + isolated graph).
deka_compiler_project_new() -> project_id: u32
deka_compiler_project_free(project_id: u32)

// Add a source file to the project.
deka_compiler_project_write_module(
  project_id: u32,
  path_ptr: u32, path_len: u32,
  source_ptr: u32, source_len: u32
)

// Compile every module in the project and resolve imports internally.
// Returns a JSON result with diagnostics and a per-module JS map.
deka_compiler_project_compile(project_id: u32) -> result_ptr: u32

// Read the emitted JS for one module after a successful compile.
deka_compiler_project_read_module_js(
  project_id: u32,
  path_ptr: u32, path_len: u32
) -> result_ptr: u32

// Format a single source file (unchanged semantics, project aware of deps).
deka_compiler_project_format_ds(
  project_id: u32,
  path_ptr: u32, path_len: u32
) -> result_ptr: u32
```

The JSON result for `project_compile` should look like:

```json
{
  "ok": true,
  "modules": {
    "main.ds": { "code": "..." },
    "./math.ds": { "code": "..." }
  },
  "diagnostics": []
}
```

If a module cannot be resolved, the compiler reports a diagnostic with the
import path and the importing file.

### 2. Parser / type checker

- Add `import` and `export` statement parsing.
- Resolve module paths relative to the importing file.
- Validate that imported names exist in the target module.
- Allow re-exports.
- Keep top-level declarations private by default; only `export` makes them
  public.

### 3. web-ide-kit module loader

New file: `src/runtime/modules.ts`.

Responsibilities:

1. **Build a module graph.** Given the entry source and optional extra files
   (e.g. from the tour's file tabs), scan imports recursively.
2. **Resolve specifiers.**
   - `@deka/*` → try the pre-compiled stdlib bundle first, otherwise fetch
     from the registry.
   - other `@scope/name` → fetch from registry tree/blob API.
   - `./foo.ds` → read from the virtual project files supplied by the UI.
   - `https://…` → fetch raw `.ds` source.
3. **Fetch and compile.** Write every non-precompiled source into a compiler
   project, run `deka_compiler_project_compile`, and read back per-module JS.
4. **Cache.** Store `url -> { sourceSha256, compiledJs }` in memory and
   IndexedDB. On a cache hit, skip the fetch/compile.
5. **Link and run.** Use the factory-function loader to execute modules in
   topological order, inject the same deka globals we already provide, and run
   the entry module.

The public API surface in web-ide-kit becomes:

```ts
export async function compileDekaProject(
  entryPath: string,
  files: Record<string, string>,
  options?: { stdlibBaseUrl?: string; registryUrl?: string }
): Promise<CompileProjectResult>;

export async function runDekaProject(
  entryPath: string,
  files: Record<string, string>,
  options?: RunOptions
): Promise<RunResult>;
```

Existing `compileDeka(source, filename)` and `runDekaJsDirect(jsCode)` remain
for single-file code.

### 4. Package registry read access from the browser

The registry API already exposes:

- `GET /api/registry/{name}.json`
- `GET /api/scoped-packages/{scope}/{name}/versions`
- `GET /api/scoped-packages/{scope}/{name}/{version}`
- `GET /api/scoped-packages/{scope}/{name}/{version}/tree`
- `GET /api/scoped-packages/{scope}/{name}/{version}/blob?path={path}`

For the browser we need:

- **Public read endpoints or a CDN mirror.** The tour cannot ask every visitor
  for a bearer token. Either the registry allows anonymous `packages:read`, or
  we mirror stdlib + selected packages to R2/Cloudflare with public CORS.
- **CORS headers** on the registry / CDN so `fetch()` from
  `deka.gg` / `testsuite.deka.gg` works.
- **A stable "raw source" URL format** for raw `.ds` imports, e.g.
  `https://pkg.deka.gg/raw/@deka/string/0.1.0/src/index.ds`.

### 5. Standard library bundling

- Publish `@deka/*` packages as both source trees and pre-compiled `.mjs`
  bundles.
- The website/test suite build downloads the pre-compiled bundles (or commits
  them like the current WASM artifacts) and serves them under
  `/tour/modules/@deka/string/index.mjs`.
- web-ide-kit rewrites `@deka/string` to that URL and skips compilation.

### 6. Test suite expansion

New category: `modules/` in `dekaruntime/testsuite`.

Fixture layout:

```
tests/modules/
  relative-import-001/
    main.pass.ds
    main.stdout
    helper.ds          # imported by main.pass.ds
  stdlib-import-001/
    main.pass.ds
    main.stdout        # expected output when importing @deka/string
  missing-export-001/
    main.fail.ds
    main.json          # expected diagnostic contains "has no export"
  circular-import-001/
    main.fail.ds
    a.ds
    b.ds
```

The test runner loads all `.ds` files in the fixture directory into the
compiler project and runs the entry file.

Tests to add (target 30+):

- Basic `import { x } from "./foo.ds"`.
- Relative `../` imports.
- `export fn`, `export const`, `export type`.
- Default exports.
- Re-exports (`export { x } from "./foo.ds"`).
- Import aliases.
- Importing from `@deka/string` (stdlib).
- Importing a raw URL.
- Missing module → diagnostic.
- Missing named export → diagnostic.
- Circular import → diagnostic.
- Import inside a function (if allowed) vs top-level only.
- Duplicate import names.

### 7. Native CLI parity

The same project-mode compiler API is exposed to the native CLI so local
multi-file DekaScript projects work identically. The CLI already understands
`php_modules` and `deka.lock`; project-mode compilation should use the same
resolver for `@scope/name` imports.

## Implementation phases

### Phase 0: Spike / proof of concept

- Add a minimal project-mode ABI to the WASM compiler: `project_new`,
  `project_write_module`, `project_compile`, `project_read_module_js`.
- Support only relative `./foo.ds` imports.
- In web-ide-kit, add `compileDekaProject` that writes two files, compiles, and
  runs the result through the existing sandbox.
- Add one passing test in testsuite: `modules/relative-import-001`.

Success criteria: a tour page can run

```dekascript
import { add } from "./math.ds";
console.log(add(1, 2));
```

### Phase 1: Import/export syntax

- Parser support for `import` and `export`.
- Type checker validates named imports against exported names.
- Compiler emits per-module JS with factory functions for the browser loader.
- Add 10–15 conformance tests for syntax, aliases, re-exports, and errors.

### Phase 2: Registry + stdlib imports

- Registry resolver in web-ide-kit.
- Pre-compiled `@deka/*` stdlib bundled with the site.
- Raw `.ds` URL imports.
- IndexedDB cache.
- Add 10–15 conformance tests for stdlib and raw URL imports.

### Phase 3: Native CLI project mode

- Expose project-mode API in the native CLI (`deka run main.ds` with multiple
  files in the directory).
- Integrate with `php_modules` resolution for `@scope/name`.
- Add CLI-level tests.

### Phase 4: Hardening

- Circular import detection.
- Better diagnostics for missing exports.
- Cache invalidation and integrity checks.
- Security review of arbitrary URL imports.

## Open questions

1. Do we allow dynamic imports (`const m = await import("./foo.ds")`) in the
   first version, or only static top-level imports?
2. Should relative imports require the `.ds` extension, or do we auto-resolve
   `./math` → `./math.ds` / `./math/index.ds`?
3. How do we version-lock stdlib imports in the tour? Always latest, or pin to
   the runtime version?
4. Do we want a browser-side import map so users can write
   `import { x } from "ui"` and have it resolve to `@pkg.deka.gg/ui`?
5. Should arbitrary URL imports be allow-listed to `*.deka.gg` for security?

## Files that will change

- `runtime/crates/phpx_compiler_wasm/src/lib.rs` — new WASM exports.
- `runtime/crates/php-rs/src/parser/parser/stmt.rs` — import/export parsing.
- `runtime/crates/phpx_js/src/compiler.rs` / `emitter/*.rs` — module
  compilation and JS emission.
- `dekaruntime/web-ide-kit/src/runtime/modules.ts` — new module loader.
- `dekaruntime/web-ide-kit/src/runtime/runtime.ts` — project-mode compile/run.
- `dekaruntime/website` — precompiled stdlib assets + tour examples.
- `dekaruntime/testsuite` — `modules/` conformance fixtures.

## Definition of done

- [ ] A tour example can import from `@deka/string` and run.
- [ ] A tour example can import a raw `.ds` URL and run.
- [ ] testsuite has at least 30 module-related conformance tests.
- [ ] web-ide-kit caches compiled registry modules across runs.
- [ ] Native CLI can compile/run a multi-file DekaScript project.
