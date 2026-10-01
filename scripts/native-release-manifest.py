#!/usr/bin/env python3
"""Create the native release contract from the exact artifacts about to ship."""
import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
version, commit, published = sys.argv[2:5]
base = version.split("-canary-")[0]

def artifact(path):
    data = (root / path).read_bytes()
    return {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}

manifest = {
    "version": version, "base_version": base, "tag": "v" + version,
    "commit": commit, "published_at": published, "channel": "canary",
    "runtime": "deka_vm", "cli_abi": 1, "promoted_from": None,
    "binaries": {p: {"name": "deka-" + p, **artifact("deka-" + p)} for p in ["linux-x64", "darwin-x64", "darwin-arm64"]},
    "native_ui": {"manifest": "native-ui/manifest.json", "abi": 1, **artifact("native-ui/manifest.json")},
}
web = json.loads((root / "native-ui/manifest.json").read_text())
assert web["schemaVersion"] == 1 and web["abiVersion"] == 1 and web["runtime"] == "deka_vm"
assert set(web["files"]) == {"deka_native_web.js", "deka_native_web.d.ts", "deka_native_web_bg.wasm", "font-OFL.txt"}
for name, expected in web["files"].items():
    assert artifact("native-ui/" + name) == expected, "native browser bytes differ from their manifest"
assert web["version"] == base, "native browser version differs from CLI"
assert web["commit"] == commit, "native browser source differs from CLI"
(root / "release.json").write_text(json.dumps(manifest, indent=2) + "\n")
(root / "canary.json").write_text(json.dumps(manifest, indent=2) + "\n")
