#!/usr/bin/env bash
# Check the optional native-only CLI, not the shipped default CLI.
# Shipped defaults include the dev server; this binary-level guard deliberately
# opts out. Emitted production JavaScript is covered by the build tests.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
mkdir -p "$CARGO_TARGET_DIR/qa-937/tmp"
export TMPDIR="$CARGO_TARGET_DIR/qa-937/tmp"
cargo build --locked --release -p cli --no-default-features --features native
python3 - "$CARGO_TARGET_DIR/release/cli" <<'PY'
from pathlib import Path
import re
import subprocess
import sys
binary = Path(sys.argv[1])
data = binary.read_bytes()
vendor = Path('crates/http/vendor/react/react-dom-client.js').read_bytes()
# Check many disjoint, distinctive chunks of the actual frozen vendor source,
# not a filename that can also legitimately appear in an import map.
chunks = [vendor[i:i + 128] for i in range(0, len(vendor) - 128, 4096)]
assert chunks and not any(chunk in data for chunk in chunks), 'dev React found in native-only CLI'

help_text = subprocess.check_output([str(binary), '--help'], text=True, stderr=subprocess.STDOUT)
# Match on rendered words, not a line's first whitespace-split token: deka#977/#978
# moved command-specific flags (like --dev) out of the global flags dump and
# under their owning command's own --help, which is the correct place for them
# to live. A word-boundary search on the full text survives that kind of
# reflow; re-deriving a new positional/column assumption would just break
# again on the next help edit.
assert re.search(r'\bserve\b', help_text), 'native-only CLI must retain serve in --help'
assert re.search(r'\bdev\b', help_text), 'native-only CLI must retain dev in --help'

serve_help = subprocess.check_output([str(binary), 'serve', '--help'], text=True, stderr=subprocess.STDOUT)
assert re.search(r'--dev\b', serve_help), \
    'native-only CLI must retain --dev help (now documented under `deka serve --help`, its owning command)'

for args in (['dev'], ['serve', '--dev']):
    result = subprocess.run([str(binary), *args], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    assert result.returncode != 0, f'{args} must fail without dev-server'
    assert 'this build lacks the dev server; rebuild with --features dev-server' in result.stdout, result.stdout
print(f'PASS: native-only release CLI ({len(data)} bytes) contains none of {len(chunks)} react-dom-client source chunks; serve and dev help are available; dev execution reports the missing feature')
PY
