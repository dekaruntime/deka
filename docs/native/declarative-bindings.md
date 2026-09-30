# Declarative bindings without React

`examples/native/bindings.dsx` is the source of the native counter experiment. It uses component-local `let` state, a `view` root, direct numeric text bindings and inline assignment handlers. The existing DSC parser/checker compiles it into native program IR; the Rust program host executes those expressions without React or V8. The website shell may still use React, but the component does not.

Run it with the matching experimental compiler:

```sh
.target/release/deka-native dev examples/native/bindings.dsx --compiler /absolute/path/to/dsc/.target/release/dsc-native
```

Build the host with `cargo build --locked --release -p deka-native --features runtime-shaders` on a Mac without the Metal compiler. The watcher preserves compatible state across edits, resets it when the component/state-slot signature changes, and keeps the last working program on compiler errors.

Each `ProgramApp` retains its own node tree and indexes dependencies of text/style expressions at mount. An assignment updates its state slot and evaluates dependent bindings. Rendering another frame without a state change performs no binding evaluations. Source replacement creates a new retained tree. Instances have separate state and caches.

`NativePreview.binding_stats()` reports tree builds, created nodes, binding evaluations and changed node IDs. The native tour displays these diagnostic counters. They count application-tree work, not layout/paint work: the existing renderer still takes a cloned scene-tree snapshot and performs layout/painting. This is a property-binding proof, not a completed incremental rendering pipeline.

The experiment supports one exported component, numeric literal initializers, arithmetic `+`, `-`, `*`, assignments `=`, `+=`, `-=`, `*=`, a single update per handler and existing native styles. Return types are inferred; `View` is not a new built-in type. `view` is root-only and fills the viewport by default. Module composition, props, keyed lists, general state types, batching multiple statements, async effects, focus scopes and navigation remain future work. Legacy conditional presence may rebuild the tree; legacy `useState` source remains accepted. The general V8/React backend and generated-Rust production emitter are unchanged.

Tests exercise node allocation identity, affected-binding counts, independent instances, source replacement, reset, compiler-error recovery and real browser input. Disabling binding propagation must make the counter test fail.
