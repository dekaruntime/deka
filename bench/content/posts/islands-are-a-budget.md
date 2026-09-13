---
title: Islands are a budget, not a motif
date: 2026-01-14
tags: [web, runtime, language-design]
excerpt: An island is a confession that this subtree needs a runtime. Spend them like you spend bytes.
---

The island diagram is easy to draw and easy to abuse. Two colored rectangles on a static page do not make a framework honest. The confession is the interesting part: this component closes over state that has to exist in the browser.

![Static page with two islands](/images/island.svg)

On this blog the budget is two:

- a theme toggle that must remember `light` / `dark`
- a newsletter form that must talk back to the user

Everything else is HTML. If we add a third island "for the demo", we are no longer measuring a blog.

```ds
export fn ThemeToggle() ReactNode {
  const theme = useContext(ThemeContext)
  return <button type="button" class="theme-toggle">{theme}</button>
}
```

The directive `client:load` is the receipt. No directive, no client JS. That is the contract this subject app is here to dogfood.

