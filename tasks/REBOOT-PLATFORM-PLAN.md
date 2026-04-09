# Reboot Platform Plan (MVP)

Status: Active
Owner: Runtime team

## Mission

Reboot Deka around PHP/PHPX only, with a clean platform abstraction and a single CLI binary.

MVP platforms:

- [x] `platform_server`
- [x] `platform_browser` (ADWA)

Deferred (post-MVP):

- [ ] `platform_multi_tenant` (Tana-specific hardening/orchestration)
- [ ] `platform_cli` (embedded VFS executable publishing)
- [ ] `platform_desktop` (desktop packaging)

## Non-goals (MVP)

- [x] No Node/Bun compatibility layer (runtime run/serve paths stripped).
- [x] No separate LSP runtime path from CLI (`deka lsp` now runs in-process).
- [x] No CLI/Desktop platform runtime work.
- [x] No JS/TS bundling pipeline in MVP (`crates/bundler` dropped from workspace); `deka build` is PHPX->JS emit only.

## Locked decisions

- [x] `introspect` is retained in reboot MVP.
- [x] `loop` crate is excluded from reboot MVP.
- [x] `bundler` crate is dropped from reboot MVP.

## Hard workflow rules

- [x] Every change must be committed before moving to the next task.
- [x] Commit history is append-only notes (no amend/rewrite unless explicitly requested).
- [x] Run relevant verification before each commit; include verification in commit message.
- [x] Release profile only; avoid debug build drift.

## Phase 1: Foundation

- [x] Create new repo/workspace scaffold for rebooted runtime.
- [x] Keep crate-per-responsibility model and command registry pattern.
- [x] Add `platform` abstraction crate with host contracts.
- [x] `fs`, `env`, `io`, `process`, `time`, `random`, `net`, `ports`.
- [x] Add architecture docs for platform contracts and crate boundaries.

## Phase 2: Runtime Core Extraction

- [x] Extract host-agnostic runtime logic into `runtime_core` (initial shared utilities).
- [x] Remove direct host/runtime globals from core execution path.
- [x] Move shared handler path/entry validation helpers into `runtime_core` and consume from `runtime` run path.
- [x] Move shared PHPX module-graph validation report flow into `runtime_core` and consume from both run/serve paths.
- [x] Move PHP run/serve handler code generation into `runtime_core` (`php_pipeline`) and consume from runtime paths.
- [x] Route run/serve environment and filesystem prep through `platform_server` contracts (`Env.get/set`, `Fs.exists/cwd/canonicalize/current_exe`) instead of direct std host calls.
- [ ] Ensure PHP/PHPX pipeline can run through injected platform contracts.
- [x] Keep module system behavior parity with current runtime where intended (`scripts/parity-runtime.sh` compare pass).

## Phase 3: Server Platform

- [x] Implement `platform_server` (initial adapter scaffold).
- [x] Update Deno dependencies to latest stable.
- [x] Port `pool` isolate/V8 interactions to latest Deno pinned-scope APIs (`JsRuntime::handle_scope` removal, `v8::PinScope` callsites).
- [x] Minimize direct Deno touchpoints to server platform adapter.
- [x] Restore parity for `deka run` and `deka serve` on server.

## Phase 4: Browser Platform (ADWA)

- [x] Implement `platform_browser` on ADWA primitives (initial adapter scaffold).
- [x] Remove browser-side "serve magic" and command stubs.
- [x] Make `cli` crate buildable for `wasm32-unknown-unknown` (`--no-default-features`) as browser command-runtime baseline.
- [x] Remove browser-CLI startup panics from unsupported env reads (wasm-safe startup for help/version paths).
- [x] Add host command-runtime injection in ADWA JS and a `createDekaWasmCommandRuntime(...)` adapter.
- [x] Wire demo vendor build to ship `deka_cli.wasm` and boot with wasm-based `deka` command runtime.
- [x] Add smoke test for wasm deka command runtime (`npm run smoke:deka-wasm-runtime`).
- [x] Add browser command runtime wrapper that handles `deka run <file>` via PHP runtime adapter and delegates other commands to wasm CLI runtime.
- [x] Add POSIX-style command resolver in ADWA JS (`PATH` + shebang interpreter chaining) so executable scripts route to registered runtimes without command-name hacks.
- [x] Run real `deka` process semantics in browser host (foreground `deka serve` lifecycle: boot logs, port publish/unpublish, wait-until-kill).
- [x] Keep strict module/network restrictions in browser adapter.

## Phase 5: CLI + LSP Unification

- [x] Move LSP implementation behind `deka lsp`.
- [x] Remove separate LSP binary packaging path.
- [x] Ensure same version lineage for runtime and LSP behavior.

## Phase 6: Parity and Verification

- [x] Build behavior parity suite baseline: old runtime vs rebooted runtime (`scripts/parity-runtime.sh` run+serve).
- [x] Cover frontmatter, JSX/components, module loading, run/serve, diagnostics.
- [x] Add required CI checks for parity gates.

## Phase 7: Artifact and Version Discipline

- [x] Emit build manifest for each build artifact set:
- [x] `git_sha`, build timestamp, target triple, runtime ABI version, artifact hashes.
- [x] Add `deka --version --verbose` with manifest details.
- [x] Add mismatch detection and fail fast on stale artifact combinations.

## Acceptance Criteria (MVP)

- [x] Single `deka` binary ships runtime + LSP command.
- [x] `platform_server` and `platform_browser` are functional and tested.
- [x] No Node/Bun compatibility code remains in reboot repo.
- [x] Runtime core contains no platform-specific glue.
- [x] Artifact versioning and stale detection are enforced.

## Migration Audit (Archive -> MVP)

Backported into `mvp2`:

- [x] Runtime/CLI: honor explicit CLI args before embedded VFS auto-serve path.
- [x] Runtime Serve: honor `--port` override in listener binding.
- [x] PHPX parser: enforce explicit parameter typing in user mode.
- [x] CLI DB generator: emit parser-safe typed params under strict PHPX rules.
- [x] Canonical repo policy: `mvp2` active, `mvp-ARCHIVE` read-only.
- [x] Missing ADWA docs/tasks synced into `mvp2`.

Still pending backport review:

- [x] LSP import/export diagnostics + completion parity set from archive commits, without pulling unfinished `phpx_lsp_core` split into `mvp2`.
- [x] Any remaining ADWA command-runtime parity commits that are not already present in `mvp2` runtime path.

Verification notes (2026-02-14):
- `cargo test -p phpx_lsp --release -- --nocapture` (import/export diagnostics + completion tests green)
- `scripts/run-adwa-playground.sh --build-only` and `scripts/test-adwa-playground-e2e.sh` (green)
- ADWA script pathing supports standalone repo default (`../adwa`) with local fallback.
