# deka-bench — phase 2

**PRELIMINARY — pending one clean idle-host rerun.** Numbers were measured on a shared host with the contention guard (**10 waits / 8 discards**). The guard mitigates compiler contention; it does not make this an isolated run. Ava schedules the clean idle-host rerun before homepage use. No benchmark rerun was performed for this review fix.

One 28-post blog, three idiomatic implementations for [deka#938](https://github.com/dekaruntime/deka/issues/938): Deka app router + hydrated islands, Vite + React CSR with lazy routes, and Next.js App Router with SSG + client components. The fairness bar from [#944](https://github.com/dekaruntime/deka/pull/944) applies: both payload stories are separate, clearly labeled claims; no zero-JS-vs-CSR headline masquerading as an interactivity comparison.

## Reproduce

Prerequisites: Node **22+** (built-in WebSocket), npm, Chrome/Chromium, and the Deka toolchain below. macOS and Linux discovery are supported; `CHROME_BIN=/path/to/chrome` overrides browser discovery. The runner uses Node/platform APIs and keeps the #944 `machine()` guards (`sw_vers`/`sysctl` only on macOS; `/proc` plus `lsb_release` or `/etc/os-release` on Linux). No browser tool or npm package is required outside the subject apps.

```sh
# One-time toolchain setup, repo root; release build only.
cargo build --release -p cli --features dev-server
mkdir -p bench/.toolchain
cp target/release/cli bench/.toolchain/deka
cp "$HOME/.deka/bin/dsc" bench/.toolchain/dsc # released dsc 0.52.2

# All measurements, sequentially, with real browser assertions:
node bench/run.mjs
```

The runner ingests shared markdown into all three apps, runs `npm ci` inside `vite-blog` / `next-blog` only if dependencies are absent, builds and serves each stack in isolation, checks production browser behavior, measures the static variants, then runs dev-server component updates. Toolchain preparation and npm installation are untimed. Close other builds and benchmarks before measuring. On macOS/Linux, the runner also checks `ps` at each timed sample boundary and every 500 ms during a sample for external cargo/rustc/clang/linker/Deka compiler processes, excluding its own process tree and ancestors. It waits while they are active, discards overlapping attempts, and retries until five (or 25 for TTFB) accepted samples exist. Rejection depends only on detected process overlap, never on measured duration. JSON preserves waits and discarded values; none are silently removed. This is a compiler-contention check, not an OS reservation: it does not detect every background workload or guarantee detection of processes shorter than the polling interval. Monitoring has the same small overhead for all stacks. It never pauses other work; unsupported platforms report the guard as unsupported. Ports 8760–8764 (production) and 8770/8771/8773 (dev) must be free; Chrome chooses its own port. Missing prerequisites, HTTP errors, missing browser updates, or widget failures fail the command. Samples are never fabricated or silently skipped.

Output: the full markdown table on stdout, `last-results.json` (ignored), and committed audit artifacts under [`results/`](results/phase2.md): every sample, CDP clock calibration and reload flag, observed resource URLs, raw + gzip JS sizes, exact commands/versions/binary hashes, and complete production build logs. Results are one local run, not a cross-machine claim or a statistical confidence interval. The source commit in the JSON is the runner's starting HEAD; the final PR diff contains the benchmark changes measured on top of it.

## Subject apps and pins

- `content/posts/*.md`: the same 28 posts, shared syntax highlighting in `lib/ingest.mjs` / `lib/markdown.mjs`, shared SVG images. Ingest is untimed.
- `deka-blog`: existing Deka app-router blog, Theme + Newsletter compiled from DSX and hydrated with `client:load`. A **release-mode main-build CLI**, not released deka 0.52.0; dsc **0.52.2**. The CLI's verbose git SHA and SHA-256 are recorded. Never substitute the older released CLI that lacks islands hydration.
- `vite-blog`: existing idiomatic React **19.1.1**, Vite **6.3.5**, React Router **7.6.2**, with `React.lazy` route splitting. Existing npm pins unchanged.
- `next-blog`: Next.js **16.3.5**, React / React DOM **19.3.0**, exact pins and committed npm lockfile. These were npm's current stable releases when this lane was implemented. Next App Router can use its bundled React implementation internally; the installed package pins do not override that. Default Turbopack production build and development server; no compiler, minification, type-checking, or prefetch shortcuts.

This PR's Deka CLI includes the fixes needed to run the real app-router update
path: generated HTML/fragment Content-Type headers, a full-reload fallback for
server-only components without a registered browser refresh family, and stale
isolate eviction before notifying browsers. These are framework fixes, not a
benchmark-injected HMR client. Deka's document PostCard updates therefore may
reload; the table reports that explicitly. The numbers measure this PR's CLI,
not an unchanged or released main binary.

All apps have `/`, `/page/2…`, `/posts/:slug`, `/tags/:tag`, `/about`, post cards, shared nav/footer, theme toggle, newsletter, and identical rendered markdown. SVGs remain regular images in markdown, with no client highlighter or image-library tax. Next uses `next/link`; Vite uses React Router; Deka uses normal anchors. Next's Theme provider takes server-rendered children: it does **not** turn all posts into client components. `PostCard` stays a Server Component in Next and a document component in Deka; it stays client-rendered in Vite.

Next uses [`generateStaticParams`](https://nextjs.org/docs/app/api-reference/functions/generate-static-params) for the local content's complete route set, with `dynamicParams = false`. It prerenders the blog at build time and uses default output served by [`next start`](https://nextjs.org/docs/app/guides/self-hosting). No `output: export`, forced SSR, timed ISR regeneration, custom server, or artificial zero-JS stripping. Build-time SSG is appropriate for markdown committed with the site; there is no live CMS to revalidate. Full build logs disclose generated routes and framework overhead. The three build commands do different amounts of framework work by design.

The Deka theme binds `theme` in typed effect code before entering `unsafe`, so Deka can infer its reactive dependency, as a workaround for the released compiler's missed dependency inside raw `unsafe` code, tracked in [#951](https://github.com/dekaruntime/deka/issues/951). The runner checks both the actual page theme/background and newsletter state. It does not change the compiler.

## Measurements

### Builds — median of 5 each

Cold: remove Deka `dist`, `.cache`, `ds_modules/.cache`, staging output; Vite `dist` and `node_modules/.vite`; Next `.next` (including persistent compiler caches). Dependencies and OS disk caches remain warm. This is **application-output/compiler-cache cold**, not OS-reboot cold. Build command process startup is included; npm wrapper startup is excluded for Vite/Next by invoking their installed CLI files with Node.

Incremental: retain each framework's caches and change the same JSX text literal in `PostCard`: `Read post` → `Read post 1 attempt 0` → … → `Read post 5 attempt 0`, then run the production build again. Retries change the attempt suffix too, so a discarded attempt cannot turn the next rebuild into a no-op. This invalidates content hashes, unlike an mtime-only touch. Next's default production cache policy is retained; “incremental” means a retained-output rebuild after an edit, not a promise that a framework reuses work. Source is restored in `finally`; a baseline rebuild after restoration is untimed, before production serving. No generated-content ingest is inside these samples.

### Payloads — SAME post, two stories

Both stories use **`/posts/islands-are-a-budget`**. They are fresh-browser, cache-disabled initial visits, including initial lazy imports and default prefetch activity through one second of network quiet. CDP `Network.responseReceived` / `loadingFinished` / `getResponseBody` supply the actual requested bodies. No guessed import-graph regex, hand-picked chunks, or ignored runtime scripts.

- **A — no application client islands.** Deka is a temporary copy with `client:load` removed: real no-client-JS HTML. Next is a temporary copy whose Theme / Newsletter modules are static Server Component markup; it retains `next/link`, App Router, React runtime, and default prefetch behavior. **Next's static-page framework JS floor is not zero.** Vite remains the same CSR SPA, including its widgets, because CSR is its honest counterpart; this is not a feature-equivalent hydration comparison.
- **B — real interactivity.** The committed apps: Deka HTML + shared React/islands runtime; Next prerendered HTML + RSC + Theme/Newsletter client components; Vite lazy-route CSR. The full shared runtime is charged on a fresh visit, not discounted as already cached. This is the like-for-like feature story.

HTML (including inline JS/RSC), external JS, CSS, and separately requested RSC are gzip-normalized with Node's default `gzipSync`, one body at a time. Totals include default Next RSC prefetches observed during the visit; the JSON lists each URL, so speculative work is visible. Images/favicon are excluded from framework payload and listed separately. Headers, TLS and compression negotiation are excluded. These are comparable normalized body sizes, **not claimed actual wire bytes**. Repeated requests count as transfers. Raw JS bytes are also in JSON. The phase-1 payload numbers used a different URL pair and regex discovery, so do not splice those values into this table.

### TTFB — median of 25 warm local requests

Deka serves the verified `dist/server/serve-entry.js` artifact (not source posture); Next serves its prerendered build, and Vite serves `dist` through preview. Measure the interactive production post, sequentially, after five unmeasured warmups, plus an unmeasured request immediately before each sample to rewarm the socket after any contention wait. Node HTTP/1.1 on loopback with one persistent keep-alive socket, `Accept-Encoding: identity`; start at request dispatch, stop when complete response headers arrive, drain the body before the next request. This is a response-header TTFB proxy, not a socket-level first-byte timestamp. No DNS, TLS, CDN, server startup, fresh-connection latency, or Internet latency. The JSON retains all 25 samples. Vite's preview server is a local production-output fixture, not a deployment recommendation.

```sh
# After production builds (each command in its app directory):
(cd bench/deka-blog && ../.toolchain/deka serve dist/server/serve-entry.js --port 8760 --no-prompt)
(cd bench/vite-blog && node node_modules/vite/bin/vite.js preview --host 127.0.0.1 --port 8761 --strictPort)
(cd bench/next-blog && node node_modules/next/dist/bin/next start --hostname 127.0.0.1 --port 8763)
```

### Component update / HMR — median of 5, CDP timestamps

All three dev servers open `/` in headless Chromium. Production outputs/compiler caches are removed before dev startup, including Deka’s `ds_modules/.cache`; this avoids serving a production artifact from `deka dev`. After initial route load and 1.5 seconds for socket setup, apply the **identical JSX text-literal edit to PostCard** used for incremental builds. No terminal “compiled” timestamp, WebSocket receipt, or file-watcher acknowledgment is the endpoint. A `MutationObserver` writes a `performance.mark` when the matching DOM text appears; CDP `Tracing` (`blink.user_timing`) provides the mark's monotonic timestamp, stable across reloads. Polling retrieves the already-recorded timestamp; polling time is not counted. This is DOM update applied, not next paint.

The host timestamps synchronous file-write start/completion with its monotonic performance clock. Seven CDP `Performance.getMetrics` (`Timestamp`) probes map it to Chromium’s monotonic clock using the minimum-round-trip midpoint; JSON records the offset, round-trip uncertainty, save bracket, and applied timestamp. Latency is applied minus write completion. The clock alignment uncertainty is up to half the recorded probe RTT, plus scheduling asymmetry; do not interpret sub-millisecond differences as significant. Writes use the same filesystem operation on all stacks.

The observer is also installed with `Page.addScriptToEvaluateOnNewDocument`, so a framework-triggered full reload is measured and flagged by changed `performance.timeOrigin`. No harness-triggered reload is substituted for HMR. The table discloses the count of full reloads: a document reload is not advertised as state-preserving Fast Refresh. Next's idiomatic Server Component update path, Vite React HMR and Deka's actual `deka dev` path are measured as shipped, without moving PostCard into an artificial island to improve one column.

```sh
(cd bench/deka-blog && ../.toolchain/deka dev . --port 8770 --no-prompt)
(cd bench/vite-blog && node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 8771 --strictPort)
(cd bench/next-blog && node node_modules/next/dist/bin/next dev --hostname 127.0.0.1 --port 8773)
```

The broader #938 content-edit HMR metric remains follow-up work; this phase's requested edit shape is the PostCard JSX literal. No content-edit number is inferred from the component samples.

## Results — PRELIMINARY

See the [full table and machine/toolchain stanza](results/phase2.md), [raw samples and resource audit](results/phase2.json), and [production build logs](results/build-logs.json). All numbers there come from the completed one-command run in this lane. Do not publish a chart without its machine, versions, payload-story label, and reload disclosure.

Final run on 2026-09-13 (macOS 26.6.1, Intel i9-10910, 128 GiB, Node 26.3.1):

| stack | cold build | incremental build | warm TTFB | component update | full reloads |
| --- | ---: | ---: | ---: | ---: | ---: |
| deka | 1016.09 ms | 1032.39 ms | 0.88 ms | 110.43 ms | 5/5 |
| vite | 1096.03 ms | 1078.04 ms | 0.51 ms | 147.26 ms | 0/5 |
| next | 5129.07 ms | 3265.59 ms | 1.00 ms | 52.21 ms | 0/5 |

Both payload stories and every resource are in the linked results. This run recorded 10 contention waits and 8 discarded overlapping attempts; accepted medians contain exactly 5 build/update or 25 TTFB samples. No other lane was paused.

### Payload after #954 minification (from the #957 run)

deka: per-page HTML + cached shared islands runtime (production React + Theme + Newsletter). Vite: CSR SPA. This is not a "27x smaller" claim.

| stack | HTML gzip | JS gzip | CSS gzip | total gzip |
| --- | ---: | ---: | ---: | ---: |
| deka (HTML + shared islands runtime) | 1465 B | 60729 B | 1170 B | 63364 B |
| Vite + React 19.1.1 (CSR SPA) | 293 B | 84396 B | 1170 B | 85859 B |
| Next.js App Router | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) | TODO (phase 2) |

Chromium (toggle + submit) passed on both stacks: theme `light→dark`, newsletter status `Thanks — we will not actually email ava@deka.gg.`
