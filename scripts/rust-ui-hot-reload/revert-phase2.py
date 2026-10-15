#!/usr/bin/env python3
"""Literal deletions of each feature's integration; sources restored in finally."""
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
LOGS = ROOT / "tasks/evidence/ui-hot-reload"
LOGS.mkdir(parents=True, exist_ok=True)

def run(name, command, expected):
    with (LOGS / (name + ".log")).open("w") as log:
        result = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
    output = (LOGS / (name + ".log")).read_text()
    assert (result.returncode == 0) == expected, f"{name}: unexpected exit {result.returncode}: {output[-3000:]}"
    if not expected:
        assert ("AssertionError" in output or "TimeoutError" in output) and "could not compile" not in output, f"{name}: not a behavioral failure: {output[-3000:]}"
    print(f"{name}: exit={result.returncode} ({'restored PASS' if expected else 'literal revert FAIL'})", flush=True)

def proof(name, path, mutate, command):
    path = ROOT / path
    original = path.read_text()
    try:
        reverted = mutate(original)
        assert reverted != original, f"{name}: mutation did not remove the integration"
        path.write_text(reverted)
        run("phase2-revert-" + name, command, False)
    finally:
        path.write_text(original)
    run("phase2-restored-" + name, command, True)

proof("props", "crates/deka_ui/src/hot_reload.rs",
      lambda source: re.sub(r"for update in plan\.props \{\s*update\(\);\s*\}", "let _ = plan.props;", source),
      ["python3", "scripts/rust-ui-hot-reload/phase2.py", "--case", "props"])
proof("slots", "crates/deka_native_ir/src/tree/template.rs",
      lambda source: source.replace("        parent.0.borrow_mut().children = untouched;", "        let _ = untouched;"),
      ["python3", "scripts/rust-ui-hot-reload/phase2.py", "--case", "slots"])
proof("ambiguous", "crates/deka_ui_hot_reload/src/lib.rs",
      lambda source: source[:source.index("        // A permutation of literal values")] + source[source.index("        let mut result = BTreeMap::new();"):],
      ["python3", "scripts/rust-ui-hot-reload/phase2.py", "--case", "ambiguous"])
proof("browser", "crates/deka_ui/web/hot-host.js",
      lambda source: source.replace("const result = JSON.parse(app.hot_reload(event.data))", "const result = { unchanged: true }"),
      ["node", "scripts/rust-ui-hot-reload/browser.mjs"])
