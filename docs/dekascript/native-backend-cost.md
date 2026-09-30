# Native backend cost experiment

Measured 2026-09-29 on this Intel Mac (macOS 26.7, x86_64), Rust 1.96.0.
This answers the incremental cost of retaining V8 behind a native renderer.
It does not measure a full Deka server or claim npm compatibility.

`deka_native_bench` builds the identical GPUI/shared-scene UI in two configurations.
The Rust backend increments a number directly. The V8 backend holds a
`deno_core::JsRuntime` and executes a JavaScript increment function. Both call
41 -> 42 at startup; both pass `--exercise 3` with `Count: 3`. Thus V8 is initialized,
executed and retained, not merely linked and then eliminated or dropped.

| Measurement | Rust backend | V8 backend | Added by V8 |
| --- | ---: | ---: | ---: |
| Stripped executable bytes | 4,805,948 | 50,566,972 | 45,761,024 |
| Stripped executable MiB | 4.58 | 48.22 | 43.64 |
| Gzip executable bytes | 2,143,847 | 19,091,895 | 16,948,048 |
| Gzip executable MiB | 2.04 | 18.21 | 16.16 |
| Median idle physical footprint (MiB) | 28.4 | 32.9 | 4.5 |
| Physical-footprint range, three launches | 27.9–28.6 | 32.6–33.9 | — |
| Median RSS (KiB) | 44,060 | 55,820 | 11,760 |

Executable sizes exclude an app bundle/installer, signing/notarization packaging,
OS frameworks and distribution metadata. Gzip is a compression measurement,
not an actual installer. Builds use the same default Cargo release optimization
and macOS `strip` on copied binaries. The font is bundled in both builds.

Memory was sampled after five seconds with a visible native window, alternating
Rust/V8 for three launches each. `vmmap -summary PID` supplies physical footprint;
`ps -o rss= -p PID` supplies RSS. Physical footprint and RSS are different accounting
measures; neither is the process's reserved virtual address space. This was an
active development machine, not an isolated laboratory benchmark. These are idle
samples after a trivial backend call, not workload peaks. Larger heaps, packages,
host APIs, networking, async operations and the full Deka module/runtime layer are
not included and will increase the cost.

## Reproduce the builds

From this checkout, with `CARGO_TARGET_DIR` set to its `.target` and `TMPDIR` to
its private `.tmp` as usual:

```
cargo build --locked --release -p deka_native_bench --features runtime-shaders
cp .target/release/deka-native-backend-bench .tmp/backend-rust
cargo build --locked --release -p deka_native_bench --features runtime-shaders,v8-backend
cp .target/release/deka-native-backend-bench .tmp/backend-v8
strip .tmp/backend-rust .tmp/backend-v8
.tmp/backend-rust --exercise 3
.tmp/backend-v8 --exercise 3
```

Run each executable separately, wait five seconds, and capture `vmmap -summary`
and RSS for that exact PID. Close only the process started for the sample. Record
three alternating samples. The `runtime-shaders` feature is used because this
Mac has Command Line Tools rather than the complete Xcode Metal toolchain.

V8 remains Deka's production baseline. The Rust-only build is a measurement
control, not a commitment to a second production execution runtime. A later
QuickJS comparison is recorded in `tasks/quickjs-1175.md`; that isolated
experiment does not switch the production backend.

V8's embedding APIs and the `deno_core` engine are described at
https://v8.dev/docs/embed and https://github.com/denoland/deno_core.

-codex
