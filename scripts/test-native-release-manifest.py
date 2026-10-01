#!/usr/bin/env python3
"""Exercise release manifest production against real browser package bytes."""
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile

package = pathlib.Path(sys.argv[1]).resolve()
manifest = json.loads((package / "manifest.json").read_text())
producer = pathlib.Path(__file__).with_name("native-release-manifest.py").resolve()
with tempfile.TemporaryDirectory(dir=package.parent) as temporary:
    root = pathlib.Path(temporary)
    shutil.copytree(package, root / "native-ui")
    for platform in ["linux-x64", "darwin-x64", "darwin-arm64"]:
        (root / ("deka-" + platform)).write_bytes(b"manifest fixture; actual executables are separately smoke-tested")
    def invoke(commit=manifest["commit"]):
        return subprocess.run([sys.executable, str(producer), str(root), manifest["version"] + "-canary-" + commit[:7], commit, "2026-09-30T00:00:00Z"], capture_output=True)
    result = invoke()
    assert result.returncode == 0, result.stderr
    release = json.loads((root / "release.json").read_text())
    assert release["runtime"] == "deka_vm" and release["cli_abi"] == 1
    assert "dsc_version" not in release and "wasm" not in release
    wasm = root / "native-ui/deka_native_web_bg.wasm"
    original = wasm.read_bytes()
    wasm.write_bytes(original + b"corrupt")
    assert invoke().returncode != 0, "corrupt browser bytes were accepted"
    wasm.write_bytes(original)
    assert invoke("0" * 40).returncode != 0, "mismatched source was accepted"
    data = dict(manifest)
    data["abiVersion"] = 999
    (root / "native-ui/manifest.json").write_text(json.dumps(data))
    assert invoke().returncode != 0, "unsupported ABI was accepted"
print("Native release contract accepts matched bytes and rejects corruption/source/ABI mismatch")
