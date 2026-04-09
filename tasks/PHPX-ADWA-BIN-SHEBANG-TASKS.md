# PHPX ADWA Bin + Shebang Track

Status: active  
Owner: runtime + adwa  
Scope: replace command stubs with runtime-native command discovery/execution

## Goals

- Make command execution module-driven through `deka.json` `bin` entries.
- Support canonical shebang `#!/usr/bin/deka` (and env fallback).
- Keep behavior consistent between ADWA and real server runtime.
- Remove UI-owned command magic and move lifecycle to runtime process model.

## Phase 1: Manifest Bin Discovery (ADWA)

- [x] Add `bin` manifest parser in ADWA command resolver.
- [x] Support `bin` as string and object map.
- [x] Keep compatibility with existing `type: "cli-adwa"` + `main`.
- [x] Add command shadowing order: builtins -> local bins -> global bins.
- [x] Add tests for duplicate command names and deterministic resolution.

## Phase 2: Shebang + Script Entry

- [x] Recognize `#!/usr/bin/deka` and `#!/usr/bin/env deka`.
- [x] Route shebang scripts through `deka run` semantics.
- [x] Pass argv exactly as POSIX shell would (including script path + args).
- [x] Add tests for executable script files in VFS.

## Phase 3: Runtime-Owned Foreground Jobs

- [x] Move foreground job ownership from browser UI into ADWA runtime process layer.
- [x] Ensure `deka serve` blocks shell prompt as foreground process.
- [x] Wire `Ctrl+C` to runtime interrupt/signal, not UI stop callbacks.
- [x] Add `wait`/exit semantics validation tests.

## Phase 4: Port/Preview Integration

- [x] Expose listening ports from runtime process table.
- [x] Drive preview URL from runtime port forwarding metadata.
- [x] Preserve clean URL navigation for serve mode routes.

## Phase 5: Migration + Cleanup

- [x] Pilot migrate one command module (`ls`) to explicit `bin` map.
- [x] Migrate existing ADWA command modules to explicit `bin` entries.
- [x] Drop legacy `type: cli-adwa` command fallback from resolver.
- [x] Remove remaining direct UI command-module shortcuts.
- [x] Document command packaging and shebang usage in PHPX docs.
- [x] Add contributor notes in `AGENTS.md` for command runtime testing.

## Exit Criteria

- [x] `ls/pwd/touch/...` execute through runtime command discovery only (`cd/history/open/run` intentionally remain shell builtins).
- [x] `deka serve` behaves like a true foreground process in shell UX.
- [x] Script files with deka shebang run directly.
- [ ] Same command behavior in ADWA browser and server-host runtime.
  parity harness: `scripts/test-adwa-command-parity.sh` (bridge smokes + browser e2e)
