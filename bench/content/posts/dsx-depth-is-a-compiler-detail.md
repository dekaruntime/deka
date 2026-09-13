---
title: DSX depth is a compiler detail
date: 2026-03-27
tags: [compilers, language-design]
excerpt: jsxRuntime is a specifier string. File depth decides whether the artifact graph closes. That is a bench detail, not a language tutorial.
---

Deka 0.52 emits `import { jsx } from "<jsxRuntime>"` into every file that contains JSX. Production artifacts then require that specifier to resolve to a relative `.js` module inside `dist/server`.

The subject app therefore keeps JSX in `src/ui/*.dsx` — one directory depth that matches the generated serve entry — and keeps `app/**/page.dsx` as thin wrappers that call those components.

```ds
import { HomePage } from "../src/ui/HomePage.dsx"
export fn Page() ReactNode {
  return HomePage()
}
```

This is not how we want the authoring story to read in a year. It is how a closed artifact graph looks today, and hiding it would be a rigged screenshot.

