# Deka VM experiment

The Rust bytecode runtime for a **restricted subset of current DekaScript**.
The local DekaScript parser and checker are linked directly as a crate. No compiler subprocess,
JavaScript emission, V8, or React is involved. The public native CLI uses this backend; the historical V8 CLI remains a
separate crate. See [native commands](../../docs/dekascript/native-cli.mdx).

## Run

From the repository root:

```sh
export CARGO_TARGET_DIR="$PWD/.target" TMPDIR="$PWD/.tmp"
cargo build --locked --release -p deka_vm --features compiler,host
.target/release/dvm crates/deka_vm/examples/host.ds --grant-timer
```

The real `main` creates a captured counter, starts a Rust/Tokio timer, calls a
synchronous Rust operation while the timer is pending, awaits its completion,
and passes two closure results back into Rust. Expected result: `Number(83.0)`.
Omitting `--grant-timer` exits unsuccessfully with `permission denied: timer`.

The optional compiler feature also enables a precompiled path:

```sh
.target/release/dvmc crates/deka_vm/examples/host.ds .tmp/host.dvm.json
cargo build --locked --release -p deka_vm --no-default-features --features host --bin dvm
.target/release/dvm .tmp/host.dvm.json --grant-timer
```

`dvm-core` is a bytecode runner with no host adapter/Tokio dependency. `dvm` with
`host` adds the Rust adapter; adding `compiler` embeds the local frontend. These are experimental
commands, not a new installation flow for the production Deka CLI.

## What was recovered

Source: `dekaruntime/deka-studs-archive`, commit
`61c262e8d72168edb20d4330dd46973594df8037` (February 10, 2026).

- `src/stack.rs` is adapted from `crates/php-rs/src/vm/stack.rs` (smaller initial
  allocation and GC root iteration; unused methods removed).
- The instruction family and stack execution design derive from its
  `vm/opcode.rs`; operands, function representation and dispatch were rewritten.
- The indexed arena design derives from `core/heap.rs`. Its old free list did
  not destroy payloads immediately, and it had neither generation checks nor
  tracing. This experiment replaces that implementation with mark-and-sweep.
- The old `vm/engine.rs` alone is 15,085 lines, intertwined with PHP object,
  class, extension and reference machinery. It is **not** restored here.
- Compiler lowering, scheduler, host registry, tracing collector and closure
  cells are new. This is a small recovery/reimplementation experiment, not the
  archived PHP engine running unchanged. The upstream package described itself
  as MIT-licensed `php-rs` by Di Wu; see `THIRD_PARTY.md`.

## Execution and memory

DSC parses and checks source against an imported `vm:host` signature module
built from the registered Rust operations. Lowering emits versioned bytecode.
The registry generates checker signatures and validates calls/results at runtime,
so the example host API is not separately maintained in JavaScript or DS files.
Operation names are resolved through the registry at runtime in this first slice.
The generated signature functions only supply type metadata; their bodies are
never emitted or executed. Registered Rust handlers are the implementation.

The VM uses an operand stack, bytecode call frames, and generation-checked heap
handles. Each local binding has a heap cell; closures capture those cells and
can survive return from their creating function. Captures are conservative:
all visible local cells are captured, rather than only free variables. Block
locals remain rooted until their frame returns. Optimizing either is future work.

Lists and records have no mutation instructions in this slice. Assigning a value
shares it; mutable `let` rebinding changes a cell. This has **not** adopted PHP's
coercions, associative arrays or copy-on-write mutation semantics. Those would
need separate language decisions and tests. Current DSC still checks the source.

Collection runs at scheduler safepoints after bounded 256-instruction task
slices. Roots include the entry promise, active task promises, every frame's
locals and operand stacks. Tracing follows collections, closure cells and
completed promise results. Unreachable payloads are dropped, slots are reused,
and cycles are collectible. Arena capacity tracks high-water demand: reclamation
means reuse, not necessarily returning resident pages to the OS. This simple
collector prioritizes demonstrable correctness over low pause times.

Async functions create tasks. `await` on a pending promise parks that task at
its instruction pointer while other tasks continue. Rust futures receive the
executor's waker. Completing the entry function cancels remaining child work;
errors and explicit cancellation drop pending host futures and collect roots.
This is one application task scope, not a settled language-wide cancellation API.
Dropping a future only guarantees the Rust future is dropped: host adapters must
ensure external work/resources are cancelled appropriately.

## Host boundary

`Hosts::register(HostOp::new(...))` defines argument/result types, async behaviour,
optional capability and the Rust handler. Capabilities are denied until explicitly
granted by the embedding host; the demo grants `timer` only via its command flag.
This is an isolated host registry, **not** an alternate route into the production
host catalog. `crates/permissions/src/host_bridge.rs` was inspected; its richer
resource, byte and package-grant contracts are not ported here.

Only unit, number, bool and owned UTF-8 strings cross this boundary. Pending Rust
futures own their arguments and cannot retain raw VM heap handles. Opaque
resources, bytes, callbacks into DS, structured data transfer, package ownership
and integration with existing production service implementations remain future
work. No file/network/process operations are exposed by this demo.

## Supported slice and limits

Supported: local `const`/`let`, top-level state, named functions, recursion,
closures, explicit scalar parameters, imported host calls, async functions and
`await`, scalar literals, immutable record/list construction and access, binding
assignment, selected arithmetic/comparison, blocks, `if`, and C-style `for`.
Exactly one zero-argument `main` entry is invoked after module initialization.

Unsupported source forms fail compilation. Relative modules support
function/constant exports, and import cycles load with JavaScript module
semantics (run-time calls across a cycle work; a load-time read of an
export that is not set yet is a named error). Bare specifiers consume
DekaScript-only packages from `ds_modules/` per `deka.json` and `deka.lock`
(installing them is separate tooling). Self-imports, generics, enums/match, exceptions/try/catch, collection methods,
struct methods, default/tuple parameters, most operators and closures inside
loops. Forward references not already bound during lowering are rejected, even
where the current checker permits them. Source-level host failures currently
propagate to the embedding caller rather than typed DS `Result` values.

This is not a security sandbox. Bytecode validation checks format and basic
operands; execution checks stack underflow, types, calls and handles. There is
an instruction limit and call-depth limit, but no complete memory/resource quota
or hostile-bytecode verifier. Only run trusted source/artifacts.

The WASM browser preview uses this same VM. This does not imply broad language
conformance or V8 feature parity. An optional native UI adapter now connects a restricted DSX
component to the existing renderer; see [NATIVE.md](NATIVE.md).

## Verification

```sh
cargo test --locked --release --features compiler,host,ui,v8-control
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

Tests exercise source-to-Rust execution, surviving closures, recursion, concurrent
async tasks, bounded allocation, cycles, stale handles, cancellation/future drops,
host failures, capability denial, checker rejection, fuel limits and serialized
bytecode execution. The dedicated CI workflow exercises source and compiler-free
runtime paths. The public native CLI and browser runtime depend on this crate.

Initial executable-size and macOS memory results, raw samples and reproduction
commands are in [measurements/README.md](measurements/README.md).
