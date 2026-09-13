---
title: Cold builds and the cache you forgot
date: 2026-01-22
tags: [performance, tooling]
excerpt: A 'cold' build that reused a persistent compiler daemon is a warm build wearing a hat.
---

Cache hygiene is part of the methodology. The easy miss is deleting `dist/` and leaving the compiler's own scratch directory.

For this bench:

- deka cold: `dist/`, `.cache/`, `.deka-dist-stage/`
- Vite cold: `dist/`, `node_modules/.vite/`
- neither cold includes reinstalling dependencies

```bash
rm -rf deka-blog/dist deka-blog/.cache deka-blog/.deka-dist-stage
rm -rf vite-blog/dist vite-blog/node_modules/.vite
```

Incremental is the opposite ritual: leave the caches, change one source file that the production graph actually imports, rebuild.

If your incremental number equals your cold number, you did not hit the cache, or the toolchain does not have one.

