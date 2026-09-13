---
title: Released binaries only
date: 2026-03-01
tags: [tooling, runtime]
excerpt: Homepage numbers from an unpublished binary are a preview. Record the commit. Ship the release before the homepage.
---

The methodology prefers GitHub releases. Islands hydration landed on `main` in deka#948 and is not in a released `deka` yet, so this clone's deka column runs a **main-build** copied to `bench/.toolchain/`, with the git sha recorded in the results stanza. Vite still uses the pinned npm release. `dsc` stays the released compiler.

```bash
cargo build --release -p cli
cp target/release/cli bench/.toolchain/deka
```

The next deka release replaces that pin. Until then the table says "main-build pending the next release" instead of pretending v0.52.0 hydrated islands.
