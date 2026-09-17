#!/usr/bin/env bash
# Hermetic tests for the lockstep pin decision function (deka#1154,
# dekaruntime/rfd#68's 2026-09-17 amendment to rfd#59).
# No network, no cargo, no CLI, no git — just fixture strings.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/scripts/lockstep-check.sh"

run() {
  local label="$1" expected_exit="$2" expected_msg="$3"
  shift 3
  local out got=0

  set +e
  out="$(lockstep_check_pin "$@" 2>&1)"
  got=$?
  set -e

  if [[ "$got" -ne "$expected_exit" ]]; then
    echo "FAIL $label: expected exit $expected_exit, got $got" >&2
    echo "$out" >&2
    return 1
  fi

  if [[ -n "$expected_msg" ]] && ! grep -qF "$expected_msg" <<<"$out"; then
    echo "FAIL $label: output did not contain '$expected_msg'" >&2
    echo "$out" >&2
    return 1
  fi

  echo "PASS $label"
}

# --- pin equal to the version passes ---
run "dsc-pin-at-current-passes" 0 "ok scripts/dsc-version" \
  "scripts/dsc-version" "" "0.53.7" "0.53.7" "0.53.6"

run "dsc-pin-at-current-with-canary-passes" 0 "ok scripts/dsc-version" \
  "scripts/dsc-version" "" "0.53.7-canary-f99a004" "0.53.7" "0.53.6"

run "corpus-pin-at-current-passes" 0 "ok scripts/testsuite-corpus-version" \
  "scripts/testsuite-corpus-version" "corpus-v" "corpus-v0.53.7" "0.53.7" "0.53.6"

# --- pin lagging by one set version passes ---
run "dsc-pin-lagging-by-one-passes" 0 "previous set version" \
  "scripts/dsc-version" "" "0.53.6-canary-5d18cc5" "0.53.7" "0.53.6"

run "corpus-pin-lagging-by-one-passes" 0 "previous set version" \
  "scripts/testsuite-corpus-version" "corpus-v" "corpus-v0.53.6" "0.53.7" "0.53.6"

# --- two-version gap fails ---
run "corpus-pin-two-versions-behind-fails" 1 "expected base version 0.53.7 or 0.53.6, found 0.53.4" \
  "scripts/testsuite-corpus-version" "corpus-v" "corpus-v0.53.4" "0.53.7" "0.53.6"

run "dsc-pin-ahead-of-current-fails" 1 "expected base version 0.53.6 or 0.53.5, found 0.53.7" \
  "scripts/dsc-version" "" "0.53.7-canary-b5228e0" "0.53.6" "0.53.5"

# --- mismatched corpus tag fails ---
run "corpus-pin-missing-prefix-fails" 1 "expected \"corpus-vX.Y.Z\"" \
  "scripts/testsuite-corpus-version" "corpus-v" "v0.53.7" "0.53.7" "0.53.6"

run "corpus-pin-bare-version-fails" 1 "expected \"corpus-vX.Y.Z\"" \
  "scripts/testsuite-corpus-version" "corpus-v" "0.53.7" "0.53.7" "0.53.6"

# --- other malformed shapes fail ---
run "empty-canary-sha-fails" 1 "expected \"" \
  "scripts/dsc-version" "" "0.53.7-canary-" "0.53.7" "0.53.6"

run "malformed-version-fails" 1 "expected \"" \
  "scripts/dsc-version" "" "0.53" "0.53.7" "0.53.6"

run "no-previous-version-names-current-only" 1 "expected base version 0.53.7," \
  "scripts/dsc-version" "" "0.40.0" "0.53.7" ""

# Confirm the current-real-state numbers from deka main today: the dsc pin
# (base 0.53.6, one behind 0.53.7) passes; the corpus pin (base 0.53.4, two
# behind) fails. This is deka#1154's own motivating fixture.
run "real-state-dsc-pin-passes" 0 "previous set version" \
  "scripts/dsc-version" "" "0.53.6-canary-5d18cc5" "0.53.7" "0.53.6"

run "real-state-corpus-pin-fails" 1 "expected base version 0.53.7 or 0.53.6, found 0.53.4" \
  "scripts/testsuite-corpus-version" "corpus-v" "corpus-v0.53.4" "0.53.7" "0.53.6"

echo "all lockstep tests passed"
