# Native renderer connection

The `ui` feature connects the VM to the shared Rust node/scene renderer in this
workspace. `gpu` adds the winit + wgpu + vello_gpu window adapter. The local
DekaScript compiler is an optional crate dependency; precompiled payloads run
without that compiler.

## Source to window

`examples/counter.dsx` contains the actual markup, state and handlers. The selected
`Counter` function runs once in the VM. JSX lowers to immutable records, lists,
handler closures and text-binding closures. No HTML, React, JavaScript or browser
is involved in the VM application.

The adapter pins the returned component graph as a GC root, converts its initial
structure to a lasting Rust node store, and emits renderer snapshots from it.
Compatible elements/text at the same structural slot keep their identity; removal or
element-kind replacement creates a fresh identity. Internal aliases retain a
detached node and its subtree independently of the visible root; weak parent
and index links prevent ownership cycles. Renderer snapshots do not retain the
resource. A button event queues the
corresponding VM closure. Bindings whose observed source addresses changed
reevaluate and patch only changed authored properties. Component state survives GC between events. Independent
application instances have independent heaps and state. Idle rendering executes
no DekaScript instructions.

This adapter checks binding dependencies after events and progressed task turns. It
supports dynamic classes, conditional/mapped children and async handlers through
the backend-independent bounded turn/ready/wake interface. Runtime errors are
displayed in the window. Last authored properties are separate from effective properties, so unchanged
bindings preserve direct Rust edits. Public imperative node access and keyed-list
reconciliation remain later work. See
[the component adapter](COMPONENTS.md) and the user-facing
[native runtime reference](../../docs/dekascript/native-runtime.mdx).

## Run

From the repository root, using its usual build directories:

```sh
export CARGO_TARGET_DIR="$PWD/.target" TMPDIR="$PWD/../.tmp-deka"
mkdir -p "$TMPDIR"
chmod 700 "$TMPDIR"
cargo build --locked --profile native --manifest-path crates/deka_vm/Cargo.toml --features compiler,gpu --bin dvm-ui --bin dvmc
.target/native/dvm-ui crates/deka_vm/examples/counter.dsx
```

Precompile the source to omit the compiler from the shipped executable:

```sh
.target/native/dvmc crates/deka_vm/examples/counter.dsx "$TMPDIR/counter.dvm.json" Counter
cargo build --locked --profile native --manifest-path crates/deka_vm/Cargo.toml --features gpu --bin dvm-ui
.target/native/dvm-ui "$TMPDIR/counter.dvm.json"
```

The same binary supports `--exercise N` to invoke actual click handlers without
opening a window and `--snapshot N` to serialize the shared renderer's resulting
scene (including geometry, glyph images and hit targets).

## Comparison control

`dvm-v8-ui` runs an explicitly translated JavaScript fixture in a real V8 isolate.
The fixture mirrors the DSX counter's state/handlers and emits the same tree.
Both controls use the same lasting Rust store, renderer, fonts, styles, 560x300 window,
release profile and shader backend. Tests compare their full native scenes after
0, 1 and 17 clicks. The JS fixture is a measurement control, not the compiler's
output and not production application code.

This follows the earlier minimal V8 backend comparison, rather than using the
much larger full Deka/React host. It excludes networking, full host catalogs,
React, project resolution and a separate compiler process on the V8 side. Thus
it measures a conservative minimal engine/renderer comparison, not full feature
parity. The embedded-compiler VM build is measured separately from the precompiled one.

See [the native measurements](measurements/NATIVE.md) for executable sizes,
visible-window memory samples, caveats and exact build commands from the earlier
experiment. Those archived measurements use an earlier renderer/build; rerun
them before attributing those numbers to the current runtime.

For a double-clickable application configured through `deka.json`, see the
[Tauri packaging demo](../../tools/deka-package/README.md).
