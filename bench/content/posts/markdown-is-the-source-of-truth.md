---
title: Markdown is the source of truth
date: 2026-02-11
tags: [tooling, web]
excerpt: Both subject apps ingest the same files. Divergent HTML is a bug in the ingest, not a framework feature.
---

There is a temptation to hand-write DSX posts on one side and MDX on the other and call them "the same blog". They will drift in a week.

This bench keeps the posts in `bench/content/posts/` as markdown with front matter. A tiny ingest step — not timed — emits:

- `deka-blog/src/posts.generated.ds`
- `vite-blog/src/posts.generated.ts`

```md
---
title: Markdown is the source of truth
date: 2026-02-11
tags: [tooling, web]
---

The paragraph you are reading is the input.
```

Syntax highlighting happens in the ingest so both runtimes paint the same spans. Measuring `highlight.js` against a hand-rolled tokenizer would be a different paper.

