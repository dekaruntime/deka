#!/bin/bash
# Recreate vendor/gpui-0.2.2: the GPUI release deka shipped (checksum from main's
# Cargo.lock) plus vendor/gpui-readback.diff, the hook `cmp-gpui snap` uses to
# read a presented frame back. With the hook unused it behaves like the release.
set -euo pipefail
cd "$(dirname "$0")/vendor"
if [ ! -d gpui-0.2.2 ]; then
  curl -fsSL https://crates.io/api/v1/crates/gpui/0.2.2/download -o gpui-0.2.2.crate
  echo "979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707  gpui-0.2.2.crate" | shasum -a 256 -c -
  tar -xzf gpui-0.2.2.crate
  rm gpui-0.2.2.crate
  (cd gpui-0.2.2 && patch -p0 < ../gpui-readback.diff)
fi
echo "vendor/gpui-0.2.2 ready"
