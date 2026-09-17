#!/usr/bin/env bash
# CI gate for deka#1154 / dekaruntime/rfd#68's 2026-09-17 amendment to rfd#59:
# deka's scripts/dsc-version and scripts/testsuite-corpus-version pins must
# carry deka's own crate version, or the immediately previous released
# version while a bump is mid-flight. Reads the real Cargo.toml, git tag
# history and pin files, then delegates the actual decision to
# scripts/lockstep-check.sh (also covered by tests/lockstep-check.sh's
# hermetic fixtures). No cargo build.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/lockstep-check.sh"

DSC_VERSION_PATH="scripts/dsc-version"
CORPUS_VERSION_PATH="scripts/testsuite-corpus-version"

current_version() {
  local file="$ROOT/Cargo.toml"
  local line
  line="$(grep -m1 -E '^version[[:space:]]*=' "$file" || true)"
  if [[ -z "$line" ]]; then
    echo "no version line found in $file" >&2
    exit 1
  fi
  # line looks like: version = "0.53.7"
  local value="${line#*=}"
  value="${value//\"/}"
  value="$(printf '%s' "$value" | tr -d '[:space:]')"
  if ! lockstep_is_semver "$value"; then
    echo "malformed version \"$value\" in $file" >&2
    exit 1
  fi
  echo "$value"
}

# Highest stable "vX.Y.Z" tag (canary tags excluded) strictly below
# $current, via `git tag --list 'v*' --sort=-v:refname`. Empty if none.
previous_released_version() {
  local current="$1"
  local tag base
  while IFS= read -r tag; do
    [[ -z "$tag" ]] && continue
    base="${tag#v}"
    lockstep_is_semver "$base" || continue  # skip canary / malformed tags
    if lockstep_version_lt "$base" "$current"; then
      echo "$base"
      return 0
    fi
  done < <(git -C "$ROOT" tag --list 'v*' --sort=-v:refname)
  echo ""
}

main() {
  local current previous dsc_pin corpus_pin failed=0

  current="$(current_version)"
  previous="$(previous_released_version "$current")"

  dsc_pin="$(head -n1 "$ROOT/$DSC_VERSION_PATH")"
  corpus_pin="$(head -n1 "$ROOT/$CORPUS_VERSION_PATH")"

  lockstep_check_pin "$DSC_VERSION_PATH" "" "$dsc_pin" "$current" "$previous" || failed=1
  lockstep_check_pin "$CORPUS_VERSION_PATH" "corpus-v" "$corpus_pin" "$current" "$previous" || failed=1

  if [[ "$failed" -ne 0 ]]; then
    echo "lockstep check failed (dekaruntime/rfd#68 amendment to rfd#59)" >&2
    return 1
  fi

  echo "lockstep ok: deka $current; pins carry $current${previous:+ or the previous set version $previous}"
}

main "$@"
