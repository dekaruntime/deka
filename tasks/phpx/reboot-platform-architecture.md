# Reboot Platform Architecture

Status: Active (MVP reboot)
Plan source: `tasks/REBOOT-PLATFORM-PLAN.md`

## Goals

- Keep runtime core host-agnostic.
- Support two MVP hosts only: server and browser (ADWA).
- Keep one CLI entrypoint (`deka`) for runtime and LSP.

## Host Contract (`platform`)

Runtime code should depend on a host trait surface, not on Deno/browser APIs directly.

Required capability groups:

- `fs`: read/write/stat/mkdir/readdir/watch primitives.
- `env`: getenv/setenv/list env values.
- `io`: stdin/stdout/stderr pipes and streams.
- `process`: spawn/kill/wait/exit, argv, cwd.
- `time`: monotonic clock, wall clock, timers.
- `random`: secure random bytes and ids.
- `net`: tcp/http/websocket primitives needed by runtime modules.
- `ports`: host port binding and reservation behavior.

Rules:

- Contracts expose semantic behavior, not host-specific types.
- Errors are normalized into runtime error families.
- Capabilities can be denied by host policy (especially browser).

## Crate Boundaries

Target boundary map for reboot:

- `core`: command/arg/context model and shared runtime-facing types.
- `runtime_core` (planned): host-agnostic execution pipeline for PHP/PHPX.
- `platform` (planned): trait/contracts used by `runtime_core`.
- `platform_server` (planned): server host adapter (Deno-backed internals isolated here).
- `platform_browser` (planned): ADWA host adapter with browser restrictions.
- `runtime`: assembly layer wiring `runtime_core` + selected platform.
- `cli`: command registry + orchestration; contains `deka lsp` entry.
- `phpx_lsp_core`: syntax/diagnostic analysis core reused by LSP entrypoints.
- `phpx_lsp`: integrated server logic consumed by `cli` (no standalone shipping path).

## Dependency Direction

Allowed direction:

- `cli` -> `runtime` -> `runtime_core` -> `platform`
- `platform_server` -> `platform`
- `platform_browser` -> `platform`

Disallowed direction:

- `runtime_core` -> `platform_server` or `platform_browser`
- `core` -> host adapters
- browser/server adapters depending on each other

## Execution Modes

- `deka run`: single execution flow using selected platform adapter.
- `deka serve`: long-lived server flow using selected platform adapter.
- `deka lsp`: in-process language server in `deka` binary.

## Browser (ADWA) Constraints

- No direct host filesystem/network escape beyond allowed adapter APIs.
- No server-only command stubs in browser host.
- Browser execution uses real runtime semantics through adapter contracts.

## Versioning and Artifacts

- Build manifest carries lineage (`git_sha`, timestamp, target, ABI, artifact hashes).
- CLI performs stale artifact mismatch checks at startup.
- LSP now shares lifecycle/version with the same `deka` binary.

## Out of Scope (MVP)

- Node/Bun compatibility layer.
- Separate LSP binary distribution.
- Desktop/CLI embedded runtime platforms.
- JS/TS bundling pipeline reboot work.
