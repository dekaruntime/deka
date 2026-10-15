#!/usr/bin/env python3
"""Encode the production offscreen RGBA evidence as lossless PNGs, no image edits."""
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[2]
WIDTH, HEIGHT = 560, 480

def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))

def main():
    source = ROOT / '.tmp/components-shots'
    output = ROOT / 'docs/rust-ui/assets'
    output.mkdir(parents=True, exist_ok=True)
    for component in ['button', 'input', 'list', 'tabs', 'badge', 'toast', 'dialog', 'menu']:
        for theme in ['light', 'dark']:
            data = (source/f'{component}-{theme}.rgba').read_bytes()
            if len(data) != WIDTH*HEIGHT*4:
                raise RuntimeError(f'{component}-{theme}: incomplete RGBA screenshot')
            rows = b''.join(b'\0' + data[start:start+WIDTH*4] for start in range(0,len(data),WIDTH*4))
            png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR',struct.pack('>IIBBBBB',WIDTH,HEIGHT,8,6,0,0,0)) + chunk(b'IDAT',zlib.compress(rows,9)) + chunk(b'IEND',b'')
            path = output/f'{component}-{theme}.png'
            path.write_bytes(png)
            print(f'{path.relative_to(ROOT)}: {len(png)} bytes')

if __name__ == '__main__':
    main()
