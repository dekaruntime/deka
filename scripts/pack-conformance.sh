#!/usr/bin/env bash
# Assemble dist/conformance/ from this tree + an existing hats-results.json dump.
# Dump first: see tests/dump/README.md
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo"

version=$(scripts/runtime-version.sh)
commit=$(git rev-parse HEAD)
out="${DEKA_CONFORMANCE_OUT:-$repo/dist/conformance}"
results="${DEKA_DUMP_OUT:-$out/hats-results.json}"

tour="$repo/.cache/tour"
[[ -f "$tour/manifest.json" ]] || { echo "fatal: tour missing; run scripts/ci-fetch-tour.sh" >&2; exit 2; }
testsuite="${DEKA_TESTSUITE_ROOT:-$repo/.cache/testsuite-corpus}"
[[ -d "$testsuite" ]] || { echo "fatal: testsuite corpus missing; run scripts/ci-fetch-testsuite-corpus.sh" >&2; exit 2; }
[[ -f "$results" ]] || { echo "fatal: dump missing at $results" >&2; echo "      fix: see tests/dump/README.md" >&2; exit 2; }

rm -rf "$out/tour" "$out/testsuite"
mkdir -p "$out/tour" "$out/testsuite"

cp "$tour/manifest.json" "$tour"/*.ds "$tour"/*.dsx "$out/tour/"

# Hats folders only — not the native runner or known-fail list.
find "$testsuite" -mindepth 1 -maxdepth 1 -type d ! -name '.*' | while read -r cat; do
  cp -R "$cat" "$out/testsuite/"
done

if [[ "$results" != "$out/hats-results.json" ]]; then
  cp "$results" "$out/hats-results.json"
fi

jq -n \
  --arg version "$version" \
  --arg commit "$commit" \
  --arg results "hats-results.json" \
  '{
    schemaVersion: 1,
    version: $version,
    commit: $commit,
    tour: "tour/",
    testsuite: "testsuite/",
    results: $results
  }' > "$out/manifest.json"

echo "packed $out (version=$version commit=${commit:0:9})"
find "$out" -type f | wc -l | awk '{print "files:", $1}'
