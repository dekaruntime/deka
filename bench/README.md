# deka-bench (phase 1)

Fair-play subject apps for [deka#938](https://github.com/dekaruntime/deka/issues/938): one blog, implemented in **deka** and **Vite + React 19**, plus a runner that times production builds and measures post-page payload.

A rigged benchmark is worse than none. The methodology is the product.

## What this lane ships

| piece | role |
| --- | --- |
| `content/posts/*.md` | 28 real posts (code, headings, images, tags) — source of truth |
| `content/images/` | shared SVGs referenced by the posts |
| `deka-blog/` | DekaScript app-router blog on **released** `deka 0.52.0` + `dsc 0.52.2` |
| `vite-blog/` | idiomatic Vite + React 19.1.1 + React Router 7 (code-split routes) |
| `run.mjs` | one command: ingest → cold/incremental builds (median of 5) → payload |
| Next.js App Router | **not in this lane** |

## Toolchain (pinned)

Install the **released** binaries. Do not build deka/dsc from this tree for the bench.

```bash
curl -fsSL https://deka.gg/install.sh | DEKA_VERSION=v0.52.0 DSC_VERSION=v0.52.2 sh
```

Or download `deka-darwin-*` / `dsc-darwin-*` from the GitHub releases and put them on `PATH`.

Vite-side pins live in `vite-blog/package.json` (`react@19.1.1`, `vite@6.3.5`, `react-router-dom@7.6.2`). npm is allowed **only** inside `vite-blog/`.

## Reproduce

From the repo root, with `deka`, `dsc`, `node`, and `npm` on `PATH`:

```bash
node bench/run.mjs
```

That command:

1. Ingests markdown (untimed) into `deka-blog/src/posts.generated.ds` and `vite-blog/src/posts.generated.ts`.
2. `npm install`s the Vite app if `node_modules` is missing (untimed).
3. Times **cold** production builds (empty `dist/` + compiler caches; dependencies already installed).
4. Times **incremental** production builds (touch one component file, rebuild).
5. Serves deka from the built artifact (`deka serve`) and Vite from `vite-blog/dist/` via a local static server.
6. Fetches `/posts/zero-js-by-default` and sums gzipped HTML+JS+CSS actually linked from that document.

It prints a markdown table and writes `bench/last-results.json`.

## What is measured

| metric | how |
| --- | --- |
| cold production build | median of 5 wall-clock `deka build` / `vite build` after wiping output caches |
| incremental production build | median of 5 after touching `PostCard` |
| payload | gzipped HTML+JS+CSS for one post page, over the local server |

Cold does **not** include `npm install` or downloading deka. Incremental does **not** wipe caches.

## What is not measured yet

These cells stay TODO until phase 2. The runner does not stub them.

| metric | status |
| --- | --- |
| HMR, component edit | TODO — CDP timestamps |
| HMR, content edit | TODO — CDP timestamps |
| TTFB | TODO — production server |
| Next.js App Router subject app | TODO |

## Fair-play notes

- Identical routes: `/`, `/page/2…`, `/posts/:slug`, `/tags/:tag`, `/about`.
- Identical widgets: theme toggle + newsletter form. Everything else is documents.
- Syntax highlighting runs in the shared ingest so neither column pays a client highlighter tax the other does not.
- Images are content. Their bytes are not folded into the framework payload column.
- Deka 0.52's production isolate does not ship React's renderer. `deka-blog/src/ui/index.ds` installs `renderToString` through a module-scope `unsafe` block so the generated serve entry can paint HTML. JSX lives in `src/ui/*.dsx` so the artifact graph's relative `jsxRuntime` specifier closes. That is a 0.52 artifact constraint, documented here rather than hidden.
- Vite is a production SPA with `React.lazy` routes, not a dev server and not a deliberately unsplit bundle.

## Machine

The runner records OS, arch, CPU, memory, and toolchain versions. Copy that stanza onto any published chart. The issue names samis-imac; this clone records whatever machine actually ran `node bench/run.mjs`.

## Local serve (manual)

```bash
# after a build
(cd bench/deka-blog && deka serve --port 3000 --no-prompt)
(cd bench/vite-blog && npx vite preview --port 4173)
```
