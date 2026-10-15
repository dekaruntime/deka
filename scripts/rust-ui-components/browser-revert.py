#!/usr/bin/env python3
"""Prove synchronous DOM focus cannot discard a renderer image update."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
GUARD = "    if (drawing) { redraw = true; return }\n"


def main():
    package = Path(sys.argv[1]).resolve()
    hosts = list(package.rglob("host.js"))
    source = (ROOT / "crates/deka_ui/web/host.js").read_bytes()
    assert hosts and GUARD.encode() in source, "Draw guard/package missing"
    originals = {path: path.read_bytes() for path in hosts}
    assert all(data == source for data in originals.values()), "Package host differs from source"
    command = ["node", "scripts/rust-tour-browser/check.mjs", str(package)]
    try:
        for path, data in originals.items():
            path.write_bytes(data.replace(GUARD.encode(), b""))
        broken = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=900)
        output = broken.stdout + broken.stderr
        log = ROOT / ".tmp/components-browser-revert-failure.log"
        log.parent.mkdir(exist_ok=True)
        log.write_text(output)
        assert broken.returncode != 0, "Removed draw guard unexpectedly passed"
        assert "values rendered frame at 100" in output and "scene.images: array lengths differ" in output, output
        print("PASS revert: synchronous focus loses a values-frame image without the draw guard", flush=True)
    finally:
        for path, data in originals.items():
            path.write_bytes(data)
    subprocess.run(command, cwd=ROOT, check=True, timeout=900)
    print("PASS restored: complete browser gate", flush=True)


if __name__ == "__main__":
    main()
