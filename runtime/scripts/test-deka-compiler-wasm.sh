#!/usr/bin/env bash
set -euo pipefail

runtime_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out_dir=${DEKA_WASM_OUT_DIR:-"$runtime_dir/dist/deka-compiler-wasm"}

cd "$runtime_dir"
cargo test --locked --release -p phpx_compiler_wasm
"$runtime_dir/scripts/build-deka-compiler-wasm.sh" "$out_dir"
bun "$runtime_dir/crates/phpx_compiler_wasm/tests/browser-parity.mjs" "$out_dir/deka_compiler.wasm"
bun "$runtime_dir/crates/phpx_lsp/tests/browser-diagnostics.mjs" "$out_dir/deka_diagnostics.wasm"
