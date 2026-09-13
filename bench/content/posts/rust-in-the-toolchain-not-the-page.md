---
title: Rust in the toolchain, not the page
date: 2026-02-24
tags: [rust, compilers, runtime]
excerpt: The compiler being Rust does not make the blog a Rust app. Keep the boundary boring.
---

dsc is a Rust program. deka's host is a Rust program. The subject app is DekaScript that emits JavaScript. Mixing those layers in a README is how "written in Rust" becomes a payload claim.

```rs
pub fn require_dsc() -> Result<PathBuf, String> {
    find_cli_dsc()?.ok_or_else(|| {
        "dsc is required for deka build".to_string()
    })
}
```

The blog never imports that function. The runner execs the released `deka` binary, which execs the released `dsc` binary. Build times include both, because a user who types `deka build` pays for both.

