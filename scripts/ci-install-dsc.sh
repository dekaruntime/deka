#!/usr/bin/env bash
# Download the published dsc CLI (and optionally the browser wasm) for CI.
#
# This is not the user installer. People run https://deka.gg/install.sh, which
# writes both deka and dsc into ~/.deka/bin. CI needs the same dsc bits without
# installing deka over the tree it is about to test.
#
#   scripts/ci-install-dsc.sh /path/to/dsc
#   scripts/ci-install-dsc.sh /path/to/dsc --wasm /path/to/wasm-dir
#
# Artifacts come from https://dsc-wasm.deka.gg (public). Checksums come from
# that host's latest/release.json. A missing file or a mismatch is a hard fail.
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 DEST_DSC [--wasm WASM_DIR]" >&2
  exit 2
fi

DEST=$1
shift
WASM_DIR=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --wasm)
      WASM_DIR="${2:-}"
      [[ -n "$WASM_DIR" ]] || { echo "fatal: --wasm needs a directory" >&2; exit 2; }
      shift 2
      ;;
    *)
      echo "fatal: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

BASE="${DSC_BASE_URL:-https://dsc-wasm.deka.gg}"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64|Darwin-aarch64) PLATFORM=darwin-arm64 ;;
  Darwin-x86_64)               PLATFORM=darwin-x64 ;;
  Linux-x86_64)                PLATFORM=linux-x64 ;;
  *)
    echo "fatal: unsupported host $(uname -sm); dsc publishes macOS arm64/x64 and Linux x64" >&2
    exit 1
    ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "${BASE}/latest/release.json" -o "${tmp}/release.json"
version=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "${tmp}/release.json")
expected=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["binaries"][sys.argv[2]]["sha256"])' "${tmp}/release.json" "$PLATFORM")
[[ -n "$version" && -n "$expected" ]] || {
  echo "fatal: could not read dsc ${PLATFORM} from ${BASE}/latest/release.json" >&2
  exit 1
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

mkdir -p "$(dirname "$DEST")"
curl -fsSL "${BASE}/v${version}/dsc-${PLATFORM}" -o "${tmp}/dsc"
actual=$(sha256_file "${tmp}/dsc")
if [[ "$expected" != "$actual" ]]; then
  echo "fatal: dsc checksum mismatch for ${PLATFORM} v${version}" >&2
  echo "  expected ${expected}" >&2
  echo "  actual   ${actual}" >&2
  exit 1
fi
chmod 755 "${tmp}/dsc"
mv -f "${tmp}/dsc" "$DEST"
echo "installed dsc v${version} -> $DEST"

if [[ -n "$WASM_DIR" ]]; then
  mkdir -p "$WASM_DIR"
  wasm_sha=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["wasm"]["compiler_sha256"])' "${tmp}/release.json")
  curl -fsSL "${BASE}/v${version}/deka_compiler.wasm" -o "${tmp}/deka_compiler.wasm"
  actual=$(sha256_file "${tmp}/deka_compiler.wasm")
  if [[ "$wasm_sha" != "$actual" ]]; then
    echo "fatal: dsc wasm checksum mismatch for v${version}" >&2
    echo "  expected ${wasm_sha}" >&2
    echo "  actual   ${actual}" >&2
    exit 1
  fi
  mv -f "${tmp}/deka_compiler.wasm" "${WASM_DIR}/deka_compiler.wasm"
  echo "installed dsc wasm v${version} -> ${WASM_DIR}/deka_compiler.wasm"
fi
