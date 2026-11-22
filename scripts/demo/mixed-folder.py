#!/usr/bin/env python3
"""A big folder that looks like a busy real one, for the big-folder speed clip.

    scripts/demo/mixed-folder.py <dir> [count]

Makes file-000000 … with types drawn by weight, sizes around each type's usual
size (sparse files, so they take no disk) and modified dates over three years,
all from a fixed seed. `crow --make-fixture` stays the bench's folder: its types
repeat every 8 rows, which strobes in a clip that scrolls several rows a frame.
"""
import os
import random
import sys
import time

out = sys.argv[1]
count = int(sys.argv[2]) if len(sys.argv) > 2 else 100_000
os.makedirs(out)
rng = random.Random(count)

# (extension, weight, usual size in bytes)
KINDS = [
    ("jpg", 9, 3_000_000), ("png", 7, 400_000), ("heic", 5, 2_500_000), ("pdf", 7, 900_000),
    ("txt", 5, 4_000), ("md", 6, 8_000), ("rs", 6, 12_000), ("swift", 4, 10_000),
    ("py", 5, 9_000), ("js", 4, 20_000), ("go", 3, 15_000), ("json", 5, 30_000),
    ("toml", 3, 2_000), ("yaml", 2, 3_000), ("html", 3, 25_000), ("css", 2, 12_000),
    ("csv", 3, 200_000), ("xlsx", 2, 150_000), ("docx", 3, 120_000), ("pptx", 1, 4_000_000),
    ("key", 1, 8_000_000), ("zip", 3, 20_000_000), ("dmg", 1, 120_000_000),
    ("mov", 2, 300_000_000), ("mp4", 2, 80_000_000), ("mp3", 2, 6_000_000),
    ("wav", 1, 30_000_000), ("svg", 2, 6_000), ("gif", 1, 1_500_000), ("sh", 2, 2_000),
    ("log", 2, 500_000),
]
names = [kind[0] for kind in KINDS]
weights = [kind[1] for kind in KINDS]
usual = {kind[0]: kind[2] for kind in KINDS}
now = time.time()

for i in range(count):
    ext = rng.choices(names, weights)[0]
    path = os.path.join(out, f"file-{i:06d}.{ext}")
    with open(path, "wb") as f:
        f.truncate(int(usual[ext] * rng.lognormvariate(0, 0.8)))
    stamp = now - rng.uniform(0, 3 * 365 * 86400)
    os.utime(path, (stamp, stamp))
print(f"{count} files: {out}")
