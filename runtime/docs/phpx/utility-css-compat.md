---
title: "Utility CSS Compatibility"
section: "phpx"
category: "styling"
categoryLabel: "Styling"
categoryOrder: 70
---

# Utility CSS Compatibility (Runtime)

This documents the current runtime utility CSS surface used by `crates/http/src/utility_css.rs`.

## Config

Create `deka.css.json` in project root:

```json
{
  "utility": {
    "enabled": true,
    "preflight": false
  }
}
```

## Supported Variants

- `hover:`
- `focus:`
- `active:`
- `dark:`
- `sm:`
- `md:`
- `lg:`
- `xl:`

## Supported Utilities (Current)

- Layout/display: `block`, `inline-block`, `flex`, `grid`, `hidden`
- Flex/grid alignment: `items-*`, `justify-*`, `flex-wrap`, `grid-cols-*`, `gap-*`
- Sizing: `w-full`, `h-full`, `min-h-screen`, `max-w-6xl`
- Spacing: `p-*`, `px-*`, `py-*`, `pt-*`, `mt-*`, `mb-*`, `mx-auto`
- Typography: `uppercase`, `font-semibold`, `font-bold`, `font-mono`, `text-*`, `tracking-wide`
- Surface/border: `rounded*`, `shadow-*`, `border`, `border-b`
- Misc: `whitespace-pre-wrap`, `transition-shadow`
- Colors: selected `bg-*`, `text-*`, `border-*` palette currently implemented in code

## Unsupported (for now)

- Arbitrary values beyond the currently implemented parser surface.
- Full Tailwind plugin ecosystem.
- Full Tailwind config parity (`theme.extend`, plugin transforms, etc.).
- Non-implemented utility families (incremental expansion expected).

## Behavior Notes

- CSS is emitted from classes found in server-rendered HTML and injected into `<head>` as `#__deka_utility_css`.
- Generated CSS is cached in project `.cache/utility-css/`.
- Unknown classes are ignored without throwing.
