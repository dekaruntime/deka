#!/usr/bin/env bash
# Populate the shared Cloudflare R2 sccache buckets from a local machine.
#
# Run this on each build host (demon/rio for Linux, iMac for macOS) before
# pushing release tags. It executes the same cargo invocations the Release
# workflow uses, so compiled artifacts are cached in R2 and GitHub-hosted or
# self-hosted runners get cache hits instead of rebuilding from scratch.
#
# Required environment:
#   R2_ACCESS_KEY_ID
#   R2_SECRET_ACCESS_KEY
#
# Example:
#   R2_ACCESS_KEY_ID=xxx R2_SECRET_ACCESS_KEY=yyy runtime/scripts/populate-sccache.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNTIME_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$RUNTIME_DIR/.." && pwd)"

if [[ -z "${R2_ACCESS_KEY_ID:-}" || -z "${R2_SECRET_ACCESS_KEY:-}" ]]; then
  echo "Error: R2_ACCESS_KEY_ID and R2_SECRET_ACCESS_KEY must be set." >&2
  exit 1
fi

# Map host platform to the same bucket names the Release workflow uses.
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$OS-$ARCH" in
  linux-x86_64)
    SCCACHE_BUCKET="deka-sccache-linux-x64"
    ;;
  darwin-x86_64)
    SCCACHE_BUCKET="deka-sccache-darwin-x64"
    ;;
  darwin-arm64)
    SCCACHE_BUCKET="deka-sccache-darwin-arm64"
    ;;
  *)
    echo "Error: unsupported platform $OS-$ARCH" >&2
    exit 1
    ;;
esac

export SCCACHE_ENDPOINT="https://08b93c93f8b7adc43678e9afba31e4ee.r2.cloudflarestorage.com"
export SCCACHE_REGION="auto"
export SCCACHE_BUCKET
export RUSTC_WRAPPER="sccache"
export CARGO_INCREMENTAL=0
export DEKA_SKIP_DIRTY_CHECK=1

# Ensure sccache is installed and show the bucket it will write to.
if ! command -v sccache >/dev/null 2>&1; then
  echo "Error: sccache is not installed. Install it first (e.g. cargo install sccache)." >&2
  exit 1
fi

echo "Populating sccache bucket: $SCCACHE_BUCKET"
sccache --show-stats

cd "$RUNTIME_DIR"

echo "=== Building tested crates ==="
cargo build -p deka_http -p pool -p engine -p deka_js -p php-rs -p bundler

echo "=== Running tests (also warms cache) ==="
cargo test -p deka_http
cargo test -p pool
cargo test -p engine
cargo test -p deka_js
cargo test -p php-rs
cargo test -p bundler

echo "=== Building CLI ==="
cargo build --release -p cli

echo "=== Building browser WASM ==="
"$SCRIPT_DIR/build-deka-compiler-wasm.sh" "$RUNTIME_DIR/dist/deka-compiler-wasm"

echo "=== Final sccache stats ==="
sccache --show-stats

echo "Done. Bucket $SCCACHE_BUCKET should now contain cached artifacts for $OS-$ARCH."
