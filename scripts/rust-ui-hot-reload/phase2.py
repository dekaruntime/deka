#!/usr/bin/env python3
"""Actual macro → on-disk save → runtime → signal/handler regression checks."""
import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "crates/deka_ui/examples/hot_reload_phase2_probe.rs"
LOGS = ROOT / "tasks/evidence/ui-hot-reload"

def text(node):
    return (node.get("text") or "") + "".join(map(text, node["children"]))

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--case", choices=["slots", "props", "fallback", "fragments", "ambiguous"], default="slots")
    args = parser.parse_args()
    original = SOURCE.read_text()
    lines = queue.Queue()
    log = []
    process = None
    LOGS.mkdir(parents=True, exist_ok=True)
    def until(prefix, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                line = lines.get(timeout=0.1)
            except queue.Empty:
                assert process.poll() is None, "probe exited: " + "\n".join(log[-20:])
                continue
            if line.startswith(prefix):
                return line[len(prefix):]
        raise AssertionError(f"timed out waiting for {prefix}: " + "\n".join(log[-20:]))
    def tree():
        return json.loads(until("TREE "))
    def command(action):
        process.stdin.write(action + "\n")
        process.stdin.flush()
        return tree()
    try:
        process = subprocess.Popen(["cargo", "run", "--locked", "-p", "deka-ui", "--no-default-features", "--features", "hot-reload", "--example", "hot_reload_phase2_probe"], cwd=ROOT, env=os.environ, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        def read():
            for line in process.stdout:
                log.append(line.rstrip())
                lines.put(line.rstrip())
        reader = threading.Thread(target=read, daemon=True)
        reader.start()
        until("READY ", 300)
        initial = tree()
        assert text(initial) == "markerrow0row1Old: 7 step 2Static|true|x|1.5|1Other|true|x|1.5|1Opaque: 3", text(initial)
        if args.case == "ambiguous":
            edited = original.replace('message="Static"', 'message="Placeholder"').replace('message="Other"', 'message="Static"').replace('message="Placeholder"', 'message="Other"').replace('"marker"', '"Must not patch"')
            SOURCE.write_text(edited)
            status = until("STATUS ")
            assert "RestartRequired" in status and "distinct component ids" in status, status
            assert tree() == initial, "ambiguous component reorder allowed a partial patch"
            print("ambiguous calls: distinct-id diagnostic and complete last good tree retained", flush=True)
            return
        if args.case == "fragments":
            edited = original.replace('<div id="fragment-left">{opaque}</div><div id="fragment-right"/>', '<div id="fragment-left"/><div id="fragment-right">{opaque}</div>').replace('"marker"', '"Must not patch"')
            assert edited != original
            SOURCE.write_text(edited)
            status = until("STATUS ")
            assert "RestartRequired" in status and "opaque fragment" in status, status
            assert tree() == initial, "unmovable fragment allowed a partial patch"
            print("opaque fragment: explicit restart before any supported markup patches", flush=True)
            return
        if args.case == "fallback":
            SOURCE.write_text(original.replace('title="Old" step=2 seed=0', 'title="Must not patch" step=2 seed=9'))
            status = until("STATUS ")
            assert "RestartRequired" in status and "seed" in status, status
            assert tree() == initial, "compiled prop logic allowed a partial patch"
            print("compiled prop logic: explicit restart; complete last good tree retained", flush=True)
            return
        if args.case == "props":
            SOURCE.write_text(original.replace('title="Old" step=2 seed=0', 'title="New" step=5 seed=0').replace("message=\"Static\" enabled=true letter='x' ratio=1.5f32 tiny=1u8", "message=\"Updated\" enabled=false letter='z' ratio=2.5f32 tiny=9u8"))
            patched = tree()
            assert "New: 7 step 5" in text(patched), text(patched)
            assert "Updated|false|z|2.5|9" in text(patched), text(patched)
            clicked = command("counter")
            assert "New: 12 step 5" in text(clicked), text(clicked)
            SOURCE.write_text(original.replace('title="Old" step=2 seed=0', 'title="Must not patch" step=2147483648 seed=0'))
            status = until("STATUS ")
            assert "RestartRequired" in status and "out of range" in status, status
            assert tree() == clicked, "out-of-range typed prop partially patched"
            print("literal props: String/static str/bool/char/f32/u8/i32 patched; retained counter 7 and original handler advanced by the patched step to 12", flush=True)
            return
        # Reorder a live list and an empty conditional; reparent the component.
        edited = original.replace('<div id="left">{conditional}<span id="marker">"marker"</span>{list}<Counter id="counter" title="Old" step=2 seed=0/></div><div id="right"/>', '<div id="left">{list}<span id="marker">"marker"</span></div><div id="right"><Counter id="counter" title="Old" step=2 seed=0/>{conditional}</div>')
        assert edited != original
        SOURCE.write_text(edited)
        patched = tree()
        assert text(patched["children"][0]) == "row0row1marker", text(patched)
        assert text(patched["children"][1]) == "Old: 7 step 2", text(patched)
        initial_rows = {node["text"]: node["id"] for node in initial["children"][0]["children"] if node["text"] in ("row0", "row1")}
        moved_rows = {node["text"]: node["id"] for node in patched["children"][0]["children"] if node["text"] in ("row0", "row1")}
        assert moved_rows == initial_rows, "moving the dynamic list reconstructed its children"
        shown = command("show")
        assert text(shown["children"][1]) == "Old: 7 step 2Conditional: 3", text(shown)
        grown = command("rows")
        assert text(grown["children"][0]) == "row0row1row2marker", text(grown)
        clicked = command("click")
        assert text(clicked["children"][1]) == "Old: 7 step 2Conditional: 4", text(clicked)
        hidden = command("hide")
        assert text(hidden["children"][1]) == "Old: 7 step 2", text(hidden)
        # Move the now-empty slot ahead of the live component and list again.
        edited2 = edited.replace('<Counter id="counter" title="Old" step=2 seed=0/>{conditional}', '{conditional}<Counter id="counter" title="Old" step=2 seed=0/>').replace('{list}<span id="marker">"marker"</span>', '<span id="marker">"marker"</span>{list}')
        SOURCE.write_text(edited2)
        tree()
        shown2 = command("show")
        assert text(shown2["children"][1]) == "Conditional: 4Old: 7 step 2", text(shown2)
        assert text(shown2["children"][0]) == "markerrow0row1row2", text(shown2)
        clicked2 = command("counter")
        assert "Old: 9 step 2" in text(clicked2), text(clicked2)
        # An unsupported new slot in the same save must block all static edits.
        SOURCE.write_text(edited2.replace('"marker"', '"Must not patch"').replace('{list}', '{list}{move || View::text("new compiled child")}'))
        status = until("STATUS ")
        stale = tree()
        assert "RestartRequired" in status and "rebuilding and restarting" in status, status
        assert stale == clicked2, "fallback partially patched the UI"
        print("slots: empty/nonempty reordering, list updates, component reparenting, state/handlers and atomic restart verified", flush=True)
    finally:
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            (LOGS / f"phase2-{args.case}.log").write_text("\n".join(log))
        SOURCE.write_text(original)

if __name__ == "__main__":
    main()
