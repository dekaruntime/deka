#!/usr/bin/env bash
# Expected-failure ratchet for scripts/typeck-published-stdlib.sh.
# Interim until dekaruntime/dsc#272 (typed bridge calls) ships and deka pins it.
# See dekaruntime/deka#1115.
#
# This file is sourced by both the typecheck gate and its hermetic tests.
# It contains no network or cargo dependencies.

# Expected failures are keyed on EXACT name@version. Newer versions of the
# same package are checked normally and must pass.
#
# Removal condition: delete an entry once deka pins a dsc that includes
# dekaruntime/dsc#272 (typed bridge calls), because that dsc will typecheck the
# listed package@version without any per-call ceremony.
EXPECTED_FAILURES=(
  "auth@0.4.1" # typed bridge calls (dsc#272)
  "http@0.4.1" # typed bridge calls (dsc#272)
  "jwt@0.4.1"  # typed bridge calls (dsc#272)
)

typeck_ratchet_reason() {
  echo "typed bridge calls (dekaruntime/dsc#272); delete this entry when deka pins a dsc that includes dsc#272"
}

# Return 0 if $1 is an exact match in EXPECTED_FAILURES.
typeck_ratchet_is_expected() {
  local key="$1"
  local ef
  for ef in ${EXPECTED_FAILURES[@]+"${EXPECTED_FAILURES[@]}"}; do
    if [[ "$ef" == "$key" ]]; then
      return 0
    fi
  done
  return 1
}

# Evaluate per-package typecheck results.
# Input file: tab-separated lines "name<TAB>version<TAB>status".
# status must be "ok" or "fail".
#
# Prints one line per package and a summary. Returns 0 if the gate passes,
# 1 if it fails. Failure modes:
#   - a failing package that is NOT in EXPECTED_FAILURES (unlisted failure)
#   - a passing package that IS in EXPECTED_FAILURES (listed package now passes)
typeck_ratchet_evaluate() {
  local results_file="$1"
  local gate_failed=0
  local name version status key

  while IFS=$'\t' read -r name version status _rest; do
    [[ -z "$name" ]] && continue
    key="${name}@${version}"

    if [[ "$status" == "fail" ]]; then
      if typeck_ratchet_is_expected "$key"; then
        echo "expected failure: $key ($(typeck_ratchet_reason))"
      else
        echo "FAIL $key: unlisted failure (add to expected-failure list only if intentional)"
        gate_failed=1
      fi
    else
      if typeck_ratchet_is_expected "$key"; then
        echo "FAIL $key: listed package now passes; delete its entry from the expected-failure list"
        gate_failed=1
      else
        echo "ok $key"
      fi
    fi
  done < "$results_file"

  if [[ "$gate_failed" -ne 0 ]]; then
    echo "published stdlib typeck ratchet failed" >&2
    return 1
  fi

  echo "all published stdlib packages typecheck"
  return 0
}
