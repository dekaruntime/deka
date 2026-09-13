---
title: Context is a tree, not a store
date: 2026-02-06
tags: [language-design, web]
excerpt: useContext without a provider is a missing-requirement error, not a default you forgot to document.
---

A store is a process singleton. Context is a prefix of the tree. That difference is why a theme provider belongs above the layout chrome and why a newsletter form does not need to know the theme's setter.

```ds
const ThemeContext = createContext("light")

fn ThemeToggle() ReactNode {
  const theme = useContext(ThemeContext)
  return <button type="button">{theme}</button>
}
```

If a page renders `ThemeToggle` outside the provider, the bug is in the page, not in the default. Silent fallbacks are how dark mode "works" in tests and fails in production.

The Vite subject app uses the same shape with React 19's `createContext`. Same tree, same rule, different file extension.

