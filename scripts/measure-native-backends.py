#!/usr/bin/env python3
"""Measure stripped native benchmark copies on macOS, never an existing PID.

Usage: python3 scripts/measure-native-backends.py .tmp/quickjs-comparison
The directory must contain backend-rust, backend-v8, and backend-quickjs.
"""
import datetime
import gzip
import hashlib
import json
import pathlib
import platform
import re
import statistics
import subprocess
import sys
import time


def footprint_mib(output):
    match = re.search(r"^Physical footprint:\s*([\d.]+)([KMG])", output, re.M)
    if not match:
        raise RuntimeError("vmmap did not report physical footprint")
    return float(match[1]) * {"K": 1 / 1024, "M": 1, "G": 1024}[match[2]]


def main():
    if sys.platform != "darwin" or len(sys.argv) != 2:
        raise SystemExit(__doc__)
    root = pathlib.Path(sys.argv[1]).resolve(strict=True)
    names = ["rust", "v8", "quickjs"]
    report = {
        "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "host": platform.platform(),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "working_diff_sha256": hashlib.sha256(subprocess.check_output(["git", "diff", "HEAD"])).hexdigest(),
        "idle_seconds": 5,
        "variants": {},
    }
    for name in names:
        binary = root / f"backend-{name}"
        data = binary.read_bytes()
        exercise = subprocess.check_output([str(binary), "--exercise", "3"], text=True).strip()
        if exercise != "Deka native backend comparison Count: 3":
            raise RuntimeError(f"{name}: unexpected exercise result: {exercise}")
        report["variants"][name] = {
            "bytes": len(data), "gzip_bytes": len(gzip.compress(data, mtime=0)),
            "sha256": hashlib.sha256(data).hexdigest(), "exercise": exercise, "samples": [],
        }
    for iteration in range(3):
        # Rotate first/last position to reduce ordering bias.
        for name in names[iteration:] + names[:iteration]:
            with (root / f"{name}-{iteration}.log").open("w") as log:
                process = subprocess.Popen([str(root / f"backend-{name}")], stdout=log, stderr=log)
                try:
                    time.sleep(5)
                    if process.poll() is not None:
                        raise RuntimeError(f"{name} exited before measurement")
                    rss = int(subprocess.check_output(["ps", "-o", "rss=", "-p", str(process.pid)], text=True))
                    vmmap = subprocess.check_output(["vmmap", "-summary", str(process.pid)], text=True, stderr=subprocess.STDOUT)
                    (root / f"vmmap-{name}-{iteration}.txt").write_text(vmmap)
                    sample = {"pid": process.pid, "rss_kib": rss, "physical_mib": footprint_mib(vmmap)}
                    report["variants"][name]["samples"].append(sample)
                    print(name, iteration, json.dumps(sample), flush=True)
                finally:
                    if process.poll() is None:
                        process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
    for value in report["variants"].values():
        value["median_physical_mib"] = statistics.median(s["physical_mib"] for s in value["samples"])
        value["median_rss_kib"] = statistics.median(s["rss_kib"] for s in value["samples"])
    (root / "measurements.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
