---
title: Internal baseline, same harness
date: 2026-04-16
tags: [tooling, runtime, performance]
excerpt: The homepage chart and the release-train gate have to be the same command. Two harnesses will drift.
---

If marketing runs a laptop script and CI runs a trimmed subset, you will eventually publish a number CI cannot see regress. The runner is `bench/run.mjs`. CI can call it. A human can call it. The table format does not change.

```bash
node bench/run.mjs
```

That is the whole interface. Flags can be added later for `--only=vite` when iterating. The default is both apps, both clocks, one payload URL.

