#!/usr/bin/env python3
"""Execute a release binary, build and relocate apps, and assert test exit status."""
import pathlib
import shutil
import subprocess
import sys
import tempfile

binary = pathlib.Path(sys.argv[1]).resolve()
root = pathlib.Path(sys.argv[2]).resolve()
root.mkdir(parents=True, exist_ok=True)

def run(args, cwd, good=True):
    result = subprocess.run([str(binary), *args], cwd=cwd, capture_output=True, text=True)
    if good and result.returncode != 0:
        raise AssertionError(result.stderr)
    if not good and result.returncode == 0:
        raise AssertionError("failure was reported as success")
    return result.stdout

with tempfile.TemporaryDirectory(dir=root) as temporary:
    work = pathlib.Path(temporary)
    source = work / "source"
    source.mkdir()
    destination = work / "artifacts"
    destination.mkdir()
    (source / "math.ds").write_text('export fn answer() number { return 40 + 2; }')
    (source / "hello.ds").write_text('import { echo } from "io"; import { assert } from "test"; import { answer } from "./math.ds"; assert(answer() == 42); echo("hello from native Deka");')
    assert run(["run", "hello.ds"], source) == "hello from native Deka\n"
    (source / "math.test.ds").write_text('import { assert } from "test"; import { answer } from "./math.ds"; fn test_answer() { assert(answer() == 42); }')
    assert "1 passed, 0 failed" in run(["test"], source)
    (source / "fail.test.ds").write_text('import { assert } from "test"; fn test_failure() { assert(false); }')
    assert "1 failed" in run(["test"], source, good=False)
    executable = destination / "hello"
    run(["build", "hello.ds", "--outfile", str(executable)], source)
    shutil.rmtree(source)
    result = subprocess.run([str(executable)], cwd=destination, env={}, capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
    assert result.stdout == "hello from native Deka\n", result.stdout
    if sys.platform == "darwin":
        subprocess.run(["/usr/bin/codesign", "--verify", "--strict", str(executable)], check=True)
    app = work / "app"
    app.mkdir()
    run(["init"], app)
    assert "Count:  3" in run(["run", "--exercise", "3"], app)
    app_binary = destination / "app"
    run(["build", "--outfile", str(app_binary)], app)
    shutil.rmtree(app)
    result = subprocess.run([str(app_binary), "--exercise", "4"], cwd=destination, env={}, capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
    assert "Count:  4" in result.stdout, result.stdout
print("native source, modules, app tests, relocated executable and UI clicks passed")
