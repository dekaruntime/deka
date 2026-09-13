#!/usr/bin/env bash
# Build dsc from dekaruntime/dsc main with the --dev transpile flag this lane
# needs (dsc#197 landed CompileOptions.dev, but the CLI flag is not in 0.51.2).
#
# Worktree-local only: clones into .cache/dsc and writes the binary to
# .cache/dsc/target/release/dsc. Never /tmp, never target/release/dsc (CI dump
# pairing keys off a sibling of the deka CLI).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/.cache/dsc"
PATCH="$ROOT/scripts/dsc-transpile-dev.patch"
mkdir -p "$ROOT/.cache"
if [[ ! -d "$SRC/.git" ]]; then
  git clone --depth 1 https://github.com/dekaruntime/dsc.git "$SRC"
else
  git -C "$SRC" fetch --depth 1 origin main
  git -C "$SRC" reset --hard origin/main
fi
git -C "$SRC" apply --check "$PATCH"
git -C "$SRC" apply "$PATCH"
cargo build --release -p cli --manifest-path "$SRC/Cargo.toml"
echo "built $($SRC/target/release/dsc --version) -> $SRC/target/release/dsc"
"$SRC/target/release/dsc" transpile --help | grep -F -- --dev
