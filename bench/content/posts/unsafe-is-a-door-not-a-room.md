---
title: unsafe is a door, not a room
date: 2026-04-02
tags: [language-design, runtime]
excerpt: Newsletter submit has to call preventDefault. That is a door. The form itself stays ordinary DSX.
---

Application DekaScript cannot call `deka.*`. The generated serve entry can, and does: `unsafe { deka.ui.renderToStreamHtml(tree) }`. Island event handlers that talk to the DOM use a small `unsafe` block as a door, then return to typed code.

```ds
onSubmit={fn(event: Any) void {
  const _ = unsafe {
    event.preventDefault()
    return 1
  }
  setSent(true)
}}
```

Posts, layouts, and islands stay in ordinary DSX. If the whole blog were an unsafe block, we would not be dogfooding the language.
