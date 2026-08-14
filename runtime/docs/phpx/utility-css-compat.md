---
title: "Utility CSS Compatibility"
section: "phpx"
category: "styling"
categoryLabel: "Styling"
categoryOrder: 70
---

# Utility CSS Compatibility (Runtime)

Deka ships its own Tailwind-style utility CSS scanner/generator. The same
TypeScript implementation and JSON registry run in both the browser tour preview
and the server-side runtime, so preview and production cannot drift.

## Source of truth

- `runtime/assets/utility-css/index.ts` — scanner/generator implementation.
- `runtime/assets/utility-css/registry.json` — scales, utility rules, variants,
  and preflight.
- `runtime/assets/utility-css/bundle.js` — IIFE bundle embedded into the Rust
  runtime via `include_str!`.

## Config

Create `deka.css.json` in the project root:

```json
{
  "utility": {
    "enabled": true,
    "preflight": false
  }
}
```

## Supported Variants

- Pseudo-classes: `hover:`, `focus:`, `active:`, `disabled:`, `visited:`,
  `first:`, `last:`, `odd:`, `even:`
- Dark mode: `dark:`
- Responsive: `sm:`, `md:`, `lg:`, `xl:`, `2xl:`

## Supported Utilities

The registry covers the commonly used Tailwind-style surface:

- **Layout/display**: `block`, `inline-block`, `inline`, `flex`,
  `inline-flex`, `grid`, `inline-grid`, `hidden`, `contents`
- **Flexbox/grid**: `flex-row*`, `flex-col*`, `flex-wrap*`, `items-*`,
  `justify-*`, `self-*`, `flex-1`, `flex-auto`, `grow`, `shrink`, `basis-*`,
  `gap-*`, `gap-x-*`, `gap-y-*`, `grid-cols-*`, `grid-rows-*`, `col-span-*`,
  `row-span-*`
- **Spacing**: `p-*`, `px-*`, `py-*`, `pt-*`, `pr-*`, `pb-*`, `pl-*`,
  `m-*`, `mx-*`, `my-*`, `mt-*`, `mb-*`, `ml-*`, `mr-*`, `space-x-*`,
  `space-y-*`
- **Sizing**: `w-*`, `h-*`, `min-w-*`, `min-h-*`, `max-w-*`, `max-h-*`
- **Typography**: `text-*` (color or size), `font-*`, `font-sans`,
  `font-mono`, `font-serif`, `tracking-*`, `leading-*`, `uppercase`,
  `lowercase`, `capitalize`, `underline`, `line-through`, `text-left`,
  `text-center`, `text-right`
- **Surface/border**: `bg-*`, `rounded-*`, `border`, `border-*`,
  `border-t/r/b/l-*`, `border-x/y-*`, `border-{color}`, `shadow-*`,
  `opacity-*`
- **Effects/transitions**: `transition-*`, `duration-*`, `ease-*`, `delay-*`
- **Positioning**: `relative`, `absolute`, `fixed`, `sticky`, `inset-*`,
  `top-*`, `right-*`, `bottom-*`, `left-*`, `z-*`
- **Misc**: `cursor-*`, `overflow-*`, `whitespace-*`, `break-words`,
  `break-all`, `object-*`, `visible`, `invisible`, `list-none`, `list-disc`,
  `list-decimal`, `sr-only`

Arbitrary values such as `w-[100px]` and `grid-cols-[1fr_2fr]` are supported.

## Behavior Notes

- CSS is emitted from classes found in server-rendered HTML and injected into
  `<head>` as `#__deka_utility_css`.
- Unknown classes are ignored without throwing.
- To add or tweak utilities, edit `runtime/assets/utility-css/registry.json`,
  run `bun build.ts` in that directory, then sync to the website with
  `bun scripts/sync-utility-css.ts`.
