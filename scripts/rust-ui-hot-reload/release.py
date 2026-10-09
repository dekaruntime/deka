#!/usr/bin/env python3
"""Build the same headless consumer with/without the feature and inspect bytes."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
ROOT = Path(__file__).resolve().parents[2]
TARGET = Path(json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT, text=True))["target_directory"])

def build(feature):
    command = ["cargo", "build", "--locked", "--release", "-p", "deka-ui", "--no-default-features", "--example", "hot_reload_size"]
    if feature:
        command += ["--features", "hot-reload"]
    subprocess.run(command, cwd=ROOT, check=True)
    binary = TARGET / "release/examples/hot_reload_size"
    result = subprocess.run([binary], check=True, capture_output=True, text=True)
    assert result.stdout.strip() == "Value: 1", result.stdout
    data = binary.read_bytes()
    (ROOT / ".tmp" / ("hot-reload-size-enabled" if feature else "hot-reload-size-disabled")).write_bytes(data)
    sections=subprocess.run(["size", "-m" if sys.platform=="darwin" else "-A", binary],check=True,capture_output=True,text=True).stdout
    print(("enabled" if feature else "disabled")+" sections:\n"+sections)
    assert b"DEKA_HOT_RELOAD_RELEASE_PAYLOAD_SENTINEL_1440" not in data, "release contains compiled source payload"
    symbols = subprocess.run(["nm", "-a", binary], check=True, capture_output=True, text=True).stdout
    assert "deka_ui10hot_reload" not in symbols and "deka_ui_hot_reload" not in symbols
    assert b"keeping last good UI" not in data and b"compiled template location missing" not in data
    return len(data), hashlib.sha256(data).hexdigest()

plain = build(False)
hot = build(True)
assert abs(hot[0]-plain[0]) <= 128, (plain, hot)
print(f"release: disabled={plain[0]} bytes; enabled={hot[0]} bytes; diff={hot[0]-plain[0]} bytes")
print(f"release hashes: disabled={plain[1]} enabled={hot[1]}")
print("release: no hot-reload symbols, watcher diagnostics, or compiled source payload; both consumers execute Value: 1")
