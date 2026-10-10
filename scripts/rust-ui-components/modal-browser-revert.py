#!/usr/bin/env python3
"""Prove modal DOM parenting and key routing using the shipped browser host."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
CASES = [
    ('modal-dom-parent',
     'active.has(node.parent) ? (semantics.get(node.parent) ?? parent) : parent',
     'semantics.get(node.parent) ?? parent',
     'modal focus reaches the actual profile input'),
    ('modal-key-target',
     ' || event.target !== event.currentTarget', '',
     'Space closes the focused modal close button'),
]


def main():
    package = Path(sys.argv[1]).resolve()
    source = (ROOT / 'crates/deka_ui/web/host.js').read_bytes()
    hosts = list(package.rglob('host.js'))
    originals = {path: path.read_bytes() for path in hosts}
    assert hosts and all(data == source for data in originals.values()), 'Package host differs from source'
    output = ROOT / '.tmp/components-revert'
    output.mkdir(parents=True, exist_ok=True)
    command = ['node', 'scripts/rust-tour-browser/check.mjs', str(package), '--components-only']
    for name, old, new, failure in CASES:
        assert source.count(old.encode()) == 1, f'{name}: mutation anchor is not unique'
        try:
            for path, data in originals.items():
                path.write_bytes(data.replace(old.encode(), new.encode()))
            result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=120)
            log = result.stdout + result.stderr
            (output / f'{name}.log').write_text(log)
            assert result.returncode != 0 and failure in log, f'{name}: expected the named DOM interaction assertion; see {output / name}.log'
            print(f'PASS browser revert: {name} fails {failure}', flush=True)
        finally:
            for path, data in originals.items():
                path.write_bytes(data)
    subprocess.run(command, cwd=ROOT, check=True, timeout=120)
    print('PASS restored: all component DOM interactions', flush=True)


if __name__ == '__main__':
    main()
