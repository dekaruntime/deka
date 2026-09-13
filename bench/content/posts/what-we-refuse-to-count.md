---
title: What we refuse to count
date: 2026-04-21
tags: [performance, tooling]
excerpt: Install time, image bytes, and font downloads are real costs. They are not this comparison.
---

Out of scope for the production columns:

- `npm install` / `deka` download
- the SVG images in `/images`
- system fonts
- gzip of the runner itself
- time to start the preview server (payload uses it, build does not)

In scope:

- `deka build` and `vite build` wall clocks
- HTML+JS+CSS bytes to read `/posts/zero-js-by-default`, gzipped

If a future lane wants "Lighthouse on a phone", it should say so and pin the phone.

