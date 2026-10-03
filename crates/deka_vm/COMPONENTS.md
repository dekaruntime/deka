# VM component experiment

`Component` keeps a VM entry result alive as a record of named callbacks. The
host calls callbacks with typed host values. `view` returns a DSX
node tree; dynamic children, classes, and input values are evaluated when the
host requests a frame. The component owns a lasting Rust node store; a frame
contains presentation snapshots plus input specifications for the platform's
editable text adapter. Compatible positional elements/text retain internal
identities. Removed or replaced nodes receive fresh identities on insertion.

Native events queue closure handles onto the persistent VM scheduler, then the
host requests a new frame. Async handlers and their child tasks survive the
event call. Native hosts drive the bounded turn/ready/wake interface; bindings
still evaluate synchronously. This does not implement dependency tracking or
property ownership for imperative edits. Keyed-list identity remains separate
from the current positional matching policy.

The compiler now lowers one-argument `Array.map` callbacks, list length, and
immutable list append. Each iteration creates fresh callback parameter cells,
so row event handlers capture their own value. Append currently copies the
result list; this is suitable for the launcher's six rows, not large collections.
Frames pin their dynamic closure graphs and collect obsolete frames after
rendering. Invalid event IDs and missing component callbacks return errors.

`input` supports `value`, `placeholder`, `onInput`, and `onKeyDown`. Its layout
rectangle belongs to the shared renderer; actual text editing belongs to the
native host. This does not add browser input parity. Source compiles through the
local DekaScript compiler crate; a standalone DSC process is not involved.

## Packaging custom host programs

Applications with their own host catalog compile their DSX against that catalog,
then pass the validated payload to the existing Tauri-backed packaging tool:

```sh
dvm-package deka.json path/to/runtime --out output --bytecode app.dvm.json
```

Without `--bytecode`, the original compile-and-package behavior is unchanged.
The runtime, bytecode and explicit resources are staged into the app bundle;
the compiler and Tauri bundler remain build-time dependencies.

## Validation

`cargo test --locked --release --manifest-path crates/deka_vm/Cargo.toml
--features compiler,host,ui` covers the runtime and component adapter. The new
component test invokes the second mapped row over 300 frames, checks captured
identity and bounded heap slots, and clears the list. Disabling event execution
makes its selected-value assertion fail; restoring it passes.

`tests/node_identity.rs` drives actual component/session renders. It verifies
stable identities after state updates, fresh identities after removal or kind
replacement, and independent session stores. Bypassing the retained store
reuses a removed positional ID and fails; bypassing identity-aware presentation
replacement also fails. The browser preview tests check the same identity
behavior through real pointer events and scene snapshots, without a display.

-codex
