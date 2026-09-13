---
title: The subject app is the spec
date: 2026-05-11
tags: [language-design, web, runtime]
excerpt: If a feature cannot be expressed in the blog, it is not in the comparison. The app is the contract.
---

Issue 938 lists the floor: shared markdown, paginated index, tag pages, about, layout, cards, highlighted code, a theme island, a newsletter island, images.

This directory is that list, implemented twice. When we add Next.js, we will not "simplify the subject" to make RSC look good. We will implement the same list a third time.

```txt
bench/content/posts/*.md
bench/deka-blog/
bench/vite-blog/
bench/run.mjs
```

A missing page is a missing measurement. A bonus widget is a tainted one. Stay on the list.

