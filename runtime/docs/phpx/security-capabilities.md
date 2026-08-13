---
title: "PHPX Security Capabilities"
section: "phpx"
category: "security"
categoryLabel: "Security"
categoryOrder: 60
---

# PHPX Security Capabilities (MVP)

`deka` supports a Deno-inspired capability model for runtime guard rails.

Security enforcement is always on by default in MVP. If no explicit allow policy or allow flags are provided, sensitive operations are denied.

## Config Key

Project policy lives in `deka.json` under `security`:

```json
{
  "security": {
    "allow": {
      "read": ["./src"],
      "write": ["./.cache"],
      "net": ["api.linkhash.dev:443"],
      "env": ["DATABASE_URL"],
      "run": ["git"],
      "db": ["postgres", "stats"],
      "wasm": true,
      "dynamic": false
    },
    "deny": {
      "net": ["169.254.169.254"],
      "run": ["bash", "sh"],
      "dynamic": true
    },
    "prompt": true
  }
}
```

## Capability Groups

- `read`: filesystem reads
- `write`: filesystem writes
- `net`: network access
- `env`: environment variable access
- `run`: subprocess execution
- `db`: database operations
- `wasm`: wasm load/instantiate/host interop
- `dynamic`: dynamic execution paths (for example module eval fallback)

## CLI Flags

Allow flags:
- `--allow-read`
- `--allow-write`
- `--allow-net`
- `--allow-env`
- `--allow-run`
- `--allow-db`
- `--allow-wasm`
- `--allow-dynamic`
- `--allow-all`

Deny flags:
- `--deny-read`
- `--deny-write`
- `--deny-net`
- `--deny-env`
- `--deny-run`
- `--deny-db`
- `--deny-wasm`
- `--deny-dynamic`

Prompt control:
- `--no-prompt`

## Precedence

1. defaults (deny)
2. `security` config
3. CLI allow flags
4. CLI deny flags

`deny` always wins when both allow and deny apply.

## Prompt Behavior

- If prompts are enabled and the process has a TTY, runtime may prompt for undeclared operations.
- `--no-prompt` disables prompts and forces deterministic deny.
- Non-TTY execution is treated as non-interactive deny.

## Path Rules (read/write/wasm)

- `read`, `write`, and `wasm` entries are treated as path prefixes.
- If you allow `./src`, any read under `./src/**` is allowed.
- Relative paths are resolved from the current working directory.

## Dev Defaults (`--dev`)

When running in dev mode (`deka serve --dev`), Deka applies safe defaults
to reduce prompt noise:

- `read`: project root (prefix)
- `write`: `./.cache`, `./php_modules/.cache`
- `wasm`: all
- `env`: all

Explicit `deny` rules still take precedence.

## Internal Runtime Privileged Writes

The PHPX runtime performs internal maintenance (module compilation cache + lock updates)
while executing user code. These internal writes use a privileged context that bypasses
user `security` prompts for specific internal paths only:

- `deka.lock`
- `php_modules/.cache/**`
- `.cache/**`

User code is still subject to normal `read`/`write` rules for all other paths.

## DB Targets

The `db` capability accepts driver targets and a stats target:

- `postgres`, `mysql`, `sqlite`: allow specific database engines
- `stats`: allow `db stats` without granting all drivers

## Subprocesses (`run`) and Privilege Escalation

Subprocess execution can bypass most in-process sandbox assumptions.

- `run` is blocked unless explicitly allowed by policy or CLI flag.
- Broad `--allow-run` emits a warning because child processes run with host privileges.
- Prefer allowlisting executables in `security.allow.run` for safer operation.

## Dynamic Execution

Dynamic execution is treated as high risk and should remain disabled unless required.

- Dynamic module eval fallback is gated by `dynamic`.
- If disabled, runtime raises `SECURITY_DYNAMIC_EXEC_DENIED`.

## Registry + Install Guard Rails

- Linkhash publish preflight detects `run` and `dynamic` usage and requires explicit declaration.
- Install checks local `security` policy and blocks packages that require denied capabilities.

## Package Integrity (Lockfile Hashing)

For third-party PHP packages, Deka verifies that the on-disk package contents match the lockfile.

- `deka install` records two hashes per PHP package in `deka.lock`:
  - `moduleGraph`: hash of the package's PHPX import graph.
  - `fsGraph`: hash of the package directory contents.
- On `deka run` / `deka serve`, Deka recomputes both and refuses to run if they differ.
- This check applies to scoped packages (for example `@scope/name`, including `@user/*`). Local project modules (`@/`) and stdlib modules are not enforced.

If a package hash is missing or mismatched, Deka exits with an integrity error. Reinstall the package or run `deka install --rehash` to regenerate the lock entry.
