#!/usr/bin/env bash
# deka#799 stress harness: run deka_http lib tests repeatedly with varied
# parallelism until the process aborts (SIGABRT / exit 134) or the iteration
# budget is exhausted. Exits 0 on reproduction, 1 if never reproduced.
set -u

ITERATIONS="${ITERATIONS:-200}"
THREADS_LIST="${THREADS_LIST:-1 2 4 8 16 32 64}"

BIN=$(ls -t target/release/deps/deka_http-* 2>/dev/null | grep -v '\.' | head -1)
if [ -z "$BIN" ]; then
  echo "test binary not found; build first" >&2
  exit 2
fi
echo "test binary: $BIN"

fail_log=target/deka799-stress-failures.log
: > "$fail_log"

i=0
while [ "$i" -lt "$ITERATIONS" ]; do
  for t in $THREADS_LIST; do
    i=$((i + 1))
    out=$("$BIN" --test-threads="$t" 2>&1)
    rc=$?
    if [ $rc -ne 0 ]; then
      {
        echo "=== FAILURE iter=$i threads=$t rc=$rc date=$(date -u +%FT%TZ) ==="
        echo "$out" | tail -30
      } | tee -a "$fail_log"
      echo "REPRODUCED at iteration $i (threads=$t, rc=$rc); see $fail_log"
      exit 0
    fi
    if [ $((i % 20)) -eq 0 ]; then
      echo "iter $i/$ITERATIONS (threads=$t) clean, rc=0"
    fi
  done
done

echo "NOT reproduced after $i iterations"
exit 1
