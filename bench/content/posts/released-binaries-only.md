---
title: Released binaries only
date: 2026-03-01
tags: [tooling, runtime]
excerpt: Homepage numbers from `cargo run --release` are a preview. Homepage numbers from GitHub releases are a product.
---

This bench installs `deka 0.52.0` and `dsc 0.52.2` as release artifacts. It does not build the compiler from the commit that contains the bench. That would make the bench a moving target and the number a function of the PR.

```bash
# from https://deka.gg/install
curl -fsSL https://deka.gg/install.sh | DEKA_VERSION=v0.52.0 DSC_VERSION=v0.52.2 sh
```

If you are iterating on the compiler, run the bench against the released pair anyway. A regression against yourself is a different chart.

