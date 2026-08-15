# Agent Notes (Current Mission)

## Canonical Repo Policy (non-negotiable)

- **ACTIVE IMPLEMENTATION REPO:** `~/Projects/deka/mvp2`
- **ARCHIVE SOURCE REPO (read-only):** `~/Projects/deka/deka-ARCHIVE`
- **Browser host substrate repo:** `~/Projects/deka/adwa`

You must only implement, test, and commit runtime/CLI/LSP MVP work in `~/Projects/deka/mvp2`.

`~/Projects/deka/deka-ARCHIVE` is migration source only.
Do not run active task work there.

## Mission scope (active)

Primary plan: `tasks/REBOOT-PLATFORM-PLAN.md`.

MVP platforms only:

- `platform_server`
- `platform_browser` (ADWA)

Deferred platforms (post-MVP):

- `platform_multi_tenant`
- `platform_cli`
- `platform_desktop`

Do not add Node/Bun compatibility work in this mission.

## Commit policy (mandatory)

- Every change, including small fixes, must be committed before starting the next task.
- Use commit messages as append-only notes.
- Do not amend/rewrite commit history unless explicitly requested.
- Include verification summary in each commit message.

## Checkpoint process (mandatory)

`checkpoint` is a required quality gate before continuing major work.

- Trigger checkpoint when:
  - finishing a phase milestone,
  - switching repos/runtime tracks,
  - touching runtime execution, CLI behavior, LSP behavior, or ADWA host behavior,
  - before handing off to another agent/session.
- Checkpoint steps:
  - Run release builds/tests for touched areas.
  - Verify local artifact wiring and lineage (`deka`, `deka lsp`, manifest).
  - Execute basic human validation flow:
    - Run the owning parser/emitter tests for the implemented `.ds` slice.
    - Do not substitute a CLI or serve smoke until those contracts are owned and available.
    - `scripts/test-islands-smoke.sh` for islands SSR/hydration metadata + directive alias checks.
    - ADWA build/e2e checks for browser platform updates.
  - Record a short checkpoint summary in commit message or task notes:
    - what passed,
    - what failed,
    - what was deferred and why.
- Do not proceed to the next major task until checkpoint results are captured.

## Build and verification policy

- Build release artifacts only.
- Do not rely on debug binaries for validation.
- Run relevant tests/checks before commit.
- Use `scripts/build-release-manifest.sh` to produce release artifacts + `target/release/deka-manifest.json`.
- Use `scripts/verify-release-manifest.sh` to fail fast on stale/mismatched `cli` and `php_rs.wasm` artifacts.
- Keep local PATH wiring pinned to this repo's release binaries:
  - `~/.local/bin/deka -> ~/Projects/deka/mvp2/target/release/cli`
  - No public DekaScript editor contract is available in the compiler-core slice.

ADWA runtime/UI changes (current script names still use `adwa`):

1. `scripts/run-adwa-playground.sh --build-only`
2. Run only the browser checks owned by the ADWA lane.

## Artifact/version discipline

- Track artifact freshness explicitly.
- Prefer a build manifest with artifact hashes and git SHA.
- Surface runtime lineage in CLI/version output when available.
- Avoid stale binary/wasm drift across runtime, browser assets, and LSP.

## Architecture direction

- Keep crate-per-responsibility organization.
- Keep command registry pattern in CLI.
- Move toward a host abstraction (`platform`) so runtime core stays host-agnostic.
- Keep LSP integrated under `deka lsp` direction; avoid separate lifecycle drift.

## Docs and tasks policy

- Keep active plans and checklists in `tasks/`.
- Keep user-facing compiler-core docs in `docs/dekascript/`.
- Keep internal plans/devlogs/design notes outside public docs.
- If runtime behavior changes, update relevant docs in the same task.
- `php_modules` exported APIs must include `/// docid:` blocks; docs publish/build must fail when coverage is missing.
- Use `scripts/build-release-docs.sh` as the default release pipeline (build `cli` + publish/bundle docs).
- Do not claim a DekaScript docs CI contract until its owner establishes one.


## DekaScript availability (explicit)

- `.ds` is the only intended public source extension.
- The current stacked compiler-core work proves parser/emitter support only.
- CLI execution, serving, routing, static assets, imports, editor integration,
  and package resolution have no public DekaScript contract in this slice.

## deka.json project contract (build/runtime)

- `deka.json` is required at project root.
- Standard key: `type`
  - `type: "lib"` => library/module package (no runnable app entry).
  - `type: "serve"` => runnable app package.
- For runnable apps (`type: "serve"`), `serve.entry` is required and must point to the runtime entry file.
## Runtime/bootstrap status

`deka.json` and module-resolution requirements belong to the CLI/runtime lane.
Do not publish a DekaScript bootstrap contract until that lane has implemented
and validated it.

## Introspect metrics (quick reality check)

- The `introspect` crate is primarily a CLI/UI client; core op timing collection lives in runtime pool internals.
- Deno op metrics are tracked in `crates/pool/src/isolate_pool.rs` via `OpMetricsEvent` (`Dispatched`, `Completed`, `CompletedAsync`, `ErrorAsync`, etc.).
- Per-op summaries include `in_flight` counts (`OpTimingSummary.in_flight`) and request traces include per-request `op_timings`.
- `crates/modules_php` bridge stats (`op_php_bridge_proto_stats`) provide transport-level metrics (calls/bytes/time), not full isolate op scheduling metrics.
- If async behavior is in question, validate with runtime op timing output (or introspect debug views) rather than only module-level wrappers.
