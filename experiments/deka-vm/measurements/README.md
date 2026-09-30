# Initial measurements — September 30, 2026

These initial figures cover the headless VM. The subsequent
[native window comparison](NATIVE.md) includes the renderer and a matched V8
control, measured separately with three visible-window samples per build.

Intel iMac, macOS 26.7, Rust 1.96.0. Release profile in this experiment:
`opt-level=s`, LTO, one codegen unit, panic abort, stripped symbols. These are
native Mach-O executable bytes, not gzip/download sizes. System libraries are
not bundled into these file sizes.

| Build | Executable bytes | Decimal MB | Median OS-reported peak RSS, allocation run |
|---|---:|---:|---:|
| Bytecode core, no host adapter | 399,288 | 0.399 | 1,105,920 bytes |
| Core + Rust/Tokio demo host, no compiler | 445,280 | 0.445 | 1,044,480 bytes |
| Core + host + embedded DSC parser/checker/compiler target | 1,135,584 | 1.136 | 1,634,304 bytes |

Peak RSS: `/usr/bin/time -l`, five fresh processes per variant, running
`examples/allocation.ds` (source build) or its bytecode (runtime builds).
All return `4,999,950,000`. Each performs 100,000 iterations, 1,000,014 VM heap
allocations and 2,700,015 VM instructions. Heap storage peaks at 110 slots and
finishes at zero live values. `macos-allocation.json` includes each RSS result
and the separately reported peak memory footprint. These two OS metrics are
not interchangeable. Small differences between variants are not evidence that
adding an adapter reduces memory use.

Idle sample: await the one-second timer in `examples/idle.ds`; sample
`ps -o rss= -p PID` after 200ms, convert KiB to bytes. Three fresh processes:

| Build | Median sampled RSS |
|---|---:|
| Runtime + host | 880,640 bytes (0.840 MiB) |
| Runtime + host + embedded compiler | 1,396,736 bytes (1.332 MiB) |

Raw samples are in `macos-idle.json`. The compiler arena has been dropped before
this idle sample. These are macOS-reported resident metrics for a very small
process, not a complete accounting of shared system memory, and not predictions
for Linux. A renderer, fonts, networking, project data, standard library, and a
real application's live graph are not included. There is **no matched V8
benchmark** here, and this is not a full Deka runtime replacement measurement.

## Reproduce

From the repository root (normal repository build environment):

```sh
export CARGO_TARGET_DIR="$PWD/.target" TMPDIR="$PWD/.tmp"
cargo build --locked --release --manifest-path experiments/deka-vm/Cargo.toml --features compiler,host
.target/release/dvmc experiments/deka-vm/examples/allocation.ds .tmp/allocation.dvm.json
/usr/bin/time -l .target/release/dvm experiments/deka-vm/examples/allocation.ds
# Record/copy this source-running binary before selecting another feature set.
cargo build --locked --release --manifest-path experiments/deka-vm/Cargo.toml --no-default-features --features host --bin dvm
/usr/bin/time -l .target/release/dvm .tmp/allocation.dvm.json
cargo build --locked --release --manifest-path experiments/deka-vm/Cargo.toml --no-default-features --bin dvm-core
/usr/bin/time -l .target/release/dvm-core .tmp/allocation.dvm.json
```

Restore `--features compiler,host` to run `.ds` files directly again. To sample idle RSS,
compile `examples/idle.ds` similarly, run its bytecode with `--grant-timer`, and
sample that specific process while it waits. No shared processes need stopping.
