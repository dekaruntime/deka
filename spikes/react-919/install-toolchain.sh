#!/usr/bin/env bash
# Released artifacts only. dsc uses the repository's CI installer with a local pin.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p .cache/ci .toolchain/bin
cp ../../scripts/ci-install-dsc.sh .cache/ci/ci-install-dsc.sh
printf '0.50.0\n' > .cache/ci/dsc-version
bash .cache/ci/ci-install-dsc.sh "$PWD/.toolchain/bin/dsc"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) platform=darwin-arm64;;
  Darwin-x86_64) platform=darwin-x64;;
  Linux-x86_64) platform=linux-x64;;
  *) echo 'Unsupported release host' >&2; exit 1;;
esac
curl -fsSL https://releases.deka.gg/0.50.0/release.json -o .cache/deka-release.json
curl -fsSL "https://releases.deka.gg/0.50.0/deka-$platform" -o .cache/deka
python3 - "$platform" <<'PY'
import hashlib,json,sys
from pathlib import Path
release=json.loads(Path('.cache/deka-release.json').read_text())
assert release['version'].lstrip('v') == '0.50.0'
assert hashlib.sha256(Path('.cache/deka').read_bytes()).hexdigest() == release['binaries'][sys.argv[1]]['sha256']
PY
chmod 755 .cache/deka
mv .cache/deka .toolchain/bin/deka
.toolchain/bin/dsc --version
.toolchain/bin/deka --version
