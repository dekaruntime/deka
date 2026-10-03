#!/bin/bash
# One round-robin batch on this Mac: GPUI (main's glue) against the new window.
# Every window opens on a CoreGraphics virtual display that cmp-batch creates
# for the batch and removes afterwards; nothing appears on a physical screen,
# and the batch fails if any window does (see src/bin/batch.rs).
# Needs an unlocked session: a locked screen occludes every window, virtual
# displays included.
# Usage: run-batch.sh OUT_DIR [ROUNDS] [cmp-batch options, e.g. --plan start]
# Plans: content (counter: read the window back until its frame shows),
# start, idle, animate, pixels. `--backends a,b` runs ./cmp-a and ./cmp-b
# from the build directory, so two builds of one backend can be compared
# (copy them there as cmp-before / cmp-after).
set -u
out=${1:?output directory}
rounds=${2:-5}
shift; shift || true
bin=${CARGO_TARGET_DIR:?set CARGO_TARGET_DIR}/native
cd "$(dirname "$0")"
bash setup.sh
# deka's shipped profile for both binaries.
cargo build --profile native --bin cmp-batch || exit 1
cargo build --profile native --features gpui --bin cmp-gpui || exit 1
cargo build --profile native --features new --bin cmp-new || exit 1
if ioreg -n Root -d1 -a | grep -A1 CGSSessionScreenIsLocked | grep -q true; then
  echo "screen is locked: windows cannot be measured"
  exit 1
fi
mkdir -p "$out"
"$bin/cmp-batch" --bin "$bin" --out "$out" --rounds "$rounds" "$@"
