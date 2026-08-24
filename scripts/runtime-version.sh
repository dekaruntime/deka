#!/usr/bin/env bash
# Print the lockstep runtime version from [workspace.package], falling back to
# crates/cli. Used by bump-version.sh and the release workflow.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

python3 - "$ROOT_DIR" <<'PY'
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])


def first_version(text: str, section: str | None = None) -> str | None:
    if section:
        m = re.search(
            rf"(?ms)^\[{re.escape(section)}\]\s*(.*?)(?=^\[|\Z)",
            text,
        )
        if not m:
            return None
        text = m.group(1)
    m = re.search(r'(?m)^version\s*=\s*"([^"]+)"', text)
    return m.group(1) if m else None


ws = (root / "Cargo.toml").read_text()
version = first_version(ws, "workspace.package")
if not version:
    cli = (root / "crates/cli/Cargo.toml").read_text()
    version = first_version(cli, "package")
if not version:
    sys.stderr.write("could not read runtime version from Cargo.toml\n")
    sys.exit(1)
print(version)
PY
