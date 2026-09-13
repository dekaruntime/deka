---
title: Tag pages are cheap if you let them be
date: 2026-03-09
tags: [web, performance]
excerpt: A tag page is a filter over static data. It does not need a search runtime until it is a search.
---

`/tags/runtime` is a list of cards. The data was known at ingest. The honest implementation is a function from `tag` to `Post[]`.

```ds
export fn postsForTag(tag: string) Array<Post> {
  let out: Array<Post> = []
  for (const post of posts) {
    if (hasTag(post.tags, tag)) {
      out.push(post)
    }
  }
  return out
}
```

Adding typeahead on top of that is a product decision and a new island. This bench does not have it, on purpose.

