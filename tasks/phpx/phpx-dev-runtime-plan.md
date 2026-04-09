# PHPX Dev Runtime Plan

## Goals

- Keep `deka serve` production-only.
- Add `deka serve --dev` as the canonical development runtime mode.
- Support HMR for all rendered HTML, not only islands.
- Keep islands as hydration scheduling primitives, not HMR boundaries.
- Provide Tailwind-compatible utility CSS generation in runtime (no required Node build step).

## Command Model

- `deka serve <entry>`: production behavior only.
- `deka serve --dev <entry>`: watch + HMR WS + dev client injection.
- `deka run dev`: script alias flow from `deka.json` (expected usage: `deka serve --dev <entry>`).

## Architecture

### 1) Dev Server Core

- [x] Add `--dev` flag to `serve`.
- [x] Enable watch mode automatically when `--dev` is set.
- [x] Start HMR websocket endpoint in dev mode.
- [x] Inject dev client bootstrap into HTML responses in dev mode only.
- [x] Keep `serve` output stable for production mode.

### 2) HTML-Wide HMR (Not Islands-Only)

- [x] Assign stable deterministic node ids (`deka*`) for rendered output.
- [x] Keep ids stable across renders when structure is unchanged.
- [x] Compute server-side DOM diffs on file changes.
- [x] Send granular patch operations over HMR WS.
- [x] Apply node/subtree patches on client without full route replacement.
- [x] Preserve focus, scroll, and form state where possible.
- [x] Escalate to subtree replace, then hard reload only as last resort.

### 3) Islands and Hydration Scheduling

- [x] Support directives: `client:load`, `client:idle`, `client:visible`, `client:media`, `client:only`.
- [x] Keep island boundaries explicit and metadata-driven.
- [x] Let non-island HTML still receive HMR patches.
- [x] Maintain client scheduling behavior independent from HMR patch transport.

### 4) Utility CSS Runtime (Tailwind-Compatible Surface)

- [x] Introduce runtime-native utility CSS engine crate.
- [x] Start with core utility class compatibility and variant handling.
- [x] Add optional preflight/base reset toggle.
- [x] Generate deduped CSS during SSR and cache in project `.cache`.
- [x] Document compatibility matrix and unsupported classes.
: See `docs/phpx/utility-css-compat.md`.

### 5) Tooling and LSP

- [x] Ensure import/export diagnostics use plain text in editor diagnostics.
- [x] Ensure import completion includes module exports reliably.
- [x] Add module path + export completion parity tests for editor integration.
- [x] Document VS Code + Zed dev-mode workflow and troubleshooting.

## Phased Rollout

### Phase A: Dev Mode Scaffolding

- [x] `serve --dev` flag and runtime dev-mode plumbing.
- [x] File-watch event stream emits HMR change notifications (initial logging path).
- [x] Dev-mode feature gate for future WS and patch application.
- [x] `deka run <script>` resolves `deka.json` scripts for dev entrypoints (for `deka run dev` flow).

### Phase B: HMR Transport

- [x] WS endpoint (`/_deka/hmr`) and event protocol.
- [x] Browser dev client bootstrap and reconnect logic.
- [x] Basic reload fallback when patching is unavailable.

### Phase C: Granular DOM Patches

- [x] Stable DOM id assignment.
- [x] Server diff generation and patch payload schema.
- [x] Client patch applier with state-preserving heuristics.

### Phase D: Islands Scheduling

- [x] Directive parser + metadata emission.
- [x] Client scheduler (`load/idle/visible/media`).
- [x] Boundary-local remount fallback.

### Phase E: Utility CSS Engine

- [x] Utility parser + resolver + emitter.
- [x] Config file support (`deka.css.json` or equivalent).
- [x] SSR injection + cache integration.

### Phase F: Hardening

- [ ] End-to-end tests for HMR state preservation. (Deferred by decision: keep unit/integration-only for now)
- [x] Unit coverage for server snapshot patch generation (`websocket.rs`).
- [x] Unit payload-size baseline (granular patch vs full replace payload).
- [x] Documentation and migration notes (`docs/phpx/utility-css-compat.md`).
: Browser-level state preservation e2e remains the only open hardening test.

## Acceptance Criteria

- [x] `deka serve` never enables watcher/HMR/dev client behavior.
- [x] `deka serve --dev` enables watch + HMR transport.
- [x] Editing a template updates changed DOM nodes without full route replacement.
- [x] Islands hydrate by directive schedule while non-island DOM still supports HMR.
- [x] Tailwind-style utility classes can be used without separate build tooling.

## Utility CSS Config

Create `deka.css.json` at project root:

```json
{
  "utility": {
    "enabled": true,
    "preflight": false
  }
}
```

- `enabled`: enables runtime utility CSS emission for HTML responses.
- `preflight`: injects a minimal base/reset preflight when `true`.
