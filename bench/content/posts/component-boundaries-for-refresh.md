---
title: Component boundaries for refresh
date: 2026-04-26
tags: [runtime, javascript]
excerpt: HMR is phase two, but the subject app still has to be refreshable or the later measurement will be a rewrite.
---

Fast Refresh cares about module shape: PascalCase components, stable hook order, no surprising exports. The deka components are written to that shape now so phase 2 is an instrument, not a redesign.

```ds
export fn PostCard(props: PostCardProps) ReactNode {
  return <article class="card"><a href={props.href}>{props.title}</a></article>
}
```

A sibling edit of `PostCard` should not reset the theme toggle's state. That sentence is the HMR spec. We do not time it in this lane; we do not write components that would make the sentence untestable.

