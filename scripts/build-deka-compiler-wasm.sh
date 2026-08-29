#!/usr/bin/env bash
set -euo pipefail

runtime_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out_dir=${1:-"$runtime_dir/dist/deka-compiler-wasm"}
# The caller may pass a relative path (e.g. in CI). Normalize it now so the
# subsequent `cd "$runtime_dir"` does not change where the artifacts land.
case "$out_dir" in
  /*) ;;
  *) out_dir="$runtime_dir/$out_dir" ;;
esac
artifact_name=deka_compiler.wasm
diagnostics_artifact_name=deka_diagnostics.wasm
target_dir=${CARGO_TARGET_DIR:-"$runtime_dir/target"}
# The dirty-tree check is useful in local/test contexts to ensure the artifact
# matches a clean commit. In release CI the tree is clean by construction, but
# actions/checkout and other setup steps can leave the index in a state that
# this check rejects; allow skipping it via env var.
if [ "${DEKA_SKIP_DIRTY_CHECK:-}" != "1" ]; then
  git -C "$runtime_dir" diff --quiet
  git -C "$runtime_dir" diff --cached --quiet
fi
source_commit=$(git -C "$runtime_dir" rev-parse HEAD)
cargo_lock_sha256=$(shasum -a 256 "$runtime_dir/Cargo.lock" | awk '{print $1}')
rustc_version=$(rustc -Vv | tr '\n' ';' | sed 's/;$/\n/')
deka_version=$("$runtime_dir/scripts/runtime-version.sh")

mkdir -p "$out_dir"
cd "$runtime_dir"
CARGO_INCREMENTAL=0 DEKA_SOURCE_COMMIT="$source_commit" \
  cargo build --release \
  --target wasm32-unknown-unknown -p deka_compiler_wasm --no-default-features
CARGO_INCREMENTAL=0 DEKA_SOURCE_COMMIT="$source_commit" \
  cargo build --release \
  --target wasm32-unknown-unknown -p dekascript_lsp_wasm --no-default-features

source_artifact="$target_dir/wasm32-unknown-unknown/release/deka_compiler_wasm.wasm"
test -f "$source_artifact"
cp "$source_artifact" "$out_dir/$artifact_name"
artifact_sha256=$(shasum -a 256 "$out_dir/$artifact_name" | awk '{print $1}')
diagnostics_source_artifact="$target_dir/wasm32-unknown-unknown/release/dekascript_lsp_wasm.wasm"
test -f "$diagnostics_source_artifact"
cp "$diagnostics_source_artifact" "$out_dir/$diagnostics_artifact_name"
diagnostics_artifact_sha256=$(shasum -a 256 "$out_dir/$diagnostics_artifact_name" | awk '{print $1}')

cat > "$out_dir/$artifact_name.metadata.json" <<EOF
{
  "schema_version": 1,
  "artifact": "$artifact_name",
  "sha256": "$artifact_sha256",
  "source_commit": "$source_commit",
  "compiler": {"name": "deka", "version": "$deka_version", "abi_version": 2},
  "target": "wasm32-unknown-unknown",
  "cargo_lock_sha256": "$cargo_lock_sha256",
  "rustc": "$rustc_version",
  "build_command": "cargo build --locked --release --target wasm32-unknown-unknown -p deka_compiler_wasm"
}
EOF
printf '%s  %s\n' "$artifact_sha256" "$artifact_name" > "$out_dir/$artifact_name.sha256"

cat > "$out_dir/$diagnostics_artifact_name.metadata.json" <<EOF
{
  "schema_version": 1,
  "artifact": "$diagnostics_artifact_name",
  "sha256": "$diagnostics_artifact_sha256",
  "source_commit": "$source_commit",
  "adapter": {"name": "deka_diagnostics", "abi_version": 1},
  "target": "wasm32-unknown-unknown",
  "cargo_lock_sha256": "$cargo_lock_sha256",
  "rustc": "$rustc_version",
  "build_command": "cargo build --locked --release --target wasm32-unknown-unknown -p dekascript_lsp_wasm --no-default-features"
}
EOF
printf '%s  %s\n' "$diagnostics_artifact_sha256" "$diagnostics_artifact_name" > "$out_dir/$diagnostics_artifact_name.sha256"

printf 'Built %s\nMetadata: %s\nBuilt %s\nMetadata: %s\n' \
  "$out_dir/$artifact_name" "$out_dir/$artifact_name.metadata.json" \
  "$out_dir/$diagnostics_artifact_name" "$out_dir/$diagnostics_artifact_name.metadata.json"
