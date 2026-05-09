# Security Notes — `op_php_cwd` and `dekaFsNormPath`

Audit follow-up to Hamza Finding 6 (MEDIUM) from PR #17 review (issue #155).

## Concern

`dekaFsNormPath` in `php.js` joins relative paths against `op_php_cwd()` before
the prefix-confinement check runs. If a tenant could mutate the process cwd to
an absolute path outside the tenant root, the relative-path join would shift
the resolution base and the literal `..` segment scan (which runs on the
tenant's input string, not the joined absolute path) would not catch it. Only
the tenant-root prefix check (layer 4) defends — and that defence depends on
`op_php_cwd()` returning a value still inside the tenant root.

## Conclusion: SAFE — outcome (a)

PHPX user code cannot mutate the process cwd. `op_php_cwd()` is therefore
stable for the lifetime of a request and effectively constant per-process,
making `dekaFsNormPath`'s base safe to use.

## Evidence

### 1. `op_php_cwd` reads the kernel-tracked cwd

`runtime/crates/modules_php/src/modules/php/mod.rs:4540-4546`

```rust
#[op2]
#[string]
fn op_php_cwd() -> Result<String, deno_core::error::CoreError> {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| deno_core::error::CoreError::from(e))
}
```

`std::env::current_dir()` issues `getcwd(2)` and returns the kernel-tracked
working directory of the process. It does **not** read `$PWD` or any other
environment variable, so `putenv("PWD", ...)` (even if such an op existed)
would not steer the result.

### 2. No `chdir` op or JS-callable cwd mutator is exposed

The PHPX op surface is enumerated at
`runtime/crates/modules_php/src/modules/php/mod.rs:4775-4813`. The only
cwd-related op is `op_php_cwd` (read). There is no `op_php_chdir`,
`op_php_set_cwd`, or equivalent. A repo-wide grep for
`set_current_dir|chdir|op_set_cwd|setCwd|set_cwd` finds no op definition that
would mutate process cwd.

Capability registration confirms this — `runtime/crates/runtime_core/src/security.rs:110-114`:

```rust
OperationCapability {
    op_id: "php.op_php_cwd",
    capability: Capability::Read,
    notes: "Read current working directory",
},
```

There is no matching `Capability::Write` entry for cwd anywhere in
`security.rs`.

### 3. `process.chdir` is not bridged

`runtime/crates/modules_php/src/modules/php/php.js:262-264` exposes only the
read side:

```js
if (!globalThis.process.cwd) {
  globalThis.process.cwd = () => op_php_cwd();
}
```

`globalThis.process.chdir` is not assigned anywhere in `php.js`,
`deka_php/php.js`, or any extension JS in `runtime/crates/`.

### 4. Env-var mutation cannot steer cwd

`std::env::set_var` is callable in Rust but never exposed as an op. Even if a
tenant could set `PWD`, that has no effect on `std::env::current_dir()`
(see (1)). `op_php_read_env` is read-only and policy-gated
(`mod.rs:1426-1433`).

### 5. Host-side `set_current_dir` calls are out-of-band

A repo-wide search for `std::env::set_current_dir` finds exactly two
call sites, both unreachable from a running tenant request:

- `runtime/crates/cli/src/cli/init.rs:143,155` — `deka init` scaffolding
  command. Single-shot CLI invocation; not part of the multi-tenant platform
  server. The cwd is also restored via `set_current_dir(previous)` in the
  same function.
- `runtime/crates/php-rs/src/bin/common/mod.rs:37` — `run_script` entry
  point used by `deka run`, executes once at process startup before any user
  code runs.

The platform server (`runtime/crates/runtime/src/platform.rs`,
`crates/platform_server/`, `crates/pool/`, `crates/engine/`,
`crates/runtime_core/`) does not call `set_current_dir` at all — confirmed by
grep across those crates.

## Defence-in-depth still in place

Even granting the (false) hypothetical that cwd could be steered, the
tenant-root prefix check at `php.js:189-192` would still fail because
`__dekaFsTenantRoot` is baked into the tenant bundle at build time
(`runtime/crates/runtime/src/js_pipeline.rs:40-49`) using the canonicalised
project root, and the prefix assertion runs on either the canonicalised
resolved path (when the file exists) or the lexically normalised path. A
relative path joined against an out-of-root cwd would still not start with
`__dekaFsTenantRoot`.

## Recommendation

No code change required. The audit conclusion is documented here so future
reviewers do not re-litigate the same concern. If a `chdir`-equivalent op is
ever added (e.g. for test harnesses), this note must be revisited and the
base for `dekaFsNormPath` should switch to `globalThis.__dekaFsTenantRoot` to
remove the implicit dependency on cwd stability.
