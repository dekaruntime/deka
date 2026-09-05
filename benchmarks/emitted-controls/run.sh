#!/usr/bin/env bash
set -euo pipefail
umask 022

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
CASES="$ROOT/cases"
DEKA=${DEKA:-$(cd "$ROOT/../.." && pwd)/target/release/cli}
NODE=${NODE:-node}
TMPDIR=$(mktemp -d "${TMPDIR:-/tmp}/deka-592.XXXXXX")
trap 'rm -rf "$TMPDIR"' EXIT

[[ -x "$DEKA" ]] || { echo "compiler is not executable: $DEKA" >&2; exit 2; }
command -v "$NODE" >/dev/null || { echo "node is required" >&2; exit 2; }

median() {
  printf '%s\n' "$@" | sort -n | awk '{a[NR]=$1} END { if (NR%2) print a[(NR+1)/2]; else print (a[NR/2]+a[NR/2+1])/2 }'
}

measure() {
  local file=$1 out real
  local -a values=()
  for _ in 1 2 3 4 5; do
    set +e
    out=$(/usr/bin/time -p "$NODE" "$file" 2>&1)
    status=$?
    set -e
    [[ $status -eq 0 ]] || { printf '%s\n' "$out" >&2; return "$status"; }
    real=$(printf '%s\n' "$out" | awk '$1 == "real" { print $2; exit }')
    [[ -n "$real" ]] || { echo "missing timing for $file" >&2; return 2; }
    values+=("$real")
  done
  median "${values[@]}"
}

printf '%-28s %-12s %-12s %-12s\n' ITEM GENERATED_S CONTROL_S GAP
printf '%-28s %-12s %-12s %-12s\n' ---------------------------- ------------ ------------ ------------

for control in "$CASES"/*.js; do
  item=$(basename "$control" .js)
  source="$CASES/$item.ds"
  generated="$TMPDIR/$item.generated.js"
  [[ -f "$source" ]] || { echo "missing DekaScript source: $source" >&2; exit 2; }

  "$DEKA" transpile "$source" --out "$generated" >/dev/null
  generated_s=$(measure "$generated")
  control_s=$(measure "$control")
  gap=$(awk -v g="$generated_s" -v c="$control_s" 'BEGIN { if (c == 0) print "n/a"; else printf "%.2fx", g/c }')
  printf '%-28s %-12s %-12s %-12s\n' "$item" "$generated_s" "$control_s" "$gap"
done

echo
echo "Machine: $(uname -srm), $(sysctl -n hw.model 2>/dev/null || true)"
echo "Runtime: $($NODE --version); compiler: $DEKA"
echo "Source revision: $(git -C "$ROOT/../.." rev-parse --short HEAD 2>/dev/null || echo unknown)"
echo "Timing: 5 /usr/bin/time real samples per side; median shown; compile excluded"
