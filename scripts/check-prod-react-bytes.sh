#!/usr/bin/env bash
# Prove the CLI users actually download does not leak frozen dev-React
# source into what it ships to end users — and does correctly carry what
# `deka dev` needs to serve.
#
# History (deka#982, a fast-follow of #980, both off tracker #969): before
# #980 this script built the exact shipped default-feature binary and
# scanned its raw bytes, asserting NONE of the frozen dev-React vendor
# source (crates/http/vendor/react/) appeared anywhere in it. #980 shipped
# the dev server in default builds, and in the same change repointed this
# script at a native-only variant (--no-default-features --features
# native) that nobody downloads — the script kept passing, but silently
# stopped covering the artifact it was named for.
#
# Re-pointing this script at the default binary is NOT a matter of
# swapping which `cargo build` it runs. The pre-#980 "zero dev-React bytes
# anywhere in the binary" assertion is no longer TRUE of the default
# binary, by design: crates/http/src/react_refresh/vendor.rs
# `include_str!`s the frozen dev-React source directly, and that module is
# compiled in whenever deka_http's `dev-server` feature is enabled — which
# the default feature set now is (deka#980). The default binary needs
# those bytes on disk to serve `deka dev`'s Fast Refresh at
# `/_deka/react/*`. Scanning the default binary for zero dev-React bytes
# would therefore be asserting a permanent, by-design "failure" — not
# guarding against a regression.
#
# So this script checks each build for what is actually true of it:
#
#   1. NATIVE-ONLY (--no-default-features --features native): dev-server
#      is fully absent, so the pre-#980 invariant still holds exactly —
#      this binary must contain ZERO bytes of the frozen dev-React vendor
#      source anywhere. Also: --help must still mention serve/dev (the
#      commands exist, just unavailable), and `dev` / `serve --dev` must
#      fail with the missing-feature message.
#
#   2. DEFAULT (shipped) feature set: the opposite byte check — this
#      binary MUST contain the frozen dev-React vendor source (its
#      absence would mean `deka dev`'s Fast Refresh silently lost its
#      assets, a real regression a byte scan can catch without spinning
#      up a server). --help must mention serve/dev. It must NOT be
#      asserted to fail running `dev` — it has a working dev server.
#
# The guard for "does dev-React leak into what a merchant actually
# ships" now lives where the shippable artifact is produced: the
# PRODUCTION BUNDLE `deka build --bundle` writes, not the CLI binary
# itself. crates/cli/tests/react_builtin.rs's
# build_bundle_inlines_prod_react_without_dev_bytes builds that bundle
# with the default-feature CLI and byte-scans the emitted FILE (not just
# the in-memory decoded string) for the same vendor chunks this script
# uses — see that test for the byte-level leak guard.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
mkdir -p "$CARGO_TARGET_DIR/qa-937/tmp"
export TMPDIR="$CARGO_TARGET_DIR/qa-937/tmp"

echo "== building native-only release CLI (no dev-server) =="
cargo build --locked --release -p cli --no-default-features --features native
NATIVE_BIN="$CARGO_TARGET_DIR/release/cli-native-only"
cp "$CARGO_TARGET_DIR/release/cli" "$NATIVE_BIN"

echo "== building default-feature release CLI (the shipped artifact) =="
cargo build --locked --release -p cli
DEFAULT_BIN="$CARGO_TARGET_DIR/release/cli"

python3 - "$DEFAULT_BIN" "$NATIVE_BIN" <<'PY'
from pathlib import Path
import re
import subprocess
import sys

default_bin = Path(sys.argv[1])
native_bin = Path(sys.argv[2])
vendor = Path('crates/http/vendor/react/react-dom-client.js').read_bytes()
# Check many disjoint, distinctive chunks of the actual frozen vendor source,
# not a filename that can also legitimately appear in an import map.
chunks = [vendor[i:i + 128] for i in range(0, len(vendor) - 128, 4096)]
assert chunks, 'no vendor chunks derived; vendor file too small or missing'


def contains_any_chunk(data: bytes) -> bool:
    return any(chunk in data for chunk in chunks)


# --- native-only binary: dev-server is fully absent; must contain NONE of
# the frozen dev-React vendor source anywhere. This is the pre-#980
# invariant, still true here because this build never compiles
# crates/http/src/react_refresh (feature-gated on deka_http's dev-server). ---
native_data = native_bin.read_bytes()
assert not contains_any_chunk(native_data), \
    f'dev React found in native-only CLI ({native_bin}) — it must never embed it'

# --- default (shipped) binary: dev-server IS present, so this binary MUST
# embed the frozen dev-React vendor source — that is what `deka dev`'s Fast
# Refresh serves at /_deka/react/*. Its absence would be a real regression
# (broken dev server) that a byte scan can catch cheaply, without starting
# one. This is intentionally the mirror image of the native-only check. ---
default_data = default_bin.read_bytes()
assert contains_any_chunk(default_data), (
    f'shipped CLI ({default_bin}) is missing the frozen dev-React vendor bytes it '
    'needs to serve `deka dev` Fast Refresh — crates/http/src/react_refresh/vendor.rs '
    'may have stopped embedding crates/http/vendor/react/react-dom-client.js, or this '
    'binary was built without the dev-server feature it should default to (deka#980)'
)

# --- help text sanity: both binaries list serve/dev; only the native-only
# binary is expected to FAIL running them. ---
# Match on rendered words, not a line's first whitespace-split token: deka#977/#978
# moved command-specific flags (like --dev) out of the global flags dump and
# under their owning command's own --help, which is the correct place for them
# to live. A word-boundary search on the full text survives that kind of
# reflow; re-deriving a new positional/column assumption would just break
# again on the next help edit.
default_help = subprocess.check_output([str(default_bin), '--help'], text=True, stderr=subprocess.STDOUT)
assert re.search(r'\bserve\b', default_help), 'shipped CLI must retain serve in --help'
assert re.search(r'\bdev\b', default_help), 'shipped CLI must retain dev in --help'

native_help = subprocess.check_output([str(native_bin), '--help'], text=True, stderr=subprocess.STDOUT)
assert re.search(r'\bserve\b', native_help), 'native-only CLI must retain serve in --help'
assert re.search(r'\bdev\b', native_help), 'native-only CLI must retain dev in --help'

native_serve_help = subprocess.check_output([str(native_bin), 'serve', '--help'], text=True, stderr=subprocess.STDOUT)
assert re.search(r'--dev\b', native_serve_help), \
    'native-only CLI must retain --dev help (now documented under `deka serve --help`, its owning command)'

for args in (['dev'], ['serve', '--dev']):
    result = subprocess.run([str(native_bin), *args], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    assert result.returncode != 0, f'{args} must fail without dev-server on the native-only build'
    assert 'this build lacks the dev server; rebuild with --features dev-server' in result.stdout, result.stdout

print(
    f'PASS: native-only CLI ({len(native_data)} bytes) contains none of {len(chunks)} '
    f'dev-React vendor chunks; shipped default-feature CLI ({len(default_data)} bytes) '
    'correctly embeds them for `deka dev`; both retain serve/dev in --help; native-only '
    'CLI reports the missing dev-server feature on execution. The production-bundle '
    'leak guard (what a merchant actually ships) lives in '
    'crates/cli/tests/react_builtin.rs::build_bundle_inlines_prod_react_without_dev_bytes.'
)
PY
