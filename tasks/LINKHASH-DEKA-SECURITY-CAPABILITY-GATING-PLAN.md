# Deka Security Capability Gating Plan (MVP)

Goal: add a Deno-inspired permission model for Deka with deterministic policy enforcement, strong defaults, and explicit guard rails for multi-agent workflows.

## Policy Model

Config key: `security`

Default security posture:
- deny sensitive operations by default
- only allow through explicit CLI flags or `security` policy
- `deny` rules always override `allow`

Primary capability groups:
- `read` (filesystem reads)
- `write` (filesystem writes)
- `net` (outbound/inbound network)
- `env` (environment variable access)
- `run` (subprocess execution)
- `db` (database access by engine)
- `dynamic` (dynamic code execution)
- `wasm` (wasm module load/instantiate/host-import boundary)

## CLI Flag Surface

Allow flags:
- `--allow-read[=<PATH>...]`
- `--allow-write[=<PATH>...]`
- `--allow-net[=<HOST>...]`
- `--allow-env[=<VAR>...]`
- `--allow-run[=<PROGRAM_NAME>...]`
- `--allow-db[=<ENGINE>...]` where engine is `postgres,mysql,sqlite`
- `--allow-dynamic`
- `--allow-wasm[=<PATH|URL>...]`
- `--allow-all`

Deny flags:
- `--deny-read[=<PATH>...]`
- `--deny-write[=<PATH>...]`
- `--deny-net[=<HOST>...]`
- `--deny-env[=<VAR>...]`
- `--deny-run[=<PROGRAM_NAME>...]`
- `--deny-db[=<ENGINE>...]`
- `--deny-dynamic`
- `--deny-wasm[=<PATH|URL>...]`

Prompt control:
- `--no-prompt`

## Subprocess Security Contract

`run` is high risk and must be treated as privilege escalation:
- code cannot spawn subprocesses by default
- subprocess execution requires explicit `--allow-run`
- if `--allow-run` is granted broadly, child process permissions are effectively outside Deka runtime gating
- recommended safe usage is allowlisting explicit executable names, e.g. `--allow-run=git,curl`

MVP policy behavior:
- `run` denied by default
- `--allow-run` with no program list is allowed but emits a high-severity warning
- `--allow-run=<list>` is preferred and should be highlighted as best practice
- `--deny-run` always takes precedence over `--allow-run`

## `security` Schema (MVP)

```json
{
  "security": {
    "allow": {
      "read": ["./src", "./migrations"],
      "write": ["./.cache"],
      "net": ["api.linkhash.dev:443"],
      "env": ["DATABASE_URL"],
      "run": ["git"],
      "db": ["postgres"],
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

## Resolution & Precedence

Effective policy computation:
1. defaults (deny)
2. project config `security`
3. CLI allow flags
4. CLI deny flags
5. `--allow-all` (except explicit deny, if we keep deny-precedence globally)

Prompt behavior:
- when a request is undecided and TTY is available: prompt if `prompt=true` and `--no-prompt` is absent
- when non-TTY or `--no-prompt`: deny with structured error

## Runtime Enforcement Model

Enforce at operation boundaries, not only call sites:
- fs operations
- network connect/listen
- env read/write
- db connection/query API entry points
- subprocess spawn
- dynamic execution entry points (`eval`, dynamic include/import/fetch+exec)

Structured errors (stable codes):
- `SECURITY_CAPABILITY_DENIED`
- `SECURITY_POLICY_DENY_PRECEDENCE`
- `SECURITY_RUN_PRIVILEGE_ESCALATION_RISK`
- `SECURITY_DYNAMIC_EXEC_DENIED`

Error payload requirements:
- capability group + action
- target resource (path/host/var/program)
- policy source that denied the request
- import/module/file:line if available
- remediation hint

## Publish-Time Guardrails (Linkhash)

At package preflight/publish:
- static capability extraction from package
- compare detected capabilities vs declared package policy
- reject undeclared sensitive capabilities
- require explicit declaration for `run` and `dynamic`
- attach capability metadata to release manifest

## Execution Plan

### Task 1: Policy & Schema
- [x] Define canonical runtime operation-to-capability matrix as single source of truth.
- [x] Add `unknown` bucket for unmapped operations and test for duplicate op ids.
- [ ] Commit: `feat(security): add canonical operation capability matrix`

### Task 2: Policy & Schema
- [x] Define `security` schema + parser in CLI/runtime shared code.
- [x] Add validation + diagnostics for invalid policy entries.
- [ ] Commit: `feat(security): add security policy schema and parser`

### Task 3: CLI Flags
- [x] Add allow/deny security flags to `deka run`, `deka dev`, `deka serve`, and relevant test flows.
- [x] Implement precedence merge (config + flags + defaults).
- [x] Add `--no-prompt` plumbing.
- [ ] Commit: `feat(cli): add capability allow/deny flags and policy merge`

### Task 4: Runtime Capability Gates
- [x] Add centralized capability gate checks in runtime op boundaries (fs/net/env/db/run/dynamic).
- [x] Return structured security errors with code + target + source location.
- [ ] Commit: `feat(runtime): enforce capability gates at operation boundaries`

### Task 5: Subprocess Hardening
- [x] Deny subprocess spawn by default.
- [x] Implement executable allowlist matching for `--allow-run=<list>`.
- [x] Add warning/error path for broad `--allow-run`.
- [ ] Commit: `feat(runtime): enforce subprocess allowlist and escalation warnings`

### Task 6: Dynamic Execution Gate
- [x] Block dynamic execution by default.
- [x] Add explicit `dynamic` capability enable path.
- [x] Add tests for denied and allowed paths.
- [ ] Commit: `feat(runtime): gate dynamic execution behind explicit capability`

### Task 7: Prompt + Non-TTY Behavior
- [x] Implement interactive prompt flow for undecided requests.
- [x] Disable prompt on non-TTY or `--no-prompt`.
- [x] Add deterministic deny behavior with remediation text.
- [ ] Commit: `feat(security): add prompt workflow and non-tty deny mode`

### Task 8: Package Preflight Integration
- [x] Add static capability extraction at publish preflight.
- [x] Enforce declaration for `run` and `dynamic`.
- [x] Store capability metadata in release records.
- [ ] Commit: `feat(linkhash): enforce capability declarations at publish preflight`

### Task 9: Install-Time Policy Check
- [x] Validate package declared capabilities against project policy during `deka add/install`.
- [x] Block denied capabilities unless user overrides explicitly.
- [ ] Commit: `feat(pm): enforce project security policy during install`

### Task 10: Tests
- [x] Add fixtures for each capability group and deny-precedence cases.
- [x] Add subprocess and dynamic-exec regression tests.
- [x] Add CLI integration tests for config+flag merge behavior.
- [ ] Commit: `test(security): add capability gating coverage`

### Task 11: Docs
- [x] Document `security` schema and examples.
- [x] Document subprocess escalation model and recommended `--allow-run=<list>` usage.
- [x] Document dynamic execution default deny.
- [ ] Commit: `docs(security): document capability gating model and run/dynamic risks`

## Definition of Done
- [ ] Security model enabled by default with deny-first behavior.
- [ ] `run` and `dynamic` both require explicit opt-in.
- [ ] Deny-precedence is consistent across config + CLI flags.
- [ ] Structured security errors are stable and machine-actionable.
- [ ] Publish/install/runtime checks align on the same capability taxonomy.
