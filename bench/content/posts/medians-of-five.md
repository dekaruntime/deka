---
title: Medians of five
date: 2026-03-22
tags: [performance, tooling]
excerpt: A single run is an anecdote. Five runs and a median is still a small sample, but it is a sample.
---

N=5 is a truce with wall-clock noise, not a statistics paper. We report the median so a thermal throttle on run 4 does not become the homepage.

```js
function median(values) {
  const xs = [...values].sort((a, b) => a - b);
  const mid = Math.floor(xs.length / 2);
  return xs.length % 2 ? xs[mid] : (xs[mid - 1] + xs[mid]) / 2;
}
```

If a run is more than twice the median, the runner prints it and still uses the median. Outliers are data. They are not the number.

