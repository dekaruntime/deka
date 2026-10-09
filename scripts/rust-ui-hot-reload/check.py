#!/usr/bin/env python3
"""Real cargo-run + cargo-deka disk-edit checks. No display or native window."""
import json
import argparse
import os
from pathlib import Path
import queue
import signal
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "crates/deka_ui/examples/tour/counter.rs"
TARGET = Path(json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT, text=True))["target_directory"])


class Probe:
    def __init__(self, command):
        self.lines = queue.Queue()
        self.log = []
        self.pids = set()
        self.process = subprocess.Popen(command, cwd=ROOT, env={**os.environ, "PATH": str(TARGET / "debug") + os.pathsep + os.environ["PATH"]}, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        def read():
            for line in self.process.stdout:
                self.log.append(line.rstrip())
                self.lines.put(line.rstrip())
        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()

    def until(self, prefix, timeout=120):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=min(0.1, deadline - time.monotonic()))
            except queue.Empty:
                if self.process.poll() is not None:
                    raise AssertionError("probe exited: " + "\n".join(self.log[-20:]))
                continue
            if line.startswith("READY "):
                self.pids.add(int(line.split("pid=")[1].split()[0]))
            if line.startswith(prefix):
                return line[len(prefix):]
        raise AssertionError(f"timed out waiting for {prefix}: " + "\n".join(self.log[-20:]))

    def stop(self):
        # Only explicit PIDs of this test's child applications and supervisor.
        for pid in self.pids:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        self.pids.clear()
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGINT)
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
        self.process.wait(timeout=5)


def text(node):
    return (node.get("text") or "") + "".join(map(text, node.get("children", [])))


def check_restart(timeout=180):
    original=SOURCE.read_text()
    probe=None
    try:
        probe = Probe(["cargo", "deka", "dev", "--locked", "-p", "deka-ui", "--no-default-features", "--example", "hot_reload_probe"])
        ready = probe.until("READY ", 300)
        old_pid = int(ready.split("pid=")[1].split()[0])
        probe.until("TREE ")
        started = time.monotonic()
        SOURCE.write_text(original.replace('count += 1', 'count += 2').replace('Deka, native.', 'Rebuilt, native.'))
        message = probe.until("deka dev: ", 10)
        while "rebuilding and restarting" not in message:
            message = probe.until("deka dev: ", 10)
        ready = probe.until("READY ", timeout)
        new_pid = int(ready.split("pid=")[1].split()[0])
        assert new_pid != old_pid
        probe.until("SCENE ")
        rebuilt = json.loads(probe.until("TREE "))
        assert "Rebuilt, native." in text(rebuilt) and "Count: 6" in text(rebuilt), text(rebuilt)
        elapsed = (time.monotonic() - started) * 1000
        print(f"cargo deka dev: restarted {old_pid} → {new_pid} in {elapsed:.1f} ms; new handler actually produces Count: 6", flush=True)
    finally:
        if probe is not None:
            probe.stop()
            (ROOT / "tasks/evidence/ui-hot-reload/supervisor-driver.log").write_text("\n".join(probe.log))
        SOURCE.write_text(original)


def main():
    arguments=argparse.ArgumentParser()
    arguments.add_argument("--case",choices=["all","markup","malformed","restart"],default="all")
    arguments.add_argument("--restart-timeout",type=float,default=180)
    args=arguments.parse_args()
    (ROOT / "tasks/evidence/ui-hot-reload").mkdir(parents=True,exist_ok=True)
    original = SOURCE.read_text()
    component_source=ROOT / "crates/deka_ui/examples/hot_reload_components_probe.rs"
    component_original=component_source.read_text()
    probe_source=ROOT / "crates/deka_ui/examples/hot_reload_probe.rs"
    probe_original=probe_source.read_text()
    probe = None
    try:
        if args.case=="restart":
            check_restart(args.restart_timeout)
            return
        command = ["cargo", "run", "--locked", "-p", "deka-ui", "--no-default-features", "--features", "hot-reload", "--example", "hot_reload_probe", "--", "--click-after-patch"]
        probe = Probe(command)
        probe.until("READY ", 300)
        initial_scene = json.loads(probe.until("SCENE "))
        initial = json.loads(probe.until("TREE "))
        assert "Count: 3" in text(initial), text(initial)
        if args.case=="malformed":
            SOURCE.write_text(original.replace('text-2xl','p-bad'))
            message=probe.until("deka ui:",2)
            assert "keeping last good UI" in message,message
            probe.until("SCENE ",2)
            assert json.loads(probe.until("TREE ",2))==initial
            print("malformed-only: terminal diagnostic and last good scene verified",flush=True)
            return
        # Static elements/attributes/class/text all change in the real tour.
        edited = original.replace('"Deka, native."', '"Hot, retained."').replace('text-2xl', 'text-xl').replace('<span>"The same UI, inside your browser."</span>', '<p id="reload-added">"New paragraph"</p><span>"Inserted sibling"</span>')
        started = time.monotonic()
        SOURCE.write_text(edited)
        changed_scene = json.loads(probe.until("SCENE ", 2))
        changed = json.loads(probe.until("TREE ", 2))
        elapsed = (time.monotonic() - started) * 1000
        assert elapsed < 500, elapsed
        assert initial_scene != changed_scene, "renderer scene did not change"
        assert "Hot, retained." in text(changed) and "New paragraph" in text(changed)
        assert "Count: 3" in text(changed), text(changed)
        clicked = json.loads(probe.until("AFTER_CLICK ", 2))
        assert "Count: 4" in text(clicked), "handler/signal did not survive the patch"
        print(f"cargo run: scene changed in {elapsed:.1f} ms; signal 3 survived; retained handler advanced it to 4", flush=True)
        if args.case=="markup":return
        SOURCE.write_text(edited.replace('text-xl', 'p-bad'))
        malformed_message = probe.until("deka ui:", 2)
        assert "keeping last good UI" in malformed_message, malformed_message
        probe.until("SCENE ", 2)
        malformed_tree = json.loads(probe.until("TREE ", 2))
        assert malformed_tree == clicked, "malformed edit changed the last good tree"
        print("malformed edit: diagnostic delivered; complete last good tree retained", flush=True)
        # Correcting a malformed save must patch without recompiling.
        SOURCE.write_text(edited.replace('Hot, retained.', 'Recovered, retained.'))
        probe.until("SCENE ", 2)
        recovered = json.loads(probe.until("TREE ", 2))
        assert "Recovered, retained." in text(recovered) and "Count: 4" in text(recovered)
        # A combined markup + compiled-handler edit must never partially patch.
        probe_source.write_text(probe_original.replace('"compiled status"', '"changed compiled status"'))
        SOURCE.write_text(edited.replace('Hot, retained.', 'Must not patch'))
        message = probe.until("deka ui:", 2)
        assert "rebuilding and restarting" in message, message
        probe.until("SCENE ", 2)
        stale = json.loads(probe.until("TREE ", 2))
        assert stale == recovered, "compiled-code edit partially patched markup"
        print("cross-file non-markup edit: restart required; no partial markup applied", flush=True)
        probe.stop()
        SOURCE.write_text(original)
        probe_source.write_text(probe_original)
        probe=Probe(["cargo","run","--locked","-p","deka-ui","--no-default-features","--features","hot-reload","--example","hot_reload_components_probe"])
        probe.until("READY ",300)
        component_source.write_text(component_original.replace('<view><Counter id="first"/><Counter id="second"/></view>', '<div class="p-4"><p>"Sibling"</p><Counter id="second"/><Counter id="first"/></div>'))
        components=json.loads(probe.until("COMPONENTS ",2))
        assert components=={"order":["second","first"],"state":[7,11]},components
        print("keyed components: reordering and wrapper change preserved both identities and independent states 7/11",flush=True)
        # A second edit patches both live instances of the component's template.
        component_source.write_text(component_source.read_text().replace('<span id={id}>','<span id={id} class="p-2">').replace('class="p-4"','class="p-6"'))
        components=json.loads(probe.until("COMPONENTS ",2))
        assert components["state"]==[7,11]
        probe.stop()
        component_source.write_text(component_original)
        check_restart()
    finally:
        if probe is not None:
            probe.stop()
            (ROOT / "tasks/evidence/ui-hot-reload/probe-driver.log").write_text("\n".join(probe.log))
        SOURCE.write_text(original)
        component_source.write_text(component_original)
        probe_source.write_text(probe_original)


if __name__ == "__main__":
    main()
