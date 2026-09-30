# QuickJS feasibility experiment — deka#1175

Base: shared native renderer PR #1174, commit
`67e669b4bb27ee9386b150c8567c171134aaf2d5`. The experiment is stacked on that
unmerged branch. Production Deka still uses V8/deno_core. This PR does not select
a new production engine or introduce a supported dual-backend product.

## What was built

The existing native benchmark now shares one `App<B>` and one render/event path.
There are separate Rust, V8 and QuickJS build configurations, with a distinct
QuickJS executable so workspace `--all-features` checks remain meaningful.
All variants execute 41 -> 42 before opening a window, retain their backend,
and use that backend for subsequent counter events. Both JavaScript variants
use the same `increment(n)` source and evaluate a call on each event.

QuickJS here means **QuickJS-NG 0.16.2**, bundled by `rquickjs` / `rquickjs-sys`
**0.14.0**, not Bellard's upstream build. Rust bindings own value lifetimes;
this experiment has no handwritten retain/release or unsafe blocks. The
`loader` and `futures` binding features are enabled. The compiler is a dev-only
dependency, so it is excluded from the measured executable.

## Measured results

Measured September 29, 2026 local time (UTC timestamp in the evidence JSON),
Intel macOS 26.7, Rust 1.96.0. All three variants rebuilt from this experiment.

| Backend | Stripped bytes | MiB | Gzip MiB | Median physical MiB | Median RSS KiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| rust | 4,805,948 | 4.58 | 2.04 | 28.2 | 44,032 |
| v8 | 50,562,996 | 48.22 | 18.21 | 32.5 | 55,464 |
| quickjs | 5,887,636 | 5.61 | 2.56 | 28.2 | 44,360 |

QuickJS adds **1.03 MiB** over the Rust control. The complete QuickJS counter is
**88.4% smaller** than the V8 counter (42.61 MiB saved). Median idle physical
footprint is **4.3 MiB lower** than V8. QuickJS and Rust have the same rounded
median physical footprint in this small sample; that does not imply a free heap
or zero runtime memory cost. No CPU throughput or peak-load claim is supported.

Raw measurements: [measurements.json](evidence/quickjs-1175/measurements.json).
Binary hashes identify exactly the copies measured. The dependency graph for the
QuickJS build contains rquickjs and no deno_core, V8, or DSC compiler.

## Linux follow-up on demon

At Sami's request, the same source commit `85b57ab541c333ea6085a2e7c7b6ba804df7950b`
was cloned to `/home/sami/Projects/codex/deka` on demon and built natively with
Rust 1.95.0 on Ubuntu 26.04.1 LTS, x86_64, kernel 7.0.0-27-generic.
All three release builds and all six execution tests pass. Each stripped binary
passes the actual three-event exercise before sampling.

There is no desktop session. An owned Xvfb server supplies a 1024x768x24 display,
and Mesa lavapipe supplies software Vulkan. Each native app opens a 560x300
window. The harness verifies that window is mapped and the selected Vulkan
library is present before accepting a sample. It terminates only its own app
and Xvfb child processes. The Xvfb process is excluded from app memory figures,
as macOS WindowServer was excluded from the Mac process measurements.

The machine was missing the X11 xkb library. Ubuntu packages
`libxkbcommon-x11-0` / `libxkbcommon-x11-dev` 1.13.1-1 and `libxcb-xkb1`
1.17.0-2ubuntu1 were downloaded and extracted under this checkout's `.tmp`.
They were supplied via linker/runtime search paths; no privileged installation
was needed. System/shared libraries are excluded from executable file size.

All figures below are MiB, medians of three launches for memory:

| Backend | Executable | RSS | PSS | USS | Executable mappings resident |
| --- | ---: | ---: | ---: | ---: | ---: |
| rust | 17.86 | 57.15 | 44.69 | 34.10 | 5.91 |
| quickjs | 19.09 | 57.81 | 45.37 | 34.75 | 6.40 |
| v8 | 65.22 | 70.14 | 57.63 | 47.01 | 15.19 |

QuickJS adds **1.23 MiB** of executable size and **0.66 MiB RSS** over the Rust
control on this host. It saves **46.13 MiB of executable size** and **12.33 MiB
RSS** versus V8 in this small idle workload. These are sample deltas, not
universal runtime costs. All sampled swap values were zero.

The executable-residency result directly answers the file-versus-memory
question: the V8 build's executable is 65.22 MiB on disk, its executable mappings
span 65.23 MiB, but only **15.19 MiB** of those mappings are resident in the
process at the sample point. The rest of its 70.14 MiB RSS comes from other
mappings and allocations. The file is not copied wholesale into the process.

RSS counts resident pages. PSS apportions shared pages among the processes
mapping them. USS here sums private clean/dirty and private hugetlb pages.
These are Linux `/proc/<pid>/smaps` and `smaps_rollup` values; none is labelled
as the macOS physical-footprint metric. Definitions:
https://docs.kernel.org/filesystems/proc.html

For a closer cross-platform metric, QuickJS RSS was 43.32 MiB on the Mac and
57.81 MiB here. The graphics backend, toolchain and host differ, so this does
not isolate an OS-only effect. The larger Linux Rust control also shows that
much of the platform's file/memory baseline exists without either JS engine.

Raw sample values, hashes, toolchain and driver metadata:
[linux-measurements.json](evidence/quickjs-1175/linux-measurements.json).
The measurement script's recorded SHA-256 matches the committed script.
Detailed mappings/window checks remain in the remote checkout's
`.tmp/linux-comparison`. The copied local log `.tmp/linux-tests.log` records
all six test results.

To rerun on that host after building the three binaries as above:

```sh
cd /home/sami/Projects/codex/deka
export LD_LIBRARY_PATH="$PWD/.tmp/linux-libs/usr/lib/x86_64-linux-gnu"
python3 .tmp/measure-native-backends-linux.py .tmp/linux-comparison
```

The tracked script is `scripts/measure-native-backends-linux.py`; the `.tmp`
copy was transferred for execution before the results commit. Remote builds
used `cargo --config 'build.rustc-wrapper="sccache"'`, per-checkout target/temp
directories, and the matching `LIBRARY_PATH` for the extracted X11 libraries.
The remote source tree remains at the measured commit; the follow-up commit
adds only the measurement script, evidence and documentation.

## Execution evidence and limits

`cargo test --locked --release -p deka_native_bench --features quickjs-backend --test quickjs`
executes six tests:

- Three native counter events run through QuickJS and change rendered output.
- DSC f3524ef8 compiles a real `.ds` module; QuickJS imports it and executes a
  closure and string conversion. Missing module imports fail.
- DSC compiles a real `.dsx` component. QuickJS loads the repository's vendored
  React/JSX production code, creates a button element, invokes its retained
  handler, collects garbage, invokes it again, and observes changed text.
- A compiler-emitted async function composes with a genuinely suspended Rust
  timer future exposed as a JS promise. The test observes host completion and
  the value 42, and checks a rejected promise's error text.
- A Rust callback receives a JS typed array and computes its sum; a Rust-thrown
  exception is caught by JS with the expected type/message.
- A thousand self-referential objects are released and collected; object counts
  return near baseline. An interrupt aborts an infinite loop and the context
  subsequently executes another expression.

The DSX fixture deliberately uses a named handler and module state. Inline
callbacks in components can make DSC emit `useCallback`; those, `useState`,
effects and reconciliation require a real component lifecycle. This experiment
**does not implement that lifecycle**. The DSX test examines actual JS element
objects but does not feed them into the native window. The native counter
still uses the existing fixed Rust UI tree with a JS increment backend.

No production filesystem/network permission bridge, HTTP server, package graph,
N-API adapter, WASM executor, HMR, inspector, full conformance corpus or
cross-platform validation is included. These tests show feasibility, not
replacement-runtime compatibility. The exception probe is not a security test.

Mutation evidence: replacing the QuickJS event call with a no-op (while keeping
the startup 41 -> 42 check passing) makes the native event test fail with
`Count: 0` versus `Count: 3`. Restoring the call makes all six tests pass.

## Validation

- Release QuickJS, Rust and V8 native builds pass; all three shipped copies
  execute `--exercise 3` with `Count: 3`.
- Six QuickJS execution tests pass, zero ignored. The event mutation fails and
  restoring it passes.
- `cargo check --locked --workspace --all-targets` passes.
- `cargo clippy --locked --workspace --all-targets --all-features` passes with
  existing warnings elsewhere; no diagnostics in the changed benchmark crate.
- `cargo build --locked --release -p cli` passes on macOS.
- QuickJS normal dependency graph excludes deno_core, V8 and the compiler.
- `cargo fmt -p deka_native_bench -- --check`, `git diff --check`, and Python
  measurement-script syntax validation pass.

## Coupling audit

At the base commit, a lexical inventory of Rust files referencing `deno_core`,
`serde_v8`, `v8::`, or `deno_napi`, excluding the experiment crates, finds
**32 files / 260 matching lines across seven crates**. This includes tests and
comments and is an inventory, not an effort estimate; indirect coupling is
larger. Eight crate manifests directly depend on deno_core when including the
native V8 benchmark. The compiler's emitted JavaScript and shared native renderer
can be reused, as the probes demonstrate.

| Integration | Existing locations | What a switch requires |
| --- | --- | --- |
| Runtime and scheduling | `crates/pool/src/isolate_pool/worker_core.rs`, `worker_execution.rs` | Replace JsRuntime ownership, promise/event-loop driving, interruption and heap metrics while preserving request lifecycle. |
| Module loading | `crates/pool/src/esm_loader.rs`, `esm_loader/import_meta_ops.rs` | Keep resolution/verification rules, adapt the Deno loader trait and import.meta operation to QuickJS. |
| Native capabilities | `crates/deka_host/src/modules/` | Replace op2/Extension/OpState bindings, value conversion and exception mapping; retain and re-verify existing permission checks. |
| Engine-specific values | `crates/pool/src/isolate_pool/helpers.rs`, `crates/engine/src/dispatch.rs`, `crates/http/src/` | Replace V8 handles and serde_v8 marshalling at request/response boundaries. |
| Code cache | `crates/pool/src/isolate_pool/worker_compile.rs` | Replace V8 cached scripts with version-specific QuickJS artifacts or source loading; old cached bytes cannot be reused. |
| Native addons | `crates/platform_server/src/lib.rs` | deno_napi is currently installed as an extension; it cannot simply be reused by QuickJS. |
| Permissions wiring | `crates/modules_common/src/lib.rs` | Preserve policy semantics while removing Deno extension-state assumptions. |
| WASM and debugging | Engine services | Supply replacements for V8 capabilities if required; neither was exercised here. |

Recommendation: the size result justifies a further bounded host-adapter spike,
not a wholesale migration. The most informative next slice would execute one
real Deka handler through the existing module/security rules with one synchronous
and one asynchronous host operation. Then compare the same conformance runner
and realistic workloads before choosing an engine. Until then V8 remains the
production baseline.

## Reproduce

Use this checkout's `.target` and private `.tmp` per AGENTS.md. Do not bypass
sccache. Build sequentially; at most two Rust builds may run on the machine.

```sh
export CARGO_TARGET_DIR="$PWD/.target" TMPDIR="$PWD/.tmp"
mkdir -p "$TMPDIR/quickjs-comparison"
chmod 700 "$TMPDIR"
cargo build --locked --release -p deka_native_bench --features runtime-shaders --bin deka-native-backend-bench
cp .target/release/deka-native-backend-bench .tmp/quickjs-comparison/backend-rust
cargo build --locked --release -p deka_native_bench --features runtime-shaders,v8-backend --bin deka-native-backend-bench
cp .target/release/deka-native-backend-bench .tmp/quickjs-comparison/backend-v8
cargo build --locked --release -p deka_native_bench --features runtime-shaders,quickjs-backend --bin deka-native-quickjs-bench
cp .target/release/deka-native-quickjs-bench .tmp/quickjs-comparison/backend-quickjs
strip .tmp/quickjs-comparison/backend-rust .tmp/quickjs-comparison/backend-v8 .tmp/quickjs-comparison/backend-quickjs
python3 scripts/measure-native-backends.py .tmp/quickjs-comparison
```

The measurement script validates three events in every binary, opens one native
window at a time, waits five seconds, captures `vmmap -summary` physical footprint
and `ps` RSS, and terminates only its own child PID. Three rounds rotate backend
order. JSON contains binary SHA-256s, byte/gzip sizes, all samples and medians;
raw vmmap output remains in the chosen directory. Its git HEAD identifies the
base when measured before commit; the report also records a working-diff hash.

Physical footprint and RSS are different metrics. Results exclude packaging,
OS frameworks, full Deka host services, compiler and workload peaks. Release
uses the workspace's default profile plus macOS strip; no per-engine size tuning.
Measurements were taken on an active Intel development Mac, not an isolated lab.

-codex
