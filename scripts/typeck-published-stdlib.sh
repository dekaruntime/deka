#!/usr/bin/env bash
# Typecheck each published @deka/* tarball against this checkout's CLI.
# Catches stdlib that compiled on an old compiler and broke on current main
# (dekaruntime/deka#405).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLI="${CLI:-$ROOT/target/release/cli}"
if [[ ! -x "$CLI" ]]; then
  CLI="$ROOT/target/debug/cli"
fi
if [[ ! -x "$CLI" ]]; then
  echo "error: no deka CLI at $CLI; build -p cli first" >&2
  exit 1
fi

REGISTRY="${REGISTRY_URL:-https://deka.gg/api/registry}"
# Live index package names. Probe each; skip if the registry 404s.
PACKAGES=(
  auth bytes cookies crypto fs http io json jwt tcp time tls
)

failed=0
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

for name in "${PACKAGES[@]}"; do
  meta="$(curl -fsS "$REGISTRY/$name.json" || true)"
  if [[ -z "$meta" ]]; then
    echo "skip $name (no registry json)"
    continue
  fi
  echo "=== $name ==="
  proj="$workdir/$name"
  mkdir -p "$proj"
  printf '%s\n' "{\"name\":\"typeck-$name\",\"version\":\"0.0.0\",\"security\":{\"prompt\":false}}" >"$proj/deka.json"
  printf '%s\n' '{"lockfileVersion":1,"packages":{}}' >"$proj/deka.lock"
  if ! (cd "$proj" && DEKA_SECURITY_NO_PROMPT=1 "$CLI" add "$name" --yes --no-prompt); then
    echo "FAIL $name: deka add"
    failed=1
    continue
  fi
  main=""
  for candidate in \
    "$proj/ds_modules/@deka/$name/index.ds" \
    "$proj/ds_modules/$name/index.ds" \
    "$proj/php_modules/@deka/$name/index.ds" \
    "$proj/php_modules/$name/index.ds"
  do
    if [[ -f "$candidate" ]]; then
      main="$candidate"
      break
    fi
  done
  if [[ -z "$main" ]]; then
    echo "FAIL $name: no index.ds after install"
    failed=1
    continue
  fi
  if ! (cd "$proj" && DEKA_SECURITY_NO_PROMPT=1 "$CLI" check "$main"); then
    echo "FAIL $name: typeck"
    failed=1
    continue
  fi
  echo "ok $name"
done

if [[ "$failed" -ne 0 ]]; then
  echo "published stdlib typeck failed" >&2
  exit 1
fi
echo "all published stdlib packages typeck"
