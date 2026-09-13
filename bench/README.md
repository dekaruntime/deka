# deka-bench (phase 1)

Fair-play subject apps for [deka#938](https://github.com/dekaruntime/deka/issues/938): one blog, implemented in **deka** and **Vite + React 19**, plus a runner that times production builds and measures two payload stories.

A rigged benchmark is worse than none. The methodology is the product. This refit follows the #944 ruling: the like-for-like row is hydrated islands vs a CSR SPA. A page with no islands (deka ships no client JS) is a separate, labeled story — not a substitute for the islands row.

## What this lane ships

| piece | role |
| --- | --- |
| `content/posts/*.md` | 28 real posts (code, headings, images, tags) — source of truth |
| `content/images/` | shared SVGs referenced by the posts |
| `deka-blog/` | DekaScript app-router blog. Theme + Newsletter are real `client:load` islands hydrated by the #948 pipeline |
| `vite-blog/` | idiomatic Vite + React 19.1.1 + React Router 7 (code-split routes) |
| `.toolchain/` | **main-build** `deka` CLI (not yet a GitHub release) plus released `dsc`. See below |
| `run.mjs` | one command: ingest → cold/incremental builds (median of 5) → two payload stories → Chromium toggle/submit |
| Next.js App Router | **not in this lane** |

## Toolchain

Islands hydration (#948) is on `main` and is **not** in a released `deka` yet. Deka rows therefore run on a CLI built **once** from `main` into `bench/.toolchain/`. Vite rows are unchanged (pinned npm). Versions and the git commit of that binary are recorded in the results stanza (`deka --version --verbose`).

```bash
# released compiler (still required)
curl -fsSL https://deka.gg/install.sh | DSC_VERSION=v0.52.2 sh

# deka CLI from this tree after merging main (release build only)
cargo build --release -p cli
mkdir -p bench/.toolchain
cp target/release/cli bench/.toolchain/deka
cp "$HOME/.deka/bin/dsc" bench/.toolchain/dsc
```

Do not point the runner at `~/.deka/bin/deka` v0.52.0 — that release predates the islands pipeline. The binaries under `.toolchain/` are gitignored.

Vite-side pins live in `vite-blog/package.json` (`react@19.1.1`, `vite@6.3.5`, `react-router-dom@7.6.2`). npm is allowed **only** inside `vite-blog/`.

## Reproduce

From the repo root, with `node`, `npm`, and `bench/.toolchain/{deka,dsc}` in place:

```bash
node bench/run.mjs
```

That command:

1. Ingests markdown (untimed) into `deka-blog/src/posts.generated.ds` and `vite-blog/src/posts.generated.ts`.
2. `npm install`s the Vite app if `node_modules` is missing (untimed).
3. Times **cold** production builds (empty `dist/` + compiler caches; dependencies already installed).
4. Times **incremental** production builds (touch one component file, rebuild).
5. Serves the deka islands app and measures `/posts/islands-are-a-budget` (HTML + shared `/assets/islands.js` runtime).
6. Checks Chromium: theme toggle flips `data-theme`, newsletter submit renders the compiled status text. Same check against the Vite SPA.
7. Copies the deka app, strips every `client:*` directive, serves that copy, and measures `/posts/zero-js-by-default` — deka ships no client JS on that document.
8. Serves Vite from `vite-blog/dist/` and measures both URLs (CSR SPA on every route).

It prints a markdown table and writes `bench/last-results.json`.

`machine()` is platform-guarded: `sw_vers` / `sysctl` on darwin, `/proc` + `lsb_release` (falling back to `/etc/os-release`) on linux. Spawn failures are returned, not thrown, so a missing macOS binary cannot crash the run on Linux.

## What is measured

| metric | how |
| --- | --- |
| cold production build | median of 5 wall-clock `deka build` / `vite build` after wiping output caches |
| incremental production build | median of 5 after touching `PostCard` |
| payload story A | gzipped HTML+JS+CSS for `/posts/zero-js-by-default` with **no** `client:*` islands. Deka ships no client JS. Label it exactly that. |
| payload story B | gzipped HTML+JS+CSS for `/posts/islands-are-a-budget` **with** Theme + Newsletter islands. Deka is per-page HTML + one cached shared runtime; Vite is a CSR SPA. Report the bytes. Do not call this "27x smaller". |

Cold does **not** include `npm install` or compiling the toolchain. Incremental does **not** wipe caches.

Story A is produced from a temporary copy of `deka-blog` whose `Shell.dsx` has every `client:load` stripped. The subject app itself keeps the islands — that copy exists only so the zero-JS row is a real document, not HTML with the islands script ignored. Story B is the subject app as committed.

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
- Those widgets are compiled DSX components marked `client:load` and hydrated through `@js/react-dom/client` `hydrateRoot` (the #948 pipeline). There is no `public/islands.js` and no hand-written vanilla shadow implementation.
- Syntax highlighting runs in the shared ingest so neither column pays a client highlighter tax the other does not.
- Images are content. Their bytes are not folded into the framework payload column.
- Vite is a production SPA with `React.lazy` routes, not a dev server and not a deliberately unsplit bundle.
- The two payload tables are different claims. Mixing them, or leading with story A as if it were islands-vs-SPA, is the #944 BLOCK.

## Machine

The runner records OS, arch, CPU, memory, and toolchain versions. Copy that stanza onto any published chart. The issue names samis-imac; this clone records whatever machine actually ran `node bench/run.mjs`.

## Local serve (manual)

```bash
# after a build, using the main-build CLI
(cd bench/deka-blog && ../.toolchain/deka serve --port 3000 --no-prompt)
(cd bench/vite-blog && npx vite preview --port 4173)
```

## Results from this clone

Generated: 2026-09-13T19:40:17.251Z

Machine: macOS 26.6.1, x86_64, Intel(R) Core(TM) i9-10910 CPU @ 3.60GHz, 128 GiB, node v26.3.1.

deka is a **main-build** (`git_sha e6bb240c1f0b`, crate version still prints 0.52.0, `react: 19.1.1`) pending the next release. dsc is released **0.52.2**. Vite is `react@19.1.1` / `vite@6.3.5` / `react-router-dom@7.6.2`.

### Build (median of 5, production)

| stack | cold build | incremental build |
| --- | ---: | ---: |
| deka | 707 ms | 481 ms |
| Vite + React 19.1.1 | 1530 ms | 1424 ms |
| Next.js App Router | TODO (phase 2) | TODO (phase 2) |

### Payload story A — zero-JS static page (`/posts/zero-js-by-default`)

deka ships no client JS. Vite is a CSR SPA on the same URL.

| stack | HTML gzip | JS gzip | CSS gzip | total gzip |
| --- | ---: | ---: | ---: | ---: |
| deka (no islands, no client JS) | 1638 B | 0 B | 1170 B | 2808 B |
| Vite + React 19.1.1 (CSR SPA) | 293 B | 84396 B | 1170 B | 85859 B |
| Next.js App Router | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) |

### Payload story B — hydrated islands (`/posts/islands-are-a-budget`)

deka: per-page HTML + cached shared islands runtime (production React + Theme + Newsletter). Vite: CSR SPA. This is not a "27x smaller" claim.

| stack | HTML gzip | JS gzip | CSS gzip | total gzip |
| --- | ---: | ---: | ---: | ---: |
| deka (HTML + shared islands runtime) | 1467 B | 101390 B | 1170 B | 104027 B |
| Vite + React 19.1.1 (CSR SPA) | 293 B | 84396 B | 1170 B | 85859 B |
| Next.js App Router | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) |

Chromium (toggle + submit) passed on both stacks: theme `light→dark`, newsletter status `Thanks — we will not actually email ava@deka.gg.`
