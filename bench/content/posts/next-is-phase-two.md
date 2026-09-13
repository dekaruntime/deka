---
title: Next.js is phase two
date: 2026-04-11
tags: [web, tooling]
excerpt: A two-way chart is not a three-way chart. We will not invent Next numbers to fill a cell.
---

The issue names three columns: deka, Vite+React, Next.js App Router. This lane ships the first two plus the runner skeleton. The Next app, HMR, and TTFB are empty cells with an explicit TODO.

An empty cell is a promise. A guessed cell is a lie.

When phase 2 lands, it will:

- add `bench/next-blog` with App Router and a production config
- pin a Next version next to the others
- fill HMR via CDP timestamps
- fill TTFB from the production server

Until then the README table keeps the holes visible.

