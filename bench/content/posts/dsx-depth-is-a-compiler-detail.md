---
title: DSX depth is a compiler detail
date: 2026-03-27
tags: [compilers, language-design]
excerpt: App routes stay thin. Components live in src/ui. That split is authoring hygiene, not a hidden runtime.
---

The subject app keeps JSX in `src/ui/*.dsx` and keeps `app/**/page.dsx` as thin wrappers that call those components. Production JSX resolves from the runtime builtin `@js/react/jsx-runtime` — no `deka.json` `jsxRuntime` field, no hand-rolled renderer.

```ds
import { HomePage } from "../src/ui/HomePage.dsx"
export fn Page() ReactNode {
  return HomePage()
}
```

Theme and Newsletter are the exceptions that opt into the client: they are the same DSX components, marked `client:load`, and the #948 pipeline hydrates them. Everything else is HTML the compiler already knew.
