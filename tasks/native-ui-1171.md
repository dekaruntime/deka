# Native UI experiment evidence

Plan: https://github.com/zegadb/staff/issues/32#issuecomment-5903417949
Owners: dekaruntime/dsc#307 and dekaruntime/deka#1171.

Baseline: Deka 2048473883364ef70463f2915b4e23f11d7e7b54;
DSC 1a1afa230bf8e286af3d14389308056c7830d127. Both 0.53.7.
The current DSC workspace passed its baseline check; no rollback was necessary.

The native contract (`deka_native_ir`) lives beside its producer in DSC.
It has no parser or renderer dependency. Its `program` feature adds only the
reloadable program format. Deka's development interpreter opts into that feature;
generated applications use only the owned UI values, not program interpretation.

Scope and reproducible commands are in docs/dekascript/native-ui-experiment.md.
Native pixel/input checks, source watcher checks, generated-code execution,
checks/Clippy, measurements and platform limitations are recorded in the PR.

## First measured slice (Intel macOS, 2026-09-29)

- DSC pin: b6a82fd48fe29497c553783b2cc448e52966dc2b; PR dekaruntime/dsc#308.
- Production executable: 4,472,572 bytes (4.27 MiB), x86_64 Mach-O.
- Open production window: vmmap physical footprint 25.4M, peak 25.7M; RSS 45,060 KiB at the sampled point.
- Open development window: vmmap physical footprint 26.7M, peak 27.1M.
- These are snapshots of the small counter on this Mac, not budgets or cross-platform guarantees. Metal shaders compile at runtime in this build.
- Native source edits visibly changed the heading while preserving the user's count of four. An invalid identifier was rejected with the existing window retained. The user confirmed native pointer clicks changed the counter.
- Generated Rust application `--exercise 7` prints Count: 7; its native window was also opened and exercised.
- Runtime tests: 5 pass. Disabling Host::click dispatch: 3 fail, 2 pass; restoring dispatch: all 5 pass.
- Compiler tests: 3 pass. Native compiler library WASM check passed (`cargo check --locked -p deka_native_compile --lib --target wasm32-unknown-unknown`). Browser preview renderer and browser-facing native compiler exports are not implemented.
- Full Deka workspace/all-target check and workspace/all-target/all-feature Clippy passed against the current baseline. Existing warnings remain outside the new crates; the new crates produced no warnings.

-codex
