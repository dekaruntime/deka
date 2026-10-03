#!/usr/bin/env python3
"""Compare layout bounds (scene.nodes[].layout_rect) between two harness runs."""
import json
import sys

before = json.load(open(sys.argv[1]))
after = json.load(open(sys.argv[2]))
for scene in before:
    b, a = before[scene], after.get(scene, [])
    print(f"\n### {scene}: {len(b)} nodes before, {len(a)} after")
    print("| node | before x,y,w,h | after x,y,w,h | dw | dh |")
    print("|---|---|---|---|---|")
    worst = (0, 0)
    for nb, na in zip(b, a):
        label = (nb["text"] or "(element)")[:38].replace("|", "/")
        fmt = lambda n: f'{n["x"]:.0f},{n["y"]:.0f},{n["w"]:.0f},{n["h"]:.0f}'
        dw, dh = na["w"] - nb["w"], na["h"] - nb["h"]
        worst = (max(worst[0], abs(dw)), max(worst[1], abs(dh)))
        print(f"| {label} | {fmt(nb)} | {fmt(na)} | {dw:+.0f} | {dh:+.0f} |")
    print(f"max |dw| {worst[0]:.0f} px, max |dh| {worst[1]:.0f} px")
