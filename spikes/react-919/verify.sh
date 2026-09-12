#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
export PATH="$PWD/.toolchain/bin:$PATH"
export NODE_ENV=production
mkdir -p .cache
[[ "$(dsc --version)" == *0.50.0* ]]
[[ "$(deka --version)" == *0.50.0* ]]
deka install --locked
dsc check Component.dsx
dsc transpile Component.dsx --out .cache/Component.mjs
cmp .cache/Component.mjs evidence/Component.mjs
dsc check probes/react-wrapper.ds
for probe in jsx function react-at-js react-relative react-default; do
  if dsc check "probes/$probe.ds" > ".cache/$probe.txt" 2>&1; then
    echo "Expected 0.50.0 finding disappeared: $probe" >&2
    exit 1
  fi
done
node run.mjs ssr
node run.mjs ink
