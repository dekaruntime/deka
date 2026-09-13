---
title: Fair play is the product
date: 2026-01-18
tags: [tooling, language-design]
excerpt: A rigged benchmark is worse than none. The methodology is the homepage, not a footnote.
---

The JS ecosystem learned the wrong lesson from a decade of "framework X is 2ms faster" charts: that the chart is the argument. The argument is whether a stranger can rerun the chart.

Fair play, for this repo:

- pinned toolchains (`deka 0.52.0`, `dsc 0.52.2`, React 19.1.1 on the Vite side)
- identical markdown, identical routes, identical widgets
- production configs, never a development server in a production column
- one machine, isolated runs, medians of five
- a table that says what is not measured yet

```json
{
  "deka": "0.52.0",
  "dsc": "0.52.2",
  "react": "19.1.1",
  "vite": "pinned in bench/vite-blog/package.json"
}
```

If we cannot name the versions, we do not publish the number.

