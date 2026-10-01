#!/usr/bin/env python3
"""Build the versioned browser package consumed by the website and R2 release."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
BINDGEN = "0.2.128"


def run(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    if run("git", "status", "--porcelain", "--untracked-files=no"):
        raise SystemExit("Build the release package from a clean committed source tree")
    out.mkdir(parents=True, exist_ok=True)
    if run("wasm-bindgen", "--version") != f"wasm-bindgen {BINDGEN}":
        raise SystemExit(f"Install wasm-bindgen-cli {BINDGEN}")
    target = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target"))).resolve()
    subprocess.run(["cargo", "build", "--locked", "--release", "--target",
                    "wasm32-unknown-unknown", "-p", "deka_native_web"], cwd=ROOT, check=True)
    subprocess.run(["wasm-bindgen", str(target / "wasm32-unknown-unknown/release/deka_native_web.wasm"),
                    "--target", "web", "--omit-default-module-path", "--out-dir", str(out)], check=True)
    shutil.copyfile(ROOT / "crates/deka_native_ui/assets/OFL.txt", out / "font-OFL.txt")
    names = ["deka_native_web.js", "deka_native_web.d.ts", "deka_native_web_bg.wasm", "font-OFL.txt"]
    files = {}
    for name in names:
        data = (out / name).read_bytes()
        files[name] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    commit = run("git", "rev-parse", "HEAD")
    # Version is the source's base version. Promotion changes only release.json;
    # this package and its checksums remain byte-for-byte identical.
    version = re.search(r'\[workspace.package\]\s+version = "([^"]+)"',
                        (ROOT / "Cargo.toml").read_text())[1]
    manifest = dict(schemaVersion=1, abiVersion=1, runtime="deka_vm", version=version,
                    source="https://github.com/dekaruntime/deka", commit=commit,
                    compilerCommit=commit, wasmBindgen=BINDGEN,
                    cargoLockSha256=hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(), files=files)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Built native browser package {version} ({commit}) at {out}")


if __name__ == "__main__":
    main()
