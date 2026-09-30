# Native renderer connection

The `ui` feature connects the VM to the same retained node/scene renderer used by
our existing native experiments, pinned to deka commit
`fd8e705d9622e19bf19fd78ad963971c15925c2d`. It is a Cargo dependency; the
production workspace and original renderer are unchanged. `gpu` adds its GPUI
window adapter with runtime Metal shaders. DSC is also a direct crate dependency.

## Source to window

`examples/counter.dsx` contains the actual markup, state and handlers. The selected
`Counter` function runs once in the VM. JSX lowers to immutable records, lists,
handler closures and text-binding closures. No HTML, React, JavaScript or browser
is involved in the VM application.

The adapter pins the returned component graph as a GC root, converts its initial
structure to native nodes, and retains those nodes. A button event invokes the
corresponding VM closure. Binding closures are then reevaluated and changed text
is patched in place. Component state survives GC between events. Independent
application instances have independent heaps and state. Idle rendering executes
no DekaScript instructions.

This first adapter reevaluates all text bindings after each event. It does **not**
yet index dependencies, do incremental layout, support reactive structural
changes, dynamic classes, async UI handlers or HMR. It supports `view`, `div`,
`p`, `span`, `button`, literal `className`, synchronous `onClick` function literals,
and scalar text expressions. Runtime errors are displayed in the window. The VM's
existing async host support remains available in its command-line runner;
integrating that scheduler with the GUI event loop is future work.

## Run

From the repository root, using its usual build directories:

```sh
export CARGO_TARGET_DIR="$PWD/.target" TMPDIR="$PWD/.tmp"
cargo build --locked --release --manifest-path experiments/deka-vm/Cargo.toml --features compiler,gpu --bin dvm-ui --bin dvmc
.target/release/dvm-ui experiments/deka-vm/examples/counter.dsx
```

Precompile the source to omit DSC from the shipped executable:

```sh
.target/release/dvmc experiments/deka-vm/examples/counter.dsx .tmp/counter.dvm.json Counter
cargo build --locked --release --manifest-path experiments/deka-vm/Cargo.toml --features gpu --bin dvm-ui
.target/release/dvm-ui .tmp/counter.dvm.json
```

The same binary supports `--exercise N` to invoke actual click handlers without
opening a window and `--snapshot N` to serialize the shared renderer's resulting
scene (including geometry, glyph images and hit targets).

## Comparison control

`dvm-v8-ui` runs an explicitly translated JavaScript fixture in a real V8 isolate.
The fixture mirrors the DSX counter's state/handlers and emits the same tree.
Both controls use identical renderer revision, fonts, styles, 560x300 window,
release profile and shader backend. Tests compare their full native scenes after
0, 1 and 17 clicks. The JS fixture is a measurement control, not the compiler's
output and not production application code.

This follows the earlier minimal V8 backend comparison, rather than using the
much larger full Deka/React host. It excludes networking, full host catalogs,
React, project resolution and a separate compiler process on the V8 side. Thus
it measures a conservative minimal engine/renderer comparison, not full feature
parity. The embedded-DSC VM build is measured separately from the precompiled one.

See [the native measurements](measurements/NATIVE.md) for executable sizes,
visible-window memory samples, caveats and exact build commands.

For a double-clickable application configured through `deka.json`, see the
[Tauri packaging demo](packaging/README.md).
