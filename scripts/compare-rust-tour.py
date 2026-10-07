#!/usr/bin/env python3
"""Fail unless two complete packages contain the same bytes, including fixtures."""
from pathlib import Path
import hashlib
import sys

def hashes(root):
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in root.rglob('*') if p.is_file()}
a, b = map(Path, sys.argv[1:])
assert hashes(a) == hashes(b), 'Rust tour rebuild is not byte reproducible'
print(f'PASS: {len(hashes(a))} package files have identical sha256')
