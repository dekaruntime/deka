---
title: Zero JS by default is a product decision
date: 2026-01-04
tags: [runtime, web, performance]
excerpt: Most of a blog is dead HTML. Shipping a framework to paint it is a choice, not a law of physics.
---

A homepage number is only as honest as the page it describes. This bench is a blog because blogs are the most common "we shipped a SPA by accident" app: twenty-odd articles, a couple of interactive widgets, and a lot of prose that never needed a runtime.

![Abstract runtime graph](/images/hero.svg)

The zero-JS default is not an aesthetic. If a route has no event handlers, the HTML that left the compiler should be the HTML the browser paints. Islands exist so we can break that rule on purpose.

## What "default" means

Default is the path you get when you do not opt in. A toggle and a newsletter form opt in. A tag listing does not.

```js
export function PostCard({ post }) {
  return `<article><a href="/posts/${post.slug}">${post.title}</a></article>`;
}
```

If that function never reads `window`, never registers an effect, and never closes over a setter, it is documentation, not a program. Treating it as a program is how 180 kB of JavaScript shows up on a paragraph of text.

## The audit you should run

1. Disable JavaScript.
2. Read a post.
3. Follow two internal links.
4. Only then turn JS back on and use the widgets.

If step 2 fails, the "blog" is an application pretending to be a document.

