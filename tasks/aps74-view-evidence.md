# APS 74 View and counter evidence

Step 1 completes deka#1413, stacked on standalone extraction PR #1414
(`279e47f8`). The base was merged into this already-pushed draft; no history was
rewritten. No VM source was edited after the approved extraction.

`View` is a consumed builder. Signals and closures bind retained NodeHandles
in the extracted #1306 store. Direct text/attribute/class setters share the
store's authored/effective comparison; structural child slots call its existing
retention implementation. Registration owners contain effects/listeners only,
not another node model. Option/iterator content retains later static siblings.
Removed bindings unsubscribe and release event captures. Composed class values
track their own reads and patch once per event. Presentation snapshots do not
evaluate getters.

Rust closures accept typed Click/Input/KeyDown events. The existing Application
seam supplies clicks; typed input/key dispatch is exposed for editor integrations.
`launch`/`launch_with` consume UiApp on the existing native window. Renderer
routing indices stay internal; author closures receive events. The default
feature enables desktop; headless use disables default features.

## Behavioral validation

- 25 deka-ui tests: 13 original reactive tests, 8 integration View tests and
  4 ownership/class/lifetime unit tests. All pass, exit 0.
- Existing native reference suite (`./run.sh`): 1,199 tests, exit 0. Original
  VM tests/fixtures remain unchanged after extraction.
- Three actual counter handlers through launch's exercise path: exit 0,
  `Count: 3 Add one`.
- Actual desktop window: exit 0; `Presented Rust counter frame 1` is required
  by the validation driver. The window closes after that presented frame.
- Final release workspace/all-targets check and workspace/all-targets/all-features
  clippy: 0; existing backlog outside touched code.
- Strict deka-ui/native-IR all-targets/all-features clippy: 0, no warnings.
- Formatting: 0. Independent packaging check/build and relocated application
  validation are recorded with explicit exit codes in the evidence directory.

## Literal reverts

The committed proof script applies one actual source removal at a time,
requires exit 101 from a behavioral assertion (not a compiler failure), and
restores original source in finally. Five proofs pass:

1. Remove property patching: count remains 0 after the real event.
2. Remove event batching: the getter observes intermediate 1 before final 2.
3. Remove registration disposal: a detached binding still observes count.
4. Compare effective rather than authored text: a same-valued author update
   overwrites a direct Rust hand edit.
5. Prepare class defaults from effective text rather than retained element kind:
   an element hand edit loses its native row layout when its classes change.

All restored checks/tests pass. The earlier six reactive-core proofs remain
under `tasks/evidence/aps74-reactive`. Current View patches, logs, exit codes and
proof script are under `tasks/evidence/aps74-view`.

Serial release builds, machine sccache, checkout-local `.target` and
`TMPDIR=/Volumes/Projects/codex/.tmp-deka`. No helpers, CI polling, merge, approval,
force push, tag, publishing or release. Docs have their own View page and the
reactive page now points at it. Macros and tour parity remain the subsequent PRs.

-codex
