---
title: Measuring the wrong loop
date: 2026-01-09
tags: [performance, tooling]
excerpt: Dev-mode HMR times are a different sport from production cold builds. Mixing them is how strawmen get published.
---

There are at least four clocks in a frontend toolchain and they are not interchangeable.

| Clock | What it includes | What it must not include |
| --- | --- | --- |
| cold production build | empty cache, deps already installed | `npm install`, network |
| incremental production build | one source byte changed | wiping `node_modules` |
| HMR | file save → paint | a full reload you called "fast" |
| TTFB | first byte of the production server | a warm CDN edge in another region |

![Compile pipeline](/images/pipeline.svg)

This phase of the bench measures the first two plus payload. HMR and TTFB wait for a later lane because we will not print a number we cannot regenerate.

```bash
# cold is a cache shape, not a machine reboot
rm -rf dist .cache node_modules/.vite
```

If a write-up reports "Vite is slow" from a cold `npm create` on Wi-Fi, it measured the registry, not the bundler.

