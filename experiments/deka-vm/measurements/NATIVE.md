# Native window comparison — September 30, 2026

Intel iMac, macOS 26.7, Rust 1.96.0. Both engines drive the same native renderer,
fonts, styles and 560×300 counter window. The VM executes the markup and handlers
in `examples/counter.dsx`; V8 executes the explicitly translated control fixture
`examples/counter-control.js`. Full scene equality is tested after 0, 1 and 17
clicks. This compares a working UI, not just an empty engine process.

| Build | Executable bytes | Executable MiB | Median RSS MiB | Median physical footprint MiB |
|---|---:|---:|---:|---:|
| Minimal V8 + renderer | 47,524,092 | 45.32 | 53.99 | 32.6 |
| VM + renderer, precompiled bytecode | 3,216,892 | 3.07 | 44.08 | 28.7 |
| VM + renderer + embedded DSC compiler | 3,886,652 | 3.71 | 43.04 | 28.1 |

The VM executable is 93.2% smaller without the compiler and 91.8% smaller with
it. Physical footprint drops about 12–14% in this sample. The renderer accounts
for much of the shared memory cost. The small ordering difference between the
two VM variants is sampling variation, not evidence that adding a compiler saves
memory. RSS and physical footprint are separate OS metrics.

## Method and limits

All three executables use the experiment's size profile: `opt-level=s`, LTO,
one codegen unit, panic abort, and stripped symbols. Build feature sets separately:
enabling `v8-control` in a VM build also brings V8 into that executable. A CI
dependency guard ensures the VM-only feature set does not depend on V8.

The bytecode file is approximately 1.9 KB, separate from the executable. Sizes
exclude OS libraries, installer metadata and signing/notarization packaging.
The renderer revision is `fd8e705d9622e19bf19fd78ad963971c15925c2d`; DSC is
`ff1d486813088df977aa5c3fe5400995ec67a710`.

We launched each variant three times in alternating order. Five seconds after
each launch, we read `ps -o rss= -p PID` (KiB) and
`vmmap -summary PID` (physical footprint), then terminated only that sample's
process. There was no automated interaction. The user reported interacting with
a comparison window during the pass, so these are approximate visible-window
measurements on an active development desktop, not controlled untouched idle
benchmarks. `vmmap` rounds its displayed footprint; converted byte values in
the JSON do not imply byte-level precision.

Raw samples, binary SHA-256 hashes and medians are in `macos-native.json`.
The V8 control is intentionally minimal: no React, networking, full Deka host
catalog or project loader. The experimental VM also supports only a language/UI
subset. These numbers do not establish full runtime feature parity or Linux
memory use. Earlier native-backend measurements used a different release profile
and fixture; the table above rebuilds both controls with matching settings.

## Reproduce

From the repository root, after creating its normal `.tmp` directory:

```sh
export CARGO_TARGET_DIR="$PWD/.target" TMPDIR="$PWD/.tmp"
cargo build --locked --release --manifest-path experiments/deka-vm/Cargo.toml --no-default-features --features compiler,gpu --bin dvm-ui --bin dvmc
cp .target/release/dvm-ui .tmp/vm-source
.target/release/dvmc experiments/deka-vm/examples/counter.dsx .tmp/counter.dvm.json Counter
cargo build --locked --release --manifest-path experiments/deka-vm/Cargo.toml --no-default-features --features gpu --bin dvm-ui
cp .target/release/dvm-ui .tmp/vm-bytecode
cargo build --locked --release --manifest-path experiments/deka-vm/Cargo.toml --no-default-features --features gpu,v8-control --bin dvm-v8-ui
cp .target/release/dvm-v8-ui .tmp/v8-control

.tmp/vm-source experiments/deka-vm/examples/counter.dsx
.tmp/vm-bytecode .tmp/counter.dvm.json
.tmp/v8-control
```

Run one window at a time. Record the PID you launched and sample that PID only.
The binaries also accept `--exercise 3` and `--snapshot 3` for headless behavior
and scene comparisons. Their normal window path is used for the memory table.

-codex
