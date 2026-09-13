---
title: Reproducing on another machine
date: 2026-05-06
tags: [tooling, runtime]
excerpt: If the README's machine block is empty, the number is a souvenir. Fill it from the runner, not from memory.
---

The runner prints a machine stanza:

- OS and version
- architecture
- CPU brand
- memory
- `deka --version` and `dsc --version`
- `node --version`

Copy that stanza into any published chart. If you rerun on a laptop that is also compiling a kernel, say so. Isolation is part of fair play, not a vibe.

```txt
os: macOS 26.6.1
arch: x86_64
cpu: Intel(R) Core(TM) i9-10910 CPU @ 3.60GHz
```

samis-imac is the machine named in the issue. This clone may be that machine or it may not. The runner does not guess; it records.

