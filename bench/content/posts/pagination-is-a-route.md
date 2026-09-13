---
title: Pagination is a route
date: 2026-03-05
tags: [web, javascript]
excerpt: Page 2 is not a query string the client interprets. It is a document with its own URL.
---

A paginated index that rewrites in-place is an application. A paginated index that is `/`, `/page/2`, `/page/3` is a set of documents.

Both subject apps expose the same routes. The Vite app can still be a SPA; the URLs have to exist so the payload fetch hits a real post listing.

```txt
/                 page 1
/page/2           page 2
/posts/:slug      a post
/tags/:tag        a tag
/about            about
```

Twenty-eight posts at six per page is five index pages, plus the posts, plus the tags, plus about. That is the "25+ pages" bar, not a carousel.

