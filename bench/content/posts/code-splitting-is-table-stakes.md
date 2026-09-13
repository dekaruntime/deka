---
title: Code splitting is table stakes
date: 2026-02-19
tags: [web, javascript, performance]
excerpt: A Vite blog that ships one JS file for twenty-eight posts is a strawman. Lazy routes are the baseline.
---

The uncharitable Vite app is a single `main.tsx` that statically imports every page. Nobody shipping a real blog does that in 2026.

```tsx
const PostPage = lazy(() => import("./pages/PostPage"));
const TagPage = lazy(() => import("./pages/TagPage"));
const AboutPage = lazy(() => import("./pages/AboutPage"));
```

The post-page payload is then: the HTML shell, the shared vendor chunk, the post route chunk, and CSS. That is the number we record. If deka ships less JS, it should be because the route did not need it, not because we forgot `lazy()`.

