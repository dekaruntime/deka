---
title: Payload is a graph, not a folder
date: 2026-01-27
tags: [web, performance]
excerpt: Bytes over the wire are the document plus everything it pulls. Listing dist/ is not a payload.
---

A production folder can contain routes you never request. The number that belongs on a homepage is the bytes a browser downloads to read **one post**.

For a post page that means:

1. the HTML document
2. every `link rel="stylesheet"`
3. every `script` and `modulepreload` the document names
4. gzipped, because that is what went over the wire

```js
const doc = await fetch(url).then((r) => r.text());
const urls = [...doc.matchAll(/<(?:script|link)[^>]+(?:src|href)="([^"]+)"/g)]
  .map((m) => m[1]);
```

Images are content, not toolchain. We keep them in the subject app so the pages are real, and we do not fold their bytes into the framework column.

