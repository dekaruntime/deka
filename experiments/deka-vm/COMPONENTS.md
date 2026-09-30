# VM component experiment

`Component` keeps a VM entry result alive as a record of named callbacks. The
host calls callbacks with scalar or string-list values. `view` returns a DSX
node tree; dynamic children, classes, and input values are evaluated when the
host requests a frame. A frame contains shared native renderer nodes plus input
specifications for the platform's editable text adapter.

Native events invoke closure handles, then the host requests a new frame. This
is an explicit event protocol, not React reconciliation or a dependency-tracking
binding implementation. The host owns asynchronous work and delivers completion
callbacks; UI event handlers are synchronous in this experiment.

The compiler now lowers one-argument `Array.map` callbacks, list length, and
immutable list append. Each iteration creates fresh callback parameter cells,
so row event handlers capture their own value. Append currently copies the
result list; this is suitable for the launcher's six rows, not large collections.
Frames pin their dynamic closure graphs and collect obsolete frames after
rendering. Invalid event IDs and missing component callbacks return errors.

`input` supports `value`, `placeholder`, `onInput`, and `onKeyDown`. Its layout
rectangle belongs to the shared renderer; actual text editing belongs to the
native host. This change does not add browser input parity or production language
support. The experiment still uses the pinned DSC compiler and its syntax limits.

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

`cargo test --locked --release --manifest-path experiments/deka-vm/Cargo.toml
--features compiler,host,ui` covers the runtime and component adapter. The new
component test invokes the second mapped row over 300 frames, checks captured
identity and bounded heap slots, and clears the list. Disabling event execution
makes its selected-value assertion fail; restoring it passes.

-codex
