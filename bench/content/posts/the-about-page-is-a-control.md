---
title: The about page is a control
date: 2026-03-18
tags: [web, tooling]
excerpt: If the about page and a post page ship the same JS, you do not have a zero-JS story. You have a bundle.
---

Control routes matter. `/about` has the chrome, the theme island, the newsletter island, and no code block. A post page has all of that plus highlighted HTML.

The payload table should not pretend those are the same class of page. Phase 1 reports a post page because that is the page a stranger actually reads. A later lane can split "static page" vs "island page" the way the issue asks.

Until then: one representative URL, named in the README, fetched the same way for both apps.

