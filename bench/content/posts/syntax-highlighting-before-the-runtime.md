---
title: Syntax highlighting before the runtime
date: 2026-03-13
tags: [tooling, javascript]
excerpt: Highlighting in the client is a payload tax. Highlighting in the ingest is a compile-time color.
---

Code blocks in these posts are highlighted once, in the shared ingest, into `<span class="tok-*">` nodes. Both apps render the HTML as a document fragment.

```rs
fn find_dsc() -> Result<Option<PathBuf>, String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("dsc");
            if sibling.is_file() {
                return Ok(Some(sibling));
            }
        }
    }
    Ok(None)
}
```

A client highlighter would pull a grammar table into every post page. That is a real cost and a real choice. It is not this choice.

