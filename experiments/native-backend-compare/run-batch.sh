#!/bin/bash
# One round-robin batch on this Mac: GPUI (main's glue) against the new window.
# Every window runs a fixed number of frames or seconds and closes itself.
# Needs an unlocked screen: a locked or sleeping display occludes every window.
# Usage: run-batch.sh OUT_DIR [ROUNDS]
set -u
out=${1:?output directory}
rounds=${2:-5}
bin=${CARGO_TARGET_DIR:?set CARGO_TARGET_DIR}/native
cd "$(dirname "$0")"
./setup.sh
# deka's shipped profile for both binaries.
cargo build --profile native --features gpui --bin cmp-gpui || exit 1
cargo build --profile native --features new --bin cmp-new || exit 1
mkdir -p "$out"
log="$out/batch.log"
: > "$log"
if ioreg -n Root -d1 -a | grep -A1 CGSSessionScreenIsLocked | grep -q true; then
  echo "screen is locked: windows cannot be measured" | tee -a "$log"
  exit 1
fi
run() {
  echo "## $*" >> "$log"
  "$@" >> "$log" 2>&1
  echo "exit $?" >> "$log"
  sleep 1
}
for round in $(seq "$rounds"); do
  for backend in gpui new; do
    for app in counter settings world; do
      run "$bin/cmp-$backend" --app "$app" --first-frame
    done
  done
done
for backend in gpui new; do
  run "$bin/cmp-$backend" --app settings --idle 10
  run "$bin/cmp-$backend" --app world --warmup 60 --frames 600
done
for backend in new gpui; do
  run "$bin/cmp-$backend" --app settings --idle 10
  run "$bin/cmp-$backend" --app world --warmup 60 --frames 600
done
for scene in counter counter-focused init-app settings world-title world-town world-room text-fallback text-sizes transforms paints; do
  run "$bin/cmp-gpui" snap --scene "$scene" --out "$out"
done
run "$bin/cmp-new" snap --out "$out"
run "$bin/cmp-new" compare --dir "$out"
grep '^RESULT' "$log"
