#!/usr/bin/env bash
# File-size gate (deka#391).
#
# CLAUDE.md's threshold is ">500 smells, >1000 is wrong, >2000 is a fire to
# put out", but nothing measured it, so framework.rs grew 97 -> 3,200 lines
# across eight individually-reasonable PRs. This script is the measurement.
#
# It runs on the tree being checked — in CI that is the PR *merge result*
# (actions/checkout's default for pull_request), which is the only version
# that catches a file grown a few hundred lines at a time across a stack.
#
# Rules for every crates/**.rs file:
#   - not in the baseline and over 1,000 lines  -> FAIL (new crosser)
#   - in the baseline and longer than recorded  -> FAIL (grew)
#   - baseline entry gone or back under 1,000   -> FAIL (remove the stale
#     line from the baseline; the list only ever shrinks — it is the
#     tracked burn-down for the existing offenders)
#
# Keep it POSIX-ish bash (macOS still ships bash 3.2): no associative arrays.
set -euo pipefail

cd "$(dirname "$0")/.."
BASELINE=scripts/file-size-baseline.txt
LIMIT=1000
fail=0

baseline_lines_for() {
    # $1 = path; prints recorded line count or nothing
    grep -F "$1 " "$BASELINE" 2>/dev/null | head -1 | awk '{print $2}' || true
}

while IFS= read -r file; do
    lines=$(wc -l < "$file" | tr -d ' ')
    [ "$lines" -le "$LIMIT" ] && continue
    base=$(baseline_lines_for "$file")
    if [ -z "$base" ]; then
        echo "FAIL: $file is $lines lines (limit $LIMIT). Split it instead of growing it."
        fail=1
    elif [ "$lines" -gt "$base" ]; then
        echo "FAIL: $file grew from $base to $lines lines (limit $LIMIT)."
        fail=1
    fi
done < <(find crates -name '*.rs' -type f | sort)

if [ -f "$BASELINE" ]; then
    while read -r path lines; do
        case "$path" in ''|\#*) continue ;; esac
        if [ ! -f "$path" ]; then
            echo "FAIL: baseline entry $path no longer exists; remove it from $BASELINE."
            fail=1
        elif [ "$(wc -l < "$path" | tr -d ' ')" -le "$LIMIT" ]; then
            echo "FAIL: $path is back under $LIMIT lines; remove it from $BASELINE (burn-down)."
            fail=1
        fi
    done < "$BASELINE"
fi

if [ "$fail" -ne 0 ]; then
    echo
    echo "See $BASELINE for the tracked burn-down list (deka#391)."
    exit 1
fi
echo "file-size gate: ok (limit $LIMIT; baseline: $(grep -cve '^\s*$' -e '^\s*#' "$BASELINE" 2>/dev/null || echo 0) grandfathered files)"
