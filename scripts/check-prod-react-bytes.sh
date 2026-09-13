#!/usr/bin/env bash
# Build the production CLI separately: workspace feature unification enables dev.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
mkdir -p "$CARGO_TARGET_DIR/qa-937/tmp"
export TMPDIR="$CARGO_TARGET_DIR/qa-937/tmp"
cargo build --locked --release -p cli
python3 - "$CARGO_TARGET_DIR/release/cli" <<'PY'
from pathlib import Path
import subprocess
import sys
binary = Path(sys.argv[1])
data = binary.read_bytes()
vendor = Path('crates/http/vendor/react/react-dom-client.js').read_bytes()
# Check many disjoint, distinctive chunks of the actual frozen vendor source,
# not a filename that can also legitimately appear in an import map.
chunks = [vendor[i:i + 128] for i in range(0, len(vendor) - 128, 4096)]
assert chunks and not any(chunk in data for chunk in chunks), 'dev React found in production CLI'
help_text = subprocess.check_output([str(binary), '--help'], text=True, stderr=subprocess.STDOUT)
commands = [line.split()[0] for line in help_text.splitlines() if line.split()]
assert 'serve' in commands, 'production CLI must retain serve'
assert 'dev' in commands, 'production CLI must retain dev help'
assert '--dev' in commands, 'production CLI must retain --dev help'
for args in (['dev'], ['serve', '--dev']):
    result = subprocess.run([str(binary), *args], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    assert result.returncode != 0, f'{args} must fail without dev-server'
    assert 'this build lacks the dev server; rebuild with --features dev-server' in result.stdout, result.stdout
print(f'PASS: production release CLI ({len(data)} bytes) contains none of {len(chunks)} react-dom-client source chunks; serve and dev help are available; dev execution reports the missing feature')
PY
