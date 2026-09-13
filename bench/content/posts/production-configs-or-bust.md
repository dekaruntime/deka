---
title: Production configs or bust
date: 2026-02-15
tags: [tooling, performance]
excerpt: If the column says production, the command must be the production command. Dev servers do not get a costume.
---

A development server is allowed to be slow, chatty, and stateful. A production build is allowed to be none of those.

Vite's production command here is `vite build`. Deka's is `deka build`. Preview/serve after that is how we read payload, not how we time compile.

```ts
export default defineConfig({
  plugins: [react()],
  build: {
    target: "es2022",
    sourcemap: false,
    modulePreload: { polyfill: false },
  },
});
```

Source maps off, because we are counting bytes a user downloads, not bytes a debugger wants. The React plugin stays on, because that is how a senior Vite app is actually built.

