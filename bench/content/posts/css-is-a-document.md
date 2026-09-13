---
title: CSS is a document
date: 2026-05-01
tags: [web, performance]
excerpt: One stylesheet, same rules, both apps. Utility soup in one column would be a costume change.
---

Both subject apps load `/style.css`. The file is duplicated (each app has a public root) and kept byte-identical by the ingest check in the runner.

No CSS-in-JS runtime on the Vite side. No per-route CSS extraction claimed on the deka side. A later lane can measure that if someone ships it.

```css
:root { color-scheme: light dark; }
[data-theme="dark"] { color-scheme: dark; }
.prose pre { overflow: auto; }
```

Dark mode is a `data-theme` attribute, not a second stylesheet. The island writes the attribute; the document already knew the rules.

