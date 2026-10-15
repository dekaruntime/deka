# APS 74 phase 4 hot reload — core and phase 2, deka#1440

## Approach and prior art

- [Dioxus RSX hot reload](https://dioxuslabs.com/learn/0.7/essentials/ui/hotreload/) uses its parser in both macros and dev tooling, reusing compiled expressions and updating static structure without recompiling. Its literal-prop handling is broader than this core. We adopt the template/compiled-slot separation, with a shared classifier and explicit rebuild fallback.
- [Slint live preview](https://docs.slint.dev/latest/docs/slint/guide/tooling/live-preview/) supports watching `.slint` definitions in a running native application. Its declarative-language interpreter is a good iteration model; adopting a second UI language/interpreter conflicts with APS 74's ordinary-Rust authoring decision.
- [Makepad](https://github.com/makepad/makepad) combines a live-editable design language with its UI runtime. We keep that separation between reloadable design data and compiled application logic, while retaining `view!`, typed Rust props and deka's existing store instead of introducing another design DSL.
- [Subsecond](https://docs.rs/subsecond/latest/subsecond/) patches compiled Rust through jump tables and an external compiler/linker protocol. Its documented struct-layout/global/thread-local constraints make it a larger and different state-preservation contract. Binary patching is unnecessary for static markup; normal Cargo rebuilding is the conservative fallback for logic/types.

## Architecture

The development macro emits a source callsite (absolute file + original line/column), the original template and compiled file snapshot, and preorder slot annotations. Source edits use their original compiled template ordinal, so changing line offsets does not break identity. The same `deka-ui-hot-reload` parser and compatibility policy are used by the macro, runtime, and Rust Cargo supervisor.

`View` remains a transient builder. Mounted template metadata holds the existing registration anchors, with weak registry ownership; signal, event, component-state and child registrations retain their established lifetimes. Edits produce transient plans validated before the shared `deka_native_ir::tree::Tree` commits them. Opaque component/compiled-expression roots are grafted in place, and static property patches use the store's existing authored/effective ownership logic. No second retained tree and no component re-execution.

The native host's existing 100 ms live turn checks a shared workspace/local-path-dependency snapshot on the UI thread. Both the application and supervisor wait for a save to remain stable for at least 20 ms before classifying it. All edited Rust files participate, so logic changes in a different file block markup patches. Components mounted after a save replay the current template. The supervisor uses a 50 ms source snapshot loop, incremental Cargo builds and explicit child PIDs; it owns the restart path. The user command is `cargo deka dev`, from the small `deka-ui-dev` crate. It avoids routing Rust applications through the retired DekaScript `deka dev` implementation. No product configuration environment variables or arbitrary Rust evaluation.

## Phase 2

Scalar component props now retain typed signal slots. `#[component]` analyzes
ordinary Rust parameters and emits development-only reactive reads for supported
markup and `move` closures; the production body and typed builder remain intact.
The original source template, spans and expression identities remain the
classifier's authority. Props consumed by initializers, shadowed/mutated locals,
forwarded component values or opaque macros have no mutable slot and rebuild.
Typed decoders prepare all writes before any tree/signal mutation, and writes
flush as one scope batch after the registry borrow ends.

Compiled child paths remain allocation identities. A development placement owns
the mutable parent anchor and following-slot boundaries; the existing retained
store replaces a dynamic slot at that boundary rather than sorting its old path.
This keeps empty slots, reordered lists and subsequent signal turns in their new
positions. Opaque fragments without a relocatable root use the complete-batch
restart path. Component template mounts find their current parent from retained
roots after a move.

`cargo deka dev --web` builds a debug `wasm32-unknown-unknown` cdylib, runs
wasm-bindgen and serves an atomic package snapshot on loopback. The shared
whole-workspace classifier produces cumulative source patches over SSE; clients
validate their compiled source and use the exact native planner/store. Browser
fallback requests trigger a WASM rebuild/reload. The old package survives failed
builds, new connections replay current edits, source rollback patches back to
baseline, and successful rebuilds reset the replay baseline. The dev-only host
adapter closes its connection on disposal. The production host now guards
reentrant drawing during DOM moves and preserves accessibility order/focus.

Remaining conservative fallbacks: added/removed compiled slots, opaque fragment
moves, root-count/kind changes, class-base changes combined with compiled toggles,
markup inside opaque expressions, and builder decorations outside `view!`.
Native and browser application machinery is feature + debug gated; the supervisor
remains a separate executable.

## Validation

Reproducible probes: `python3 scripts/rust-ui-hot-reload/check.py`,
`python3 scripts/rust-ui-hot-reload/phase2.py --case props` (also `slots` and
`fallback`, `fragments` and `ambiguous`), `node scripts/rust-ui-hot-reload/browser.mjs`,
`python3 scripts/rust-ui-hot-reload/revert-phase2.py`, and
`python3 scripts/rust-ui-hot-reload/release.py`. The probes use the existing checkout target directory and never open a visible window. The disk-edit driver restores every edited fixture in `finally` and kills only its own explicit child PIDs.

Release checks compare native consumer sizes/symbols and bound WASM sizes/exports
with the feature disabled/enabled. Acceptance numbers and literal-revert outcomes
belong in the PR; logs remain under ignored `tasks/evidence/ui-hot-reload/`.

-codex
