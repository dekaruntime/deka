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
    # Hot reload lives in deka_ui::hot_reload and deka_native_ir::hot_reload; neither may reach a release binary.
    assert "deka_ui10hot_reload" not in symbols and "deka_native_ir10hot_reload" not in symbols and "deka_ui_hot_reload" not in symbols
    assert b"keeping last good UI" not in data and b"compiled template location missing" not in data
    return loaded_bytes(sections), len(data), hashlib.sha256(data).hexdigest()

def loaded_bytes(sections):
    """Code and data bytes, leaving out unwind and exception tables.

    deka_native_ir::hot_reload shares object files with the rest of the crate, so on Linux the
    linker keeps the exception tables of its dead functions after dropping their code (about
    200 KB of .gcc_except_table). That is linker metadata, not code; the symbol and payload
    checks above prove none of the module itself is linked.
    """
    total = 0
    for line in sections.splitlines():
        parts = line.replace(":", " ").split()
        if sys.platform == "darwin":
            if len(parts) < 3 or parts[0] != "Section":
                continue
            name, size = parts[1], parts[2]
        else:
            if len(parts) < 2 or not parts[0].startswith("."):
                continue
            name, size = parts[0], parts[1]
        if any(word in name for word in ("except", "eh_frame", "unwind")):
            continue
        total += int(size)
    return total

plain = build(False)
hot = build(True)
# Enabling the feature may not add code or data to a release build. The symbol and payload checks
# above are the real guard; this one catches the parser or watcher being linked, which would add
# hundreds of KB. SLACK covers inlining shifts: hot_reload calls into deka_native_ir, so LLVM
# inlines some of that crate's functions differently before the dead callers are dropped
# (+172 bytes of WASM on macOS, +907 on Linux, 15 Oct). Smaller is always fine.
SLACK = 2048
assert hot[0]-plain[0] <= SLACK, (plain, hot)
print(f"release code+data: disabled={plain[0]} bytes; enabled={hot[0]} bytes; diff={hot[0]-plain[0]} bytes")
print(f"release files: disabled={plain[1]} bytes; enabled={hot[1]} bytes")
print(f"release hashes: disabled={plain[2]} enabled={hot[2]}")
print("release: no hot-reload symbols, watcher diagnostics, or compiled source payload; both consumers execute Value: 1")

def wasm_build(feature):
    command = ["cargo", "build", "--locked", "--profile", "native", "--target", "wasm32-unknown-unknown", "-p", "deka-ui-tour", "--example", "hot_reload_web"]
    if feature:
        command += ["--features", "deka-ui/hot-reload"]
    subprocess.run(command, cwd=ROOT, check=True)
    out = ROOT / ".tmp" / ("hot-reload-wasm-enabled" if feature else "hot-reload-wasm-disabled")
    subprocess.run(["wasm-bindgen", str(TARGET / "wasm32-unknown-unknown/native/examples/hot_reload_web.wasm"), "--target", "web", "--out-name", "app", "--out-dir", str(out)], cwd=ROOT, check=True)
    data = (out / "app_bg.wasm").read_bytes()
    for sentinel in [b"DEKA_HOT_RELOAD_RELEASE_PAYLOAD_SENTINEL_1440", b"keeping last good UI", b"browserapp_hot_reload", b"crates/deka_ui/src/hot_reload.rs", b"__deka/events", b"__deka/rebuild"]:
        assert sentinel not in data, f"release WASM contains development payload: {sentinel!r}"
    assert not list(out.rglob("hot-host.js")), "release imports the development transport"
    javascript = (out / "app.js").read_text()
    assert "hot_reload" not in javascript and "hot-host" not in javascript
    return len(data), hashlib.sha256(data).hexdigest()

wasm_plain = wasm_build(False)
wasm_hot = wasm_build(True)
assert wasm_hot[0]-wasm_plain[0] <= SLACK, (wasm_plain, wasm_hot)  # one-sided, as above
print(f"release WASM: disabled={wasm_plain[0]} bytes; enabled={wasm_hot[0]} bytes; diff={wasm_hot[0]-wasm_plain[0]} bytes; no development exports, source payload or transport")
