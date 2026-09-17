#!/usr/bin/env bash
# Hermetic tests for the typecheck ratchet decision function.
# No network, no cargo, no CLI — just fixture result files.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/typeck-ratchet.sh"

# The tests exercise the decision function with their own fixture list, so
# they stay valid whatever the live expected-failure list contains.
EXPECTED_FAILURES=("auth@0.4.1" "http@0.4.1" "jwt@0.4.1")

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

# Build a fixture results file from triples: name version status
fixture() {
  local f="$WORKDIR/$1.tsv"
  shift
  : >"$f"
  while [[ $# -ge 3 ]]; do
    printf '%s\t%s\t%s\n' "$1" "$2" "$3" >>"$f"
    shift 3
  done
  echo "$f"
}

run() {
  local label="$1" expected_exit="$2" fixture="$3" expected_msg="$4"
  local out="$WORKDIR/$label.out"
  local got=0

  set +e
  typeck_ratchet_evaluate "$fixture" >"$out" 2>&1
  got=$?
  set -e

  if [[ "$got" -ne "$expected_exit" ]]; then
    echo "FAIL $label: expected exit $expected_exit, got $got" >&2
    cat "$out" >&2
    return 1
  fi

  if ! grep -qF "$expected_msg" "$out"; then
    echo "FAIL $label: output did not contain '$expected_msg'" >&2
    cat "$out" >&2
    return 1
  fi

  echo "PASS $label"
}

run "listed-pass-fails" 1 \
  "$(fixture listed_pass auth 0.4.1 ok)" \
  "listed package now passes"

run "unlisted-fail-fails" 1 \
  "$(fixture unlisted_fail bytes 0.2.0 fail)" \
  "unlisted failure"

run "three-listed-failures-pass" 0 \
  "$(fixture three_listed \
    auth 0.4.1 fail \
    http 0.4.1 fail \
    jwt 0.4.1 fail)" \
  "expected failure"

run "newer-version-fails-normally" 1 \
  "$(fixture newer_version auth 0.4.2 fail)" \
  "unlisted failure"

run "mixed-ok-and-expected" 0 \
  "$(fixture mixed \
    auth 0.4.1 fail \
    http 0.4.1 fail \
    jwt 0.4.1 fail \
    bytes 0.2.0 ok)" \
  "all published stdlib packages typecheck"

echo "all ratchet tests passed"
