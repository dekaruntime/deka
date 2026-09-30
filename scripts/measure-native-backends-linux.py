#!/usr/bin/env python3
"""Measure the native counter variants on headless Linux using private Xvfb.

Usage: python3 scripts/measure-native-backends-linux.py .tmp/linux-comparison
The directory must contain stripped backend-rust, backend-v8, backend-quickjs.
Requires Xvfb, xwininfo, and Mesa's lavapipe Vulkan ICD. Starts only its own
processes, disables X TCP listening, and always stops its children.
"""
import datetime
import gzip
import hashlib
import json
import os
import pathlib
import re
import select
import statistics
import subprocess
import sys
import time


def stop(process):
    if process.poll() is None:
        process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def main():
    if sys.platform != "linux" or len(sys.argv) != 2:
        raise SystemExit(__doc__)
    root = pathlib.Path(sys.argv[1]).resolve(strict=True)
    driver = pathlib.Path("/usr/share/vulkan/icd.d/lvp_icd.json")
    if not driver.is_file():
        raise SystemExit("Mesa lavapipe ICD is required for this controlled headless run")
    names = ["rust", "v8", "quickjs"]
    report = {
        "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "host": pathlib.Path("/etc/os-release").read_text(),
        "kernel": os.uname().release,
        "machine": os.uname().machine,
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "measurement_script_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
        "display": "Xvfb 1024x768x24; Mesa lavapipe software Vulkan",
        "driver_icd": json.loads(driver.read_text()),
        "idle_seconds": 5,
        "variants": {},
    }
    for name in names:
        binary = root / f"backend-{name}"
        data = binary.read_bytes()
        exercise = subprocess.check_output([str(binary), "--exercise", "3"], text=True, timeout=30).strip()
        if exercise != "Deka native backend comparison Count: 3":
            raise RuntimeError(f"{name}: unexpected exercise result: {exercise}")
        report["variants"][name] = {
            "bytes": len(data), "gzip_bytes": len(gzip.compress(data, mtime=0)),
            "sha256": hashlib.sha256(data).hexdigest(), "exercise": exercise, "samples": [],
        }
    # Xvfb chooses a free display; no shared user desktop or guessed PID is used.
    read_fd, write_fd = os.pipe()
    with (root / "xvfb.log").open("w") as log:
        server = subprocess.Popen(
            ["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "1024x768x24", "-nolisten", "tcp"],
            pass_fds=(write_fd,), stdout=log, stderr=log,
        )
        os.close(write_fd)
        try:
            if not select.select([read_fd], [], [], 10)[0]:
                raise RuntimeError("Xvfb did not become ready")
            display = os.read(read_fd, 32).decode().strip()
            if not display.isdigit():
                raise RuntimeError("Xvfb did not provide a display number")
            runtime_dir = root / "xdg-runtime"
            runtime_dir.mkdir(mode=0o700, exist_ok=True)
            env = dict(os.environ, DISPLAY=f":{display}", XDG_RUNTIME_DIR=str(runtime_dir),
                       VK_DRIVER_FILES=str(driver), VK_ICD_FILENAMES=str(driver))
            env.pop("WAYLAND_DISPLAY", None)
            for iteration in range(3):
                for name in names[iteration:] + names[:iteration]:
                    with (root / f"{name}-{iteration}.log").open("w") as app_log:
                        process = subprocess.Popen([str(root / f"backend-{name}")], env=env, stdout=app_log, stderr=app_log)
                        try:
                            time.sleep(5)
                            if process.poll() is not None:
                                raise RuntimeError(f"{name} exited before measurement; inspect its log")
                            windows = subprocess.check_output(["xwininfo", "-root", "-tree"], env=env, text=True, timeout=10)
                            (root / f"windows-{name}-{iteration}.txt").write_text(windows)
                            window = re.search(r'(0x[0-9a-fA-F]+) "Deka native experiment"', windows)
                            if window is None:
                                raise RuntimeError(f"{name} has no native experiment window")
                            info = subprocess.check_output(["xwininfo", "-id", window[1]], env=env, text=True, timeout=10)
                            (root / f"window-{name}-{iteration}.txt").write_text(info)
                            if "Map State: IsViewable" not in info:
                                raise RuntimeError(f"{name} window is not mapped")
                            mappings = pathlib.Path(f"/proc/{process.pid}/smaps").read_text()
                            (root / f"mappings-{name}-{iteration}.txt").write_text(mappings)
                            driver_library = pathlib.Path(report["driver_icd"]["ICD"]["library_path"]).name
                            if driver_library not in mappings:
                                raise RuntimeError(f"{name} did not load the selected Vulkan driver")
                            executable = {"Size": 0, "Rss": 0}
                            selected = False
                            for line in mappings.splitlines():
                                if re.match(r"^[0-9a-f]+-[0-9a-f]+\s", line):
                                    selected = line.endswith(str(root / f"backend-{name}"))
                                elif selected:
                                    field = re.match(r"^(Size|Rss):\s+(\d+) kB$", line)
                                    if field:
                                        executable[field[1]] += int(field[2])
                            if executable["Size"] == 0:
                                raise RuntimeError(f"{name} executable mappings were not found")
                            smaps = pathlib.Path(f"/proc/{process.pid}/smaps_rollup").read_text()
                            (root / f"smaps-{name}-{iteration}.txt").write_text(smaps)
                            values = {key: int(value) for key, value in re.findall(r"^(\w+):\s+(\d+) kB$", smaps, re.M)}
                            sample = {
                                "pid": process.pid, "rss_kib": values["Rss"], "pss_kib": values["Pss"],
                                "uss_kib": values["Private_Clean"] + values["Private_Dirty"] + values.get("Private_Hugetlb", 0), "swap_kib": values["Swap"],
                                "executable_mapping_size_kib": executable["Size"],
                                "executable_mapping_rss_kib": executable["Rss"],
                            }
                            report["variants"][name]["samples"].append(sample)
                            print(name, iteration, json.dumps(sample), flush=True)
                        finally:
                            stop(process)
        finally:
            os.close(read_fd)
            stop(server)
    for value in report["variants"].values():
        for metric in ["rss_kib", "pss_kib", "uss_kib", "swap_kib", "executable_mapping_size_kib", "executable_mapping_rss_kib"]:
            value[f"median_{metric}"] = statistics.median(s[metric] for s in value["samples"])
    (root / "measurements.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
