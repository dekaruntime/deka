#!/usr/bin/env bash
# Lockstep pin decision logic — rfd#68's 2026-09-17 amendment to rfd#59: deka,
# dsc, testsuite and tour share ONE version, on both channels. deka's own
# crate version (Cargo.toml) is the source of truth, and its
# scripts/dsc-version / scripts/testsuite-corpus-version pins must each carry
# that version — or the immediately previous released version, while a bump
# is mid-flight (deka and dsc cannot both publish the new number in the same
# instant). Any other value fails.
#
# This file is sourced by both the CI gate (scripts/lockstep-verify.sh, which
# reads the real Cargo.toml / git tags / pin files) and its hermetic tests
# (tests/lockstep-check.sh, which supply fixture strings). No network, no
# cargo, no CLI — see deka#1154, dekaruntime/rfd#68.

# Validate "X.Y.Z" (three non-negative integers, no extra characters).
lockstep_is_semver() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
}

# Numeric semver comparison: 0 if $1 < $2, 1 otherwise.
lockstep_version_lt() {
  local a="$1" b="$2"
  local a_major a_minor a_patch b_major b_minor b_patch
  IFS='.' read -r a_major a_minor a_patch <<<"$a"
  IFS='.' read -r b_major b_minor b_patch <<<"$b"
  if (( a_major != b_major )); then
    (( a_major < b_major ))
    return
  fi
  if (( a_minor != b_minor )); then
    (( a_minor < b_minor ))
    return
  fi
  (( a_patch < b_patch ))
}

# Parse a pin's line 1. Args: raw_line, required_prefix ("" for a bare
# version pin such as scripts/dsc-version, "corpus-v" for the corpus pin).
# On success prints the base "X.Y.Z" version and returns 0. On any shape
# mismatch (wrong/missing prefix, malformed version, empty canary sha)
# prints nothing and returns 1.
lockstep_parse_pin() {
  local raw="$1" prefix="$2"
  local trimmed rest base sha

  trimmed="$(printf '%s' "$raw" | tr -d '\r')"
  trimmed="${trimmed%"${trimmed##*[![:space:]]}"}"  # trim trailing whitespace

  if [[ -n "$prefix" ]]; then
    case "$trimmed" in
      "$prefix"*) rest="${trimmed#"$prefix"}" ;;
      *) return 1 ;;
    esac
  else
    rest="$trimmed"
  fi

  if [[ "$rest" == *-canary-* ]]; then
    base="${rest%%-canary-*}"
    sha="${rest#*-canary-}"
    [[ -z "$sha" ]] && return 1
  else
    base="$rest"
  fi

  lockstep_is_semver "$base" || return 1
  printf '%s\n' "$base"
}

# Core decision. Args:
#   label            human label for messages, e.g. "scripts/dsc-version"
#   prefix           required literal prefix on the pin line ("" or "corpus-v")
#   raw_pin          the pin file's line 1, verbatim
#   current_version  this repo's own X.Y.Z (from Cargo.toml)
#   previous_version highest released vX.Y.Z below current_version, or ""
#                    if none exists
# Prints one result line to stdout; returns 0 pass / 1 fail.
lockstep_check_pin() {
  local label="$1" prefix="$2" raw_pin="$3" current_version="$4" previous_version="$5"
  local expected base trimmed

  trimmed="$(printf '%s' "$raw_pin" | tr -d '\r')"
  trimmed="${trimmed%"${trimmed##*[![:space:]]}"}"

  if [[ -n "$previous_version" ]]; then
    expected="$current_version or $previous_version"
  else
    expected="$current_version"
  fi

  if ! base="$(lockstep_parse_pin "$raw_pin" "$prefix")"; then
    echo "FAIL $label: expected \"${prefix}X.Y.Z\" or \"${prefix}X.Y.Z-canary-<sha>\" with X.Y.Z = $expected, found \"$trimmed\""
    return 1
  fi

  if [[ "$base" == "$current_version" ]]; then
    echo "ok $label: $trimmed (current $current_version)"
    return 0
  fi

  if [[ -n "$previous_version" && "$base" == "$previous_version" ]]; then
    echo "ok $label: $trimmed (previous set version $previous_version, bump in flight)"
    return 0
  fi

  echo "FAIL $label: expected base version $expected, found $base in \"$trimmed\""
  return 1
}
