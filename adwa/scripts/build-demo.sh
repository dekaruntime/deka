#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="$(cd "$ROOT_DIR/.." && pwd)"
OUT_DIR="${ADWA_DEMO_OUT_DIR:-$ROOT_DIR/dist/demo}"
COMPILER_DIR="$OUT_DIR/compiler"
DEMO_DIR="$OUT_DIR/demo"
CONSUMER_DIR="$OUT_DIR/consumer"

# This is deliberately a single DekaScript demo package. Its contents are the
# versioned compiler artifact, the DS inputs, and the DS-only browser consumer.
rm -rf "$OUT_DIR"
mkdir -p "$COMPILER_DIR" "$DEMO_DIR" "$CONSUMER_DIR"

"$REPO_DIR/runtime/scripts/build-deka-compiler-wasm.sh" "$COMPILER_DIR"
cp -R "$ROOT_DIR/ds-demo/." "$DEMO_DIR/"
cp "$ROOT_DIR/website/core/deka_demo_consumer.js" "$CONSUMER_DIR/deka_demo_consumer.js"

cat > "$OUT_DIR/deka-demo-package.json" <<'EOF2'
{
  "schema_version": 1,
  "compiler": "compiler/deka_compiler.wasm",
  "compiler_metadata": "compiler/deka_compiler.wasm.metadata.json",
  "compiler_checksum": "compiler/deka_compiler.wasm.sha256",
  "demo_config": "demo/deka.json",
  "consumer": "consumer/deka_demo_consumer.js"
}
EOF2

printf 'Built DekaScript demo package: %s\n' "$OUT_DIR"
