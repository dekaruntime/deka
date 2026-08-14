# Shared utility-CSS generator

This directory holds the single implementation of deka's Tailwind-style utility
CSS scanner/generator.

## Files

- `index.ts` — TypeScript implementation. Scans HTML for used classes and emits
  only the matching CSS.
- `registry.json` — Single source of truth for scales, utility rules, variants,
  and preflight.
- `bundle.js` — IIFE bundle generated from `index.ts` for embedding in the Rust
  runtime via `include_str!`.
- `build.ts` — Bun script that regenerates `bundle.js`.

## Consumers

1. **Server runtime** (`crates/http/src/utility_css.rs`) loads `bundle.js` into
   a `deno_core` isolate and calls `globalThis.__dekaGenerateUtilityCss`.
2. **Browser tour** (`website-deploy/lib/phpx-tour/utility-css.ts`) imports
   `index.ts` and `registry.json` directly.

## Updating

After editing `index.ts` or `registry.json`:

```bash
cd /Users/sami/Projects/deka/runtime/assets/utility-css
bun build.ts
```

Then sync the source files to the website:

```bash
cd /Users/sami/Projects/website-deploy
bun scripts/sync-utility-css.ts
```

Finally rebuild and test both the runtime and the website.
