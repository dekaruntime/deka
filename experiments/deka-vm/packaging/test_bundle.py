"""End-to-end packaging checks; macOS only. All temporary files stay under output."""
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile


def check(packager, runtime, manifest, output):
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="package-test-", dir=output) as tmp:
        root = Path(tmp)
        project = root / "project"
        shutil.copytree(manifest.parent, project)
        config = json.loads(manifest.read_text())
        # A different project configuration and changed source must reach the artifact.
        source = (manifest.parent / config["desktop"]["entry"]).read_text()
        (project / "main.dsx").write_text(source.replace("count += 1", "count += 7"))
        config["version"] = "0.2.3"
        config["desktop"].update(productName="Relocation Test", identifier="gg.deka.test.relocation", entry="main.dsx")
        config_path = project / "deka.json"
        config_path.write_text(json.dumps(config))
        subprocess.run([packager, config_path, runtime, "--out", root / "stage"], check=True)
        # A separately compiled payload must work even without source files.
        precompiled = root / "compiled.dvm.json"
        shutil.copyfile(root / "stage/app.dvm.json", precompiled)
        (project / "main.dsx").unlink()
        subprocess.run([packager, config_path, runtime, "--out", root / "precompiled", "--bytecode", precompiled], check=True)
        built = root / "precompiled/bundle/macos/Relocation Test.app"
        relocated = root / "elsewhere/Relocation Test.app"
        shutil.copytree(built, relocated)
        # Remove both build output and input project before executing the copied app.
        shutil.rmtree(root / "stage")
        shutil.rmtree(project)
        contents = relocated / "Contents"
        with (contents / "Info.plist").open("rb") as stream:
            info = plistlib.load(stream)
        assert info["CFBundleName"] == "Relocation Test", info
        assert info["CFBundleIdentifier"] == "gg.deka.test.relocation", info
        assert info["CFBundleShortVersionString"] == "0.2.3", info
        assert (contents / "Resources" / info["CFBundleIconFile"]).is_file()
        assert (contents / "Resources/ABOUT.txt").read_bytes() == (manifest.parent / "ABOUT.txt").read_bytes()
        executable = contents / "MacOS" / info["CFBundleExecutable"]
        command = [executable, "--exercise", "3"]
        result = subprocess.run(command, cwd=root, capture_output=True, text=True)
        assert result.returncode == 0, result.stderr
        assert "Count: 21" in result.stdout, result.stdout
        subprocess.run(["/usr/bin/codesign", "--verify", "--strict", relocated], check=True)
        # Missing bundled bytecode must fail, not silently fall back to a demo/checkout.
        (contents / "Resources/app.dvm.json").unlink()
        missing = subprocess.run(command, cwd=root, capture_output=True, text=True)
        assert missing.returncode != 0, missing.stdout
        assert "cannot read application payload" in missing.stderr, missing.stderr
        print("PASS: deka.json metadata/icon/resources, source change, relocated app handlers, signature, missing-payload failure")


if __name__ == "__main__":
    if len(sys.argv) != 5:
        raise SystemExit("usage: test_bundle.py packager runtime deka.json output-directory")
    check(*(Path(arg).resolve() for arg in sys.argv[1:]))
