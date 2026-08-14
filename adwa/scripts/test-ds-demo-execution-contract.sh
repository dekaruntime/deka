#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="$(cd "$ROOT_DIR/.." && pwd)"
OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

DEKA_WASM_OUT_DIR="$OUT_DIR" "$REPO_DIR/runtime/scripts/build-deka-compiler-wasm.sh" "$OUT_DIR"
node "$ROOT_DIR/tests/deka_demo_execution_contract.mjs" "$OUT_DIR/deka_compiler.wasm"
