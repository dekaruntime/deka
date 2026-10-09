# APS 74 phase 4 markup-patch core — deka#1440

## Approach and prior art

- [Dioxus RSX hot reload](https://dioxuslabs.com/learn/0.7/essentials/ui/hotreload/) uses its parser in both macros and dev tooling, reusing compiled expressions and updating static structure without recompiling. Its literal-prop handling is broader than this core. We adopt the template/compiled-slot separation, with a shared classifier and explicit rebuild fallback.
- [Slint live preview](https://docs.slint.dev/latest/docs/slint/guide/tooling/live-preview/) supports watching `.slint` definitions in a running native application. Its declarative-language interpreter is a good iteration model; adopting a second UI language/interpreter conflicts with APS 74's ordinary-Rust authoring decision.
- [Makepad](https://github.com/makepad/makepad) combines a live-editable design language with its UI runtime. We keep that separation between reloadable design data and compiled application logic, while retaining `view!`, typed Rust props and deka's existing store instead of introducing another design DSL.
- [Subsecond](https://docs.rs/subsecond/latest/subsecond/) patches compiled Rust through jump tables and an external compiler/linker protocol. Its documented struct-layout/global/thread-local constraints make it a larger and different state-preservation contract. Binary patching is unnecessary for static markup; normal Cargo rebuilding is the conservative fallback for logic/types.

## Architecture

The development macro emits a source callsite (absolute file + original line/column), the original template and compiled file snapshot, and preorder slot annotations. Source edits use their original compiled template ordinal, so changing line offsets does not break identity. The same `deka-ui-hot-reload` parser and compatibility policy are used by the macro, runtime, and Rust Cargo supervisor.

`View` remains a transient builder. Mounted template metadata holds the existing registration anchors, with weak registry ownership; signal, event, component-state and child registrations retain their established lifetimes. Edits produce transient plans validated before the shared `deka_native_ir::tree::Tree` commits them. Opaque component/compiled-expression roots are grafted in place, and static property patches use the store's existing authored/effective ownership logic. No second retained tree and no component re-execution.

The native host's existing 100 ms live turn checks a shared workspace/local-path-dependency snapshot on the UI thread. Both the application and supervisor wait for a save to remain stable for at least 20 ms before classifying it. All edited Rust files participate, so logic changes in a different file block markup patches. Components mounted after a save replay the current template. The supervisor uses a 50 ms source snapshot loop, incremental Cargo builds and explicit child PIDs; it owns the restart path. The user command is `cargo deka dev`, from the small `deka-ui-dev` crate. It avoids routing Rust applications through the retired DekaScript `deka dev` implementation. No product configuration environment variables or arbitrary Rust evaluation.

## Deliberate core boundary

Literal component props, additions/removals of Rust slots, markup inside opaque expressions/component children, reparenting compiled components, movable structural Rust child slots, root-count/kind changes, builder decorations chained after `view!`, and class-base changes combined with compiled class toggles take the visible rebuild path. The remaining task is typed mutable prop slots and movable structural-slot ownership, with state-preservation tests. These are not implemented or claimed here.

Browser transport is deferred: the browser cannot poll the author's local source files, and needs a separate dev server/connection and scheduling bridge. The compiled tour/browser behavior must continue to pass unchanged. Release application code excludes all development payload/metadata/polling/patch code under feature + debug + native cfg; the Rust supervisor is a separate executable.

## Validation

Reproducible probes: `python3 scripts/rust-ui-hot-reload/check.py` and `python3 scripts/rust-ui-hot-reload/release.py`. Both use the existing checkout target directory and never open a visible window. The disk-edit driver restores every edited fixture in `finally` and kills only its own explicit child PIDs.

Local logs live in ignored `tasks/evidence/ui-hot-reload/`; acceptance numbers and literal-revert outcomes belong in the PR body. The phase-4 task remains partial until the typed-prop and structural-slot work is done.

-codex
