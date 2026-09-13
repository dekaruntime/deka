---
title: unsafe is a door, not a room
date: 2026-04-02
tags: [language-design, runtime]
excerpt: The renderer polyfill lives in an unsafe block because the host freeze of deka.ui is the wall we have. Do not build the blog inside that block.
---

Application DekaScript cannot call `deka.*`. The generated serve entry can, and does: `unsafe { deka.ui.renderToStreamHtml(tree) }`. The 0.52 isolate bootstrap freezes `deka.ui` as signals-only.

The subject app therefore installs `renderToString` through a module-scope `unsafe` block in the JSX runtime. That is a door: it runs once at module load, patches the host object, and gets out.

```ds
const _boot: number = match (unsafe {
  globalThis.deka.ui = Object.assign({}, globalThis.deka.ui, {
    renderToString: renderToString,
    renderToStreamHtml: renderToStreamHtml
  })
  return 1
}) {
  Ok(n) => n,
  Err(_) => 0
}
```

Posts, layouts, and islands stay in ordinary DSX. If the whole blog were an unsafe block, we would not be dogfooding the language.

