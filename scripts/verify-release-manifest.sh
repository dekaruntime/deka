#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

MANIFEST="${ROOT_DIR}/target/release/deka-manifest.json"
CLI_BIN="${ROOT_DIR}/target/release/cli"

if [[ ! -f "${MANIFEST}" ]]; then
  echo "missing manifest: ${MANIFEST}" >&2
  exit 1
fi
if [[ ! -f "${CLI_BIN}" ]]; then
  echo "missing artifact: ${CLI_BIN}" >&2
  exit 1
fi
manifest_value() {
  local key="$1"
  sed -n "s/.*\"${key}\": \"\([^\"]*\)\".*/\1/p" "${MANIFEST}" | head -n1
}

manifest_cli_sha="$(awk '/"cli"/{f=1} f && /"sha256"/{gsub(/[", ]/,"",$2); print $2; exit}' "${MANIFEST}")"
manifest_runtime_abi="$(manifest_value runtime_abi)"
manifest_git_sha="$(manifest_value git_sha)"

cli_sha="$(shasum -a 256 "${CLI_BIN}" | awk '{print $1}')"

if [[ "${cli_sha}" != "${manifest_cli_sha}" ]]; then
  echo "cli sha mismatch: manifest=${manifest_cli_sha} actual=${cli_sha}" >&2
  exit 1
fi
cli_meta="$(${CLI_BIN} --version --verbose 2>&1)"
runtime_abi="$(printf '%s\n' "${cli_meta}" | awk -F': ' '/^runtime_abi:/ { print $2 }')"
git_sha="$(printf '%s\n' "${cli_meta}" | awk -F': ' '/^git_sha:/ { print $2 }')"

if [[ -n "${manifest_runtime_abi}" && "${runtime_abi}" != "${manifest_runtime_abi}" ]]; then
  echo "runtime abi mismatch: manifest=${manifest_runtime_abi} actual=${runtime_abi}" >&2
  exit 1
fi
if [[ -n "${manifest_git_sha}" && "${git_sha}" != "${manifest_git_sha}" ]]; then
  echo "git sha mismatch: manifest=${manifest_git_sha} actual=${git_sha}" >&2
  exit 1
fi

echo "artifact manifest verification ok"
