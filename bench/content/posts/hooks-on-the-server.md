---
title: Hooks on the server are a scheduling story
date: 2026-02-02
tags: [runtime, language-design]
excerpt: useState during SSR is legal. useEffect during SSR is a no-op. Mixing those facts is how you leak documents.
---

The compiler-known hooks in DekaScript are not a courtesy import. They are a color on the function: a hook-typed function is only callable from a component or another hook.

```ds
export fn ThemeToggle() ReactNode {
  const pair = useState("light")
  const theme = pair[0]
  useEffect(fn() Option<fn() void> {
    return None
  })
  return <button type="button" data-theme={theme}>{theme}</button>
}
```

On the server, `useState` exists so the first paint has a value. `useEffect` exists so the client can subscribe after hydration. Running the effect on the server would be a second renderer pretending to be a browser.

The theme island is where those two worlds meet. The document can ship `data-theme="light"` without JS; the island writes the preference once a human asks. No directive, no client JS.
