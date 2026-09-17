---
title: Vite + React 19 is not a strawman
date: 2026-04-07
tags: [javascript, web, tooling]
excerpt: The comparison target is a senior React app: lazy routes, production build, the same React version the deka vendor pins.
---

React 19.1.1 is the version vendored for `deka dev` Fast Refresh. The Vite app pins the same version from npm, because npm is how that ecosystem is actually consumed.

```json
{
  "dependencies": {
    "react": "19.1.1",
    "react-dom": "19.1.1",
    "react-router-dom": "7.18.4"
  }
}
```

No `React.FC` archaeology, no CSS-in-JS runtime, no extra state library for a theme string. Context, lazy, and a CSS file. If deka wins, it should win against that, not against a 2018 CRA screenshot.

