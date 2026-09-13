export const PAGE_SIZE = 6 as const;
export type Post = {
  slug: string;
  title: string;
  date: string;
  excerpt: string;
  tags: string[];
  html: string;
};
export const posts: Post[] = [
  {
    "slug": "the-subject-app-is-the-spec",
    "title": "The subject app is the spec",
    "date": "2026-05-11",
    "excerpt": "If a feature cannot be expressed in the blog, it is not in the comparison. The app is the contract.",
    "tags": [
      "language-design",
      "web",
      "runtime"
    ],
    "html": "<p>Issue 938 lists the floor: shared markdown, paginated index, tag pages, about, layout, cards, highlighted code, a theme island, a newsletter island, images.</p>\n<p>This directory is that list, implemented twice. When we add Next.js, we will not \"simplify the subject\" to make RSC look good. We will implement the same list a third time.</p>\n<pre class=\"code\"><code class=\"language-txt\"><span class=\"tok-id\">bench</span>/<span class=\"tok-id\">content</span>/<span class=\"tok-id\">posts</span>/*.<span class=\"tok-id\">md</span>\n<span class=\"tok-id\">bench</span>/<span class=\"tok-id\">deka-blog</span>/\n<span class=\"tok-id\">bench</span>/<span class=\"tok-id\">vite-blog</span>/\n<span class=\"tok-id\">bench</span>/<span class=\"tok-id\">run</span>.<span class=\"tok-id\">mjs</span></code></pre>\n<p>A missing page is a missing measurement. A bonus widget is a tainted one. Stay on the list.</p>"
  },
  {
    "slug": "reproducing-on-another-machine",
    "title": "Reproducing on another machine",
    "date": "2026-05-06",
    "excerpt": "If the README's machine block is empty, the number is a souvenir. Fill it from the runner, not from memory.",
    "tags": [
      "tooling",
      "runtime"
    ],
    "html": "<p>The runner prints a machine stanza:</p>\n<ul><li>OS and version</li><li>architecture</li><li>CPU brand</li><li>memory</li><li><code>deka --version</code> and <code>dsc --version</code></li><li><code>node --version</code></li></ul>\n<p>Copy that stanza into any published chart. If you rerun on a laptop that is also compiling a kernel, say so. Isolation is part of fair play, not a vibe.</p>\n<pre class=\"code\"><code class=\"language-txt\"><span class=\"tok-id\">os</span>: <span class=\"tok-id\">macOS</span> <span class=\"tok-num\">26.6</span>.<span class=\"tok-num\">1</span>\n<span class=\"tok-id\">arch</span>: <span class=\"tok-id\">x86_64</span>\n<span class=\"tok-id\">cpu</span>: <span class=\"tok-id\">Intel</span>(<span class=\"tok-id\">R</span>) <span class=\"tok-id\">Core</span>(<span class=\"tok-id\">TM</span>) <span class=\"tok-id\">i9-10910</span> <span class=\"tok-id\">CPU</span> @ <span class=\"tok-num\">3</span>.60<span class=\"tok-id\">GHz</span></code></pre>\n<p>samis-imac is the machine named in the issue. This clone may be that machine or it may not. The runner does not guess; it records.</p>"
  },
  {
    "slug": "css-is-a-document",
    "title": "CSS is a document",
    "date": "2026-05-01",
    "excerpt": "One stylesheet, same rules, both apps. Utility soup in one column would be a costume change.",
    "tags": [
      "web",
      "performance"
    ],
    "html": "<p>Both subject apps load <code>/style.css</code>. The file is duplicated (each app has a public root) and kept byte-identical by the ingest check in the runner.</p>\n<p>No CSS-in-JS runtime on the Vite side. No per-route CSS extraction claimed on the deka side. A later lane can measure that if someone ships it.</p>\n<pre class=\"code\"><code class=\"language-css\">:<span class=\"tok-kw\">root</span> { <span class=\"tok-kw\">color-scheme</span>: <span class=\"tok-id\">light</span> <span class=\"tok-id\">dark</span>; }\n[<span class=\"tok-id\">data-theme</span>=<span class=\"tok-str\">\"dark\"</span>] { <span class=\"tok-kw\">color-scheme</span>: <span class=\"tok-id\">dark</span>; }\n.<span class=\"tok-id\">prose</span> <span class=\"tok-id\">pre</span> { <span class=\"tok-id\">overflow</span>: <span class=\"tok-id\">auto</span>; }</code></pre>\n<p>Dark mode is a <code>data-theme</code> attribute, not a second stylesheet. The island writes the attribute; the document already knew the rules.</p>"
  },
  {
    "slug": "component-boundaries-for-refresh",
    "title": "Component boundaries for refresh",
    "date": "2026-04-26",
    "excerpt": "HMR is phase two, but the subject app still has to be refreshable or the later measurement will be a rewrite.",
    "tags": [
      "runtime",
      "javascript"
    ],
    "html": "<p>Fast Refresh cares about module shape: PascalCase components, stable hook order, no surprising exports. The deka components are written to that shape now so phase 2 is an instrument, not a redesign.</p>\n<pre class=\"code\"><code class=\"language-ds\"><span class=\"tok-kw\">export</span> <span class=\"tok-kw\">fn</span> <span class=\"tok-id\">PostCard</span>(<span class=\"tok-id\">props</span>: <span class=\"tok-id\">PostCardProps</span>) <span class=\"tok-id\">ReactNode</span> {\n  <span class=\"tok-kw\">return</span> &lt;<span class=\"tok-id\">article</span> <span class=\"tok-id\">class</span>=<span class=\"tok-str\">\"card\"</span>&gt;&lt;<span class=\"tok-id\">a</span> <span class=\"tok-id\">href</span>={<span class=\"tok-id\">props</span>.<span class=\"tok-id\">href</span>}&gt;{<span class=\"tok-id\">props</span>.<span class=\"tok-id\">title</span>}&lt;/<span class=\"tok-id\">a</span>&gt;&lt;/<span class=\"tok-id\">article</span>&gt;\n}</code></pre>\n<p>A sibling edit of <code>PostCard</code> should not reset the theme toggle's state. That sentence is the HMR spec. We do not time it in this lane; we do not write components that would make the sentence untestable.</p>"
  },
  {
    "slug": "what-we-refuse-to-count",
    "title": "What we refuse to count",
    "date": "2026-04-21",
    "excerpt": "Install time, image bytes, and font downloads are real costs. They are not this comparison.",
    "tags": [
      "performance",
      "tooling"
    ],
    "html": "<p>Out of scope for the production columns:</p>\n<ul><li><code>npm install</code> / <code>deka</code> download</li><li>the SVG images in <code>/images</code></li><li>system fonts</li><li>gzip of the runner itself</li><li>time to start the preview server (payload uses it, build does not)</li></ul>\n<p>In scope:</p>\n<ul><li><code>deka build</code> and <code>vite build</code> wall clocks</li><li>HTML+JS+CSS bytes to read <code>/posts/zero-js-by-default</code>, gzipped</li></ul>\n<p>If a future lane wants \"Lighthouse on a phone\", it should say so and pin the phone.</p>"
  },
  {
    "slug": "internal-baseline-same-harness",
    "title": "Internal baseline, same harness",
    "date": "2026-04-16",
    "excerpt": "The homepage chart and the release-train gate have to be the same command. Two harnesses will drift.",
    "tags": [
      "tooling",
      "runtime",
      "performance"
    ],
    "html": "<p>If marketing runs a laptop script and CI runs a trimmed subset, you will eventually publish a number CI cannot see regress. The runner is <code>bench/run.mjs</code>. CI can call it. A human can call it. The table format does not change.</p>\n<pre class=\"code\"><code class=\"language-bash\"><span class=\"tok-id\">node</span> <span class=\"tok-id\">bench</span>/<span class=\"tok-id\">run</span>.<span class=\"tok-id\">mjs</span></code></pre>\n<p>That is the whole interface. Flags can be added later for <code>--only=vite</code> when iterating. The default is both apps, both clocks, one payload URL.</p>"
  },
  {
    "slug": "next-is-phase-two",
    "title": "Next.js is phase two",
    "date": "2026-04-11",
    "excerpt": "A two-way chart is not a three-way chart. We will not invent Next numbers to fill a cell.",
    "tags": [
      "web",
      "tooling"
    ],
    "html": "<p>The issue names three columns: deka, Vite+React, Next.js App Router. This lane ships the first two plus the runner skeleton. The Next app, HMR, and TTFB are empty cells with an explicit TODO.</p>\n<p>An empty cell is a promise. A guessed cell is a lie.</p>\n<p>When phase 2 lands, it will:</p>\n<ul><li>add <code>bench/next-blog</code> with App Router and a production config</li><li>pin a Next version next to the others</li><li>fill HMR via CDP timestamps</li><li>fill TTFB from the production server</li></ul>\n<p>Until then the README table keeps the holes visible.</p>"
  },
  {
    "slug": "vite-react-19-is-not-a-strawman",
    "title": "Vite + React 19 is not a strawman",
    "date": "2026-04-07",
    "excerpt": "The comparison target is a senior React app: lazy routes, production build, the same React version the deka vendor pins.",
    "tags": [
      "javascript",
      "web",
      "tooling"
    ],
    "html": "<p>React 19.1.1 is the version vendored for <code>deka dev</code> Fast Refresh. The Vite app pins the same version from npm, because npm is how that ecosystem is actually consumed.</p>\n<pre class=\"code\"><code class=\"language-json\">{\n  <span class=\"tok-str\">\"dependencies\"</span>: {\n    <span class=\"tok-str\">\"react\"</span>: <span class=\"tok-str\">\"19.1.1\"</span>,\n    <span class=\"tok-str\">\"react-dom\"</span>: <span class=\"tok-str\">\"19.1.1\"</span>,\n    <span class=\"tok-str\">\"react-router-dom\"</span>: <span class=\"tok-str\">\"7.6.2\"</span>\n  }\n}</code></pre>\n<p>No <code>React.FC</code> archaeology, no CSS-in-JS runtime, no extra state library for a theme string. Context, lazy, and a CSS file. If deka wins, it should win against that, not against a 2018 CRA screenshot.</p>"
  },
  {
    "slug": "unsafe-is-a-door-not-a-room",
    "title": "unsafe is a door, not a room",
    "date": "2026-04-02",
    "excerpt": "Newsletter submit has to call preventDefault. That is a door. The form itself stays ordinary DSX.",
    "tags": [
      "language-design",
      "runtime"
    ],
    "html": "<p>Application DekaScript cannot call <code>deka.*</code>. The generated serve entry can, and does: <code>unsafe { deka.ui.renderToStreamHtml(tree) }</code>. Island event handlers that talk to the DOM use a small <code>unsafe</code> block as a door, then return to typed code.</p>\n<pre class=\"code\"><code class=\"language-ds\"><span class=\"tok-id\">onSubmit</span>={<span class=\"tok-kw\">fn</span>(<span class=\"tok-id\">event</span>: <span class=\"tok-id\">Any</span>) <span class=\"tok-id\">void</span> {\n  <span class=\"tok-kw\">const</span> <span class=\"tok-id\">_</span> = <span class=\"tok-kw\">unsafe</span> {\n    <span class=\"tok-id\">event</span>.<span class=\"tok-id\">preventDefault</span>()\n    <span class=\"tok-kw\">return</span> <span class=\"tok-num\">1</span>\n  }\n  <span class=\"tok-id\">setSent</span>(<span class=\"tok-id\">true</span>)\n}}</code></pre>\n<p>Posts, layouts, and islands stay in ordinary DSX. If the whole blog were an unsafe block, we would not be dogfooding the language.</p>"
  },
  {
    "slug": "dsx-depth-is-a-compiler-detail",
    "title": "DSX depth is a compiler detail",
    "date": "2026-03-27",
    "excerpt": "App routes stay thin. Components live in src/ui. That split is authoring hygiene, not a hidden runtime.",
    "tags": [
      "compilers",
      "language-design"
    ],
    "html": "<p>The subject app keeps JSX in <code>src/ui/<em>.dsx</code> and keeps <code>app/</em>*/page.dsx</code> as thin wrappers that call those components. Production JSX resolves from the runtime builtin <code>@js/react/jsx-runtime</code> — no <code>deka.json</code> <code>jsxRuntime</code> field, no hand-rolled renderer.</p>\n<pre class=\"code\"><code class=\"language-ds\"><span class=\"tok-kw\">import</span> { <span class=\"tok-id\">HomePage</span> } <span class=\"tok-id\">from</span> <span class=\"tok-str\">\"../src/ui/HomePage.dsx\"</span>\n<span class=\"tok-kw\">export</span> <span class=\"tok-kw\">fn</span> <span class=\"tok-id\">Page</span>() <span class=\"tok-id\">ReactNode</span> {\n  <span class=\"tok-kw\">return</span> <span class=\"tok-id\">HomePage</span>()\n}</code></pre>\n<p>Theme and Newsletter are the exceptions that opt into the client: they are the same DSX components, marked <code>client:load</code>, and the #948 pipeline hydrates them. Everything else is HTML the compiler already knew.</p>"
  },
  {
    "slug": "medians-of-five",
    "title": "Medians of five",
    "date": "2026-03-22",
    "excerpt": "A single run is an anecdote. Five runs and a median is still a small sample, but it is a sample.",
    "tags": [
      "performance",
      "tooling"
    ],
    "html": "<p>N=5 is a truce with wall-clock noise, not a statistics paper. We report the median so a thermal throttle on run 4 does not become the homepage.</p>\n<pre class=\"code\"><code class=\"language-js\"><span class=\"tok-kw\">function</span> <span class=\"tok-id\">median</span>(<span class=\"tok-id\">values</span>) {\n  <span class=\"tok-kw\">const</span> <span class=\"tok-id\">xs</span> = [...<span class=\"tok-id\">values</span>].<span class=\"tok-id\">sort</span>((<span class=\"tok-id\">a</span>, <span class=\"tok-id\">b</span>) =&gt; <span class=\"tok-id\">a</span> - <span class=\"tok-id\">b</span>);\n  <span class=\"tok-kw\">const</span> <span class=\"tok-id\">mid</span> = <span class=\"tok-id\">Math</span>.<span class=\"tok-id\">floor</span>(<span class=\"tok-id\">xs</span>.<span class=\"tok-id\">length</span> / <span class=\"tok-num\">2</span>);\n  <span class=\"tok-kw\">return</span> <span class=\"tok-id\">xs</span>.<span class=\"tok-id\">length</span> % <span class=\"tok-num\">2</span> ? <span class=\"tok-id\">xs</span>[<span class=\"tok-id\">mid</span>] : (<span class=\"tok-id\">xs</span>[<span class=\"tok-id\">mid</span> - <span class=\"tok-num\">1</span>] + <span class=\"tok-id\">xs</span>[<span class=\"tok-id\">mid</span>]) / <span class=\"tok-num\">2</span>;\n}</code></pre>\n<p>If a run is more than twice the median, the runner prints it and still uses the median. Outliers are data. They are not the number.</p>"
  },
  {
    "slug": "the-about-page-is-a-control",
    "title": "The about page is a control",
    "date": "2026-03-18",
    "excerpt": "If the about page and a post page ship the same JS, you do not have a zero-JS story. You have a bundle.",
    "tags": [
      "web",
      "tooling"
    ],
    "html": "<p>Control routes matter. <code>/about</code> has the chrome, the theme island, the newsletter island, and no code block. A post page has all of that plus highlighted HTML.</p>\n<p>The payload table should not pretend those are the same class of page. Phase 1 reports a post page because that is the page a stranger actually reads. A later lane can split \"static page\" vs \"island page\" the way the issue asks.</p>\n<p>Until then: one representative URL, named in the README, fetched the same way for both apps.</p>"
  },
  {
    "slug": "syntax-highlighting-before-the-runtime",
    "title": "Syntax highlighting before the runtime",
    "date": "2026-03-13",
    "excerpt": "Highlighting in the client is a payload tax. Highlighting in the ingest is a compile-time color.",
    "tags": [
      "tooling",
      "javascript"
    ],
    "html": "<p>Code blocks in these posts are highlighted once, in the shared ingest, into <code>&lt;span class=\"tok-*\"&gt;</code> nodes. Both apps render the HTML as a document fragment.</p>\n<pre class=\"code\"><code class=\"language-rs\"><span class=\"tok-kw\">fn</span> <span class=\"tok-id\">find_dsc</span>() -&gt; <span class=\"tok-id\">Result</span>&lt;<span class=\"tok-id\">Option</span>&lt;<span class=\"tok-id\">PathBuf</span>&gt;, <span class=\"tok-id\">String</span>&gt; {\n    <span class=\"tok-kw\">if</span> <span class=\"tok-kw\">let</span> <span class=\"tok-id\">Ok</span>(<span class=\"tok-id\">exe</span>) = <span class=\"tok-id\">std</span>::<span class=\"tok-id\">env</span>::<span class=\"tok-id\">current_exe</span>() {\n        <span class=\"tok-kw\">if</span> <span class=\"tok-kw\">let</span> <span class=\"tok-id\">Some</span>(<span class=\"tok-id\">dir</span>) = <span class=\"tok-id\">exe</span>.<span class=\"tok-id\">parent</span>() {\n            <span class=\"tok-kw\">let</span> <span class=\"tok-id\">sibling</span> = <span class=\"tok-id\">dir</span>.<span class=\"tok-id\">join</span>(<span class=\"tok-str\">\"dsc\"</span>);\n            <span class=\"tok-kw\">if</span> <span class=\"tok-id\">sibling</span>.<span class=\"tok-id\">is_file</span>() {\n                <span class=\"tok-kw\">return</span> <span class=\"tok-id\">Ok</span>(<span class=\"tok-id\">Some</span>(<span class=\"tok-id\">sibling</span>));\n            }\n        }\n    }\n    <span class=\"tok-id\">Ok</span>(<span class=\"tok-id\">None</span>)\n}</code></pre>\n<p>A client highlighter would pull a grammar table into every post page. That is a real cost and a real choice. It is not this choice.</p>"
  },
  {
    "slug": "tag-pages-are-cheap-if-you-let-them-be",
    "title": "Tag pages are cheap if you let them be",
    "date": "2026-03-09",
    "excerpt": "A tag page is a filter over static data. It does not need a search runtime until it is a search.",
    "tags": [
      "web",
      "performance"
    ],
    "html": "<p><code>/tags/runtime</code> is a list of cards. The data was known at ingest. The honest implementation is a function from <code>tag</code> to <code>Post[]</code>.</p>\n<pre class=\"code\"><code class=\"language-ds\"><span class=\"tok-kw\">export</span> <span class=\"tok-kw\">fn</span> <span class=\"tok-id\">postsForTag</span>(<span class=\"tok-id\">tag</span>: <span class=\"tok-id\">string</span>) <span class=\"tok-id\">Array</span>&lt;<span class=\"tok-id\">Post</span>&gt; {\n  <span class=\"tok-kw\">let</span> <span class=\"tok-id\">out</span>: <span class=\"tok-id\">Array</span>&lt;<span class=\"tok-id\">Post</span>&gt; = []\n  <span class=\"tok-kw\">for</span> (<span class=\"tok-kw\">const</span> <span class=\"tok-id\">post</span> <span class=\"tok-id\">of</span> <span class=\"tok-id\">posts</span>) {\n    <span class=\"tok-kw\">if</span> (<span class=\"tok-id\">hasTag</span>(<span class=\"tok-id\">post</span>.<span class=\"tok-id\">tags</span>, <span class=\"tok-id\">tag</span>)) {\n      <span class=\"tok-id\">out</span>.<span class=\"tok-id\">push</span>(<span class=\"tok-id\">post</span>)\n    }\n  }\n  <span class=\"tok-kw\">return</span> <span class=\"tok-id\">out</span>\n}</code></pre>\n<p>Adding typeahead on top of that is a product decision and a new island. This bench does not have it, on purpose.</p>"
  },
  {
    "slug": "pagination-is-a-route",
    "title": "Pagination is a route",
    "date": "2026-03-05",
    "excerpt": "Page 2 is not a query string the client interprets. It is a document with its own URL.",
    "tags": [
      "web",
      "javascript"
    ],
    "html": "<p>A paginated index that rewrites in-place is an application. A paginated index that is <code>/</code>, <code>/page/2</code>, <code>/page/3</code> is a set of documents.</p>\n<p>Both subject apps expose the same routes. The Vite app can still be a SPA; the URLs have to exist so the payload fetch hits a real post listing.</p>\n<pre class=\"code\"><code class=\"language-txt\">/                 <span class=\"tok-id\">page</span> <span class=\"tok-num\">1</span>\n/<span class=\"tok-id\">page</span>/<span class=\"tok-num\">2</span>           <span class=\"tok-id\">page</span> <span class=\"tok-num\">2</span>\n/<span class=\"tok-id\">posts</span>/:<span class=\"tok-id\">slug</span>      <span class=\"tok-id\">a</span> <span class=\"tok-id\">post</span>\n/<span class=\"tok-id\">tags</span>/:<span class=\"tok-id\">tag</span>        <span class=\"tok-id\">a</span> <span class=\"tok-id\">tag</span>\n/<span class=\"tok-id\">about</span>            <span class=\"tok-id\">about</span></code></pre>\n<p>Twenty-eight posts at six per page is five index pages, plus the posts, plus the tags, plus about. That is the \"25+ pages\" bar, not a carousel.</p>"
  },
  {
    "slug": "released-binaries-only",
    "title": "Released binaries only",
    "date": "2026-03-01",
    "excerpt": "Homepage numbers from an unpublished binary are a preview. Record the commit. Ship the release before the homepage.",
    "tags": [
      "tooling",
      "runtime"
    ],
    "html": "<p>The methodology prefers GitHub releases. Islands hydration landed on <code>main</code> in deka#948 and is not in a released <code>deka</code> yet, so this clone's deka column runs a <strong>main-build</strong> copied to <code>bench/.toolchain/</code>, with the git sha recorded in the results stanza. Vite still uses the pinned npm release. <code>dsc</code> stays the released compiler.</p>\n<pre class=\"code\"><code class=\"language-bash\"><span class=\"tok-id\">cargo</span> <span class=\"tok-id\">build</span> --<span class=\"tok-id\">release</span> -<span class=\"tok-id\">p</span> <span class=\"tok-id\">cli</span>\n<span class=\"tok-id\">cp</span> <span class=\"tok-id\">target</span>/<span class=\"tok-id\">release</span>/<span class=\"tok-id\">cli</span> <span class=\"tok-id\">bench</span>/.<span class=\"tok-id\">toolchain</span>/<span class=\"tok-id\">deka</span></code></pre>\n<p>The next deka release replaces that pin. Until then the table says \"main-build pending the next release\" instead of pretending v0.52.0 hydrated islands.</p>"
  },
  {
    "slug": "rust-in-the-toolchain-not-the-page",
    "title": "Rust in the toolchain, not the page",
    "date": "2026-02-24",
    "excerpt": "The compiler being Rust does not make the blog a Rust app. Keep the boundary boring.",
    "tags": [
      "rust",
      "compilers",
      "runtime"
    ],
    "html": "<p>dsc is a Rust program. deka's host is a Rust program. The subject app is DekaScript that emits JavaScript. Mixing those layers in a README is how \"written in Rust\" becomes a payload claim.</p>\n<pre class=\"code\"><code class=\"language-rs\"><span class=\"tok-kw\">pub</span> <span class=\"tok-kw\">fn</span> <span class=\"tok-id\">require_dsc</span>() -&gt; <span class=\"tok-id\">Result</span>&lt;<span class=\"tok-id\">PathBuf</span>, <span class=\"tok-id\">String</span>&gt; {\n    <span class=\"tok-id\">find_cli_dsc</span>()?.<span class=\"tok-id\">ok_or_else</span>(|| {\n        <span class=\"tok-str\">\"dsc is required for deka build\"</span>.<span class=\"tok-id\">to_string</span>()\n    })\n}</code></pre>\n<p>The blog never imports that function. The runner execs the released <code>deka</code> binary, which execs the released <code>dsc</code> binary. Build times include both, because a user who types <code>deka build</code> pays for both.</p>"
  },
  {
    "slug": "code-splitting-is-table-stakes",
    "title": "Code splitting is table stakes",
    "date": "2026-02-19",
    "excerpt": "A Vite blog that ships one JS file for twenty-eight posts is a strawman. Lazy routes are the baseline.",
    "tags": [
      "web",
      "javascript",
      "performance"
    ],
    "html": "<p>The uncharitable Vite app is a single <code>main.tsx</code> that statically imports every page. Nobody shipping a real blog does that in 2026.</p>\n<pre class=\"code\"><code class=\"language-tsx\"><span class=\"tok-kw\">const</span> <span class=\"tok-id\">PostPage</span> = <span class=\"tok-id\">lazy</span>(() =&gt; <span class=\"tok-kw\">import</span>(<span class=\"tok-str\">\"./pages/PostPage\"</span>));\n<span class=\"tok-kw\">const</span> <span class=\"tok-id\">TagPage</span> = <span class=\"tok-id\">lazy</span>(() =&gt; <span class=\"tok-kw\">import</span>(<span class=\"tok-str\">\"./pages/TagPage\"</span>));\n<span class=\"tok-kw\">const</span> <span class=\"tok-id\">AboutPage</span> = <span class=\"tok-id\">lazy</span>(() =&gt; <span class=\"tok-kw\">import</span>(<span class=\"tok-str\">\"./pages/AboutPage\"</span>));</code></pre>\n<p>The post-page payload is then: the HTML shell, the shared vendor chunk, the post route chunk, and CSS. That is the number we record. If deka ships less JS, it should be because the route did not need it, not because we forgot <code>lazy()</code>.</p>"
  },
  {
    "slug": "production-configs-or-bust",
    "title": "Production configs or bust",
    "date": "2026-02-15",
    "excerpt": "If the column says production, the command must be the production command. Dev servers do not get a costume.",
    "tags": [
      "tooling",
      "performance"
    ],
    "html": "<p>A development server is allowed to be slow, chatty, and stateful. A production build is allowed to be none of those.</p>\n<p>Vite's production command here is <code>vite build</code>. Deka's is <code>deka build</code>. Preview/serve after that is how we read payload, not how we time compile.</p>\n<pre class=\"code\"><code class=\"language-ts\"><span class=\"tok-kw\">export</span> <span class=\"tok-kw\">default</span> <span class=\"tok-id\">defineConfig</span>({\n  <span class=\"tok-id\">plugins</span>: [<span class=\"tok-id\">react</span>()],\n  <span class=\"tok-id\">build</span>: {\n    <span class=\"tok-id\">target</span>: <span class=\"tok-str\">\"es2022\"</span>,\n    <span class=\"tok-id\">sourcemap</span>: <span class=\"tok-id\">false</span>,\n    <span class=\"tok-id\">modulePreload</span>: { <span class=\"tok-id\">polyfill</span>: <span class=\"tok-id\">false</span> },\n  },\n});</code></pre>\n<p>Source maps off, because we are counting bytes a user downloads, not bytes a debugger wants. The React plugin stays on, because that is how a senior Vite app is actually built.</p>"
  },
  {
    "slug": "markdown-is-the-source-of-truth",
    "title": "Markdown is the source of truth",
    "date": "2026-02-11",
    "excerpt": "Both subject apps ingest the same files. Divergent HTML is a bug in the ingest, not a framework feature.",
    "tags": [
      "tooling",
      "web"
    ],
    "html": "<p>There is a temptation to hand-write DSX posts on one side and MDX on the other and call them \"the same blog\". They will drift in a week.</p>\n<p>This bench keeps the posts in <code>bench/content/posts/</code> as markdown with front matter. A tiny ingest step — not timed — emits:</p>\n<ul><li><code>deka-blog/src/posts.generated.ds</code></li><li><code>vite-blog/src/posts.generated.ts</code></li></ul>\n<pre class=\"code\"><code class=\"language-md\">---\n<span class=\"tok-kw\">title</span>: <span class=\"tok-id\">Markdown</span> <span class=\"tok-id\">is</span> <span class=\"tok-id\">the</span> <span class=\"tok-id\">source</span> <span class=\"tok-id\">of</span> <span class=\"tok-id\">truth</span>\n<span class=\"tok-kw\">date</span>: <span class=\"tok-num\">2026</span>-<span class=\"tok-num\">02</span>-<span class=\"tok-num\">11</span>\n<span class=\"tok-kw\">tags</span>: [<span class=\"tok-id\">tooling</span>, <span class=\"tok-id\">web</span>]\n---\n\n<span class=\"tok-id\">The</span> <span class=\"tok-id\">paragraph</span> <span class=\"tok-id\">you</span> <span class=\"tok-id\">are</span> <span class=\"tok-id\">reading</span> <span class=\"tok-id\">is</span> <span class=\"tok-id\">the</span> <span class=\"tok-id\">input</span>.</code></pre>\n<p>Syntax highlighting happens in the ingest so both runtimes paint the same spans. Measuring <code>highlight.js</code> against a hand-rolled tokenizer would be a different paper.</p>"
  },
  {
    "slug": "context-is-a-tree-not-a-store",
    "title": "Context is a tree, not a store",
    "date": "2026-02-06",
    "excerpt": "useContext without a provider is a missing-requirement error, not a default you forgot to document.",
    "tags": [
      "language-design",
      "web"
    ],
    "html": "<p>A store is a process singleton. Context is a prefix of the tree. That difference is why a theme provider belongs above the layout chrome and why a newsletter form does not need to know the theme's setter.</p>\n<pre class=\"code\"><code class=\"language-ds\"><span class=\"tok-kw\">const</span> <span class=\"tok-id\">ThemeContext</span> = <span class=\"tok-kw\">createContext</span>(<span class=\"tok-str\">\"light\"</span>)\n\n<span class=\"tok-kw\">fn</span> <span class=\"tok-id\">ThemeToggle</span>() <span class=\"tok-id\">ReactNode</span> {\n  <span class=\"tok-kw\">const</span> <span class=\"tok-id\">theme</span> = <span class=\"tok-kw\">useContext</span>(<span class=\"tok-id\">ThemeContext</span>)\n  <span class=\"tok-kw\">return</span> &lt;<span class=\"tok-id\">button</span> <span class=\"tok-id\">type</span>=<span class=\"tok-str\">\"button\"</span>&gt;{<span class=\"tok-id\">theme</span>}&lt;/<span class=\"tok-id\">button</span>&gt;\n}</code></pre>\n<p>If a page renders <code>ThemeToggle</code> outside the provider, the bug is in the page, not in the default. Silent fallbacks are how dark mode \"works\" in tests and fails in production.</p>\n<p>The Vite subject app uses the same shape with React 19's <code>createContext</code>. Same tree, same rule, different file extension.</p>"
  },
  {
    "slug": "hooks-on-the-server",
    "title": "Hooks on the server are a scheduling story",
    "date": "2026-02-02",
    "excerpt": "useState during SSR is legal. useEffect during SSR is a no-op. Mixing those facts is how you leak documents.",
    "tags": [
      "runtime",
      "language-design"
    ],
    "html": "<p>The compiler-known hooks in DekaScript are not a courtesy import. They are a color on the function: a hook-typed function is only callable from a component or another hook.</p>\n<pre class=\"code\"><code class=\"language-ds\"><span class=\"tok-kw\">export</span> <span class=\"tok-kw\">fn</span> <span class=\"tok-id\">ThemeToggle</span>() <span class=\"tok-id\">ReactNode</span> {\n  <span class=\"tok-kw\">const</span> <span class=\"tok-id\">pair</span> = <span class=\"tok-kw\">useState</span>(<span class=\"tok-str\">\"light\"</span>)\n  <span class=\"tok-kw\">const</span> <span class=\"tok-id\">theme</span> = <span class=\"tok-id\">pair</span>[<span class=\"tok-num\">0</span>]\n  <span class=\"tok-kw\">useEffect</span>(<span class=\"tok-kw\">fn</span>() <span class=\"tok-id\">Option</span>&lt;<span class=\"tok-kw\">fn</span>() <span class=\"tok-id\">void</span>&gt; {\n    <span class=\"tok-kw\">return</span> <span class=\"tok-id\">None</span>\n  })\n  <span class=\"tok-kw\">return</span> &lt;<span class=\"tok-id\">button</span> <span class=\"tok-id\">type</span>=<span class=\"tok-str\">\"button\"</span> <span class=\"tok-id\">data-theme</span>={<span class=\"tok-id\">theme</span>}&gt;{<span class=\"tok-id\">theme</span>}&lt;/<span class=\"tok-id\">button</span>&gt;\n}</code></pre>\n<p>On the server, <code>useState</code> exists so the first paint has a value. <code>useEffect</code> exists so the client can subscribe after hydration. Running the effect on the server would be a second renderer pretending to be a browser.</p>\n<p>The theme island is where those two worlds meet. The document can ship <code>data-theme=\"light\"</code> without JS; the island writes the preference once a human asks. No directive, no client JS.</p>"
  },
  {
    "slug": "payload-is-a-graph",
    "title": "Payload is a graph, not a folder",
    "date": "2026-01-27",
    "excerpt": "Bytes over the wire are the document plus everything it pulls. Listing dist/ is not a payload.",
    "tags": [
      "web",
      "performance"
    ],
    "html": "<p>A production folder can contain routes you never request. The number that belongs on a homepage is the bytes a browser downloads to read <strong>one post</strong>.</p>\n<p>For a post page that means:</p>\n<ol><li>the HTML document</li><li>every <code>link rel=\"stylesheet\"</code></li><li>every <code>script</code> and <code>modulepreload</code> the document names</li><li>gzipped, because that is what went over the wire</li></ol>\n<pre class=\"code\"><code class=\"language-js\"><span class=\"tok-kw\">const</span> <span class=\"tok-id\">doc</span> = <span class=\"tok-kw\">await</span> <span class=\"tok-id\">fetch</span>(<span class=\"tok-id\">url</span>).<span class=\"tok-id\">then</span>((<span class=\"tok-id\">r</span>) =&gt; <span class=\"tok-id\">r</span>.<span class=\"tok-id\">text</span>());\n<span class=\"tok-kw\">const</span> <span class=\"tok-id\">urls</span> = [...<span class=\"tok-id\">doc</span>.<span class=\"tok-id\">matchAll</span>(/&lt;(?:<span class=\"tok-id\">script</span>|<span class=\"tok-id\">link</span>)[^&gt;]+(?:<span class=\"tok-id\">src</span>|<span class=\"tok-id\">href</span>)=<span class=\"tok-str\">\"([^\"</span>]+)\"/<span class=\"tok-id\">g</span>)]\n  .<span class=\"tok-id\">map</span>((<span class=\"tok-id\">m</span>) =&gt; <span class=\"tok-id\">m</span>[<span class=\"tok-num\">1</span>]);</code></pre>\n<p>Images are content, not toolchain. We keep them in the subject app so the pages are real, and we do not fold their bytes into the framework column.</p>"
  },
  {
    "slug": "cold-builds-and-the-cache-you-forgot",
    "title": "Cold builds and the cache you forgot",
    "date": "2026-01-22",
    "excerpt": "A 'cold' build that reused a persistent compiler daemon is a warm build wearing a hat.",
    "tags": [
      "performance",
      "tooling"
    ],
    "html": "<p>Cache hygiene is part of the methodology. The easy miss is deleting <code>dist/</code> and leaving the compiler's own scratch directory.</p>\n<p>For this bench:</p>\n<ul><li>deka cold: <code>dist/</code>, <code>.cache/</code>, <code>.deka-dist-stage/</code></li><li>Vite cold: <code>dist/</code>, <code>node_modules/.vite/</code></li><li>neither cold includes reinstalling dependencies</li></ul>\n<pre class=\"code\"><code class=\"language-bash\"><span class=\"tok-kw\">rm</span> -<span class=\"tok-id\">rf</span> <span class=\"tok-id\">deka-blog</span>/<span class=\"tok-id\">dist</span> <span class=\"tok-id\">deka-blog</span>/.<span class=\"tok-id\">cache</span> <span class=\"tok-id\">deka-blog</span>/.<span class=\"tok-id\">deka-dist-stage</span>\n<span class=\"tok-kw\">rm</span> -<span class=\"tok-id\">rf</span> <span class=\"tok-id\">vite-blog</span>/<span class=\"tok-id\">dist</span> <span class=\"tok-id\">vite-blog</span>/<span class=\"tok-id\">node_modules</span>/.<span class=\"tok-id\">vite</span></code></pre>\n<p>Incremental is the opposite ritual: leave the caches, change one source file that the production graph actually imports, rebuild.</p>\n<p>If your incremental number equals your cold number, you did not hit the cache, or the toolchain does not have one.</p>"
  },
  {
    "slug": "fair-play-is-the-product",
    "title": "Fair play is the product",
    "date": "2026-01-18",
    "excerpt": "A rigged benchmark is worse than none. The methodology is the homepage, not a footnote.",
    "tags": [
      "tooling",
      "language-design"
    ],
    "html": "<p>The JS ecosystem learned the wrong lesson from a decade of \"framework X is 2ms faster\" charts: that the chart is the argument. The argument is whether a stranger can rerun the chart.</p>\n<p>Fair play, for this repo:</p>\n<ul><li>pinned toolchains (<code>deka</code> main-build pending the next release, <code>dsc 0.52.2</code>, React 19.1.1 on the Vite side)</li><li>identical markdown, identical routes, identical widgets</li><li>production configs, never a development server in a production column</li><li>one machine, isolated runs, medians of five</li><li>a table that says what is not measured yet</li></ul>\n<pre class=\"code\"><code class=\"language-json\">{\n  <span class=\"tok-str\">\"deka\"</span>: <span class=\"tok-str\">\"main-build (git sha in the results stanza)\"</span>,\n  <span class=\"tok-str\">\"dsc\"</span>: <span class=\"tok-str\">\"0.52.2\"</span>,\n  <span class=\"tok-str\">\"react\"</span>: <span class=\"tok-str\">\"19.1.1\"</span>,\n  <span class=\"tok-str\">\"vite\"</span>: <span class=\"tok-str\">\"pinned in bench/vite-blog/package.json\"</span>\n}</code></pre>\n<p>If we cannot name the versions, we do not publish the number.</p>"
  },
  {
    "slug": "islands-are-a-budget",
    "title": "Islands are a budget, not a motif",
    "date": "2026-01-14",
    "excerpt": "An island is a confession that this subtree needs a runtime. Spend them like you spend bytes.",
    "tags": [
      "web",
      "runtime",
      "language-design"
    ],
    "html": "<p>The island diagram is easy to draw and easy to abuse. Two colored rectangles on a static page do not make a framework honest. The confession is the interesting part: this component closes over state that has to exist in the browser.</p>\n<p><img src=\"/images/island.svg\" alt=\"Static page with two islands\" /></p>\n<p>On this blog the budget is two:</p>\n<ul><li>a theme toggle that must remember <code>light</code> / <code>dark</code></li><li>a newsletter form that must talk back to the user</li></ul>\n<p>Everything else is HTML. If we add a third island \"for the demo\", we are no longer measuring a blog.</p>\n<pre class=\"code\"><code class=\"language-ds\"><span class=\"tok-kw\">export</span> <span class=\"tok-kw\">fn</span> <span class=\"tok-id\">ThemeToggle</span>() <span class=\"tok-id\">ReactNode</span> {\n  <span class=\"tok-kw\">const</span> <span class=\"tok-id\">theme</span> = <span class=\"tok-kw\">useContext</span>(<span class=\"tok-id\">ThemeContext</span>)\n  <span class=\"tok-kw\">return</span> &lt;<span class=\"tok-id\">button</span> <span class=\"tok-id\">type</span>=<span class=\"tok-str\">\"button\"</span> <span class=\"tok-id\">class</span>=<span class=\"tok-str\">\"theme-toggle\"</span>&gt;{<span class=\"tok-id\">theme</span>}&lt;/<span class=\"tok-id\">button</span>&gt;\n}</code></pre>\n<p>The directive <code>client:load</code> is the receipt. No directive, no client JS. That is the contract this subject app is here to dogfood.</p>"
  },
  {
    "slug": "measuring-the-wrong-loop",
    "title": "Measuring the wrong loop",
    "date": "2026-01-09",
    "excerpt": "Dev-mode HMR times are a different sport from production cold builds. Mixing them is how strawmen get published.",
    "tags": [
      "performance",
      "tooling"
    ],
    "html": "<p>There are at least four clocks in a frontend toolchain and they are not interchangeable.</p>\n<table><thead><tr><th>Clock</th><th>What it includes</th><th>What it must not include</th></tr></thead><tbody><tr><td>cold production build</td><td>empty cache, deps already installed</td><td><code>npm install</code>, network</td></tr><tr><td>incremental production build</td><td>one source byte changed</td><td>wiping <code>node_modules</code></td></tr><tr><td>HMR</td><td>file save → paint</td><td>a full reload you called \"fast\"</td></tr><tr><td>TTFB</td><td>first byte of the production server</td><td>a warm CDN edge in another region</td></tr></tbody></table>\n<p><img src=\"/images/pipeline.svg\" alt=\"Compile pipeline\" /></p>\n<p>This phase of the bench measures the first two plus payload. HMR and TTFB wait for a later lane because we will not print a number we cannot regenerate.</p>\n<pre class=\"code\"><code class=\"language-bash\"><span class=\"tok-com\"># cold is a cache shape, not a machine reboot</span>\n<span class=\"tok-kw\">rm</span> -<span class=\"tok-id\">rf</span> <span class=\"tok-id\">dist</span> .<span class=\"tok-id\">cache</span> <span class=\"tok-id\">node_modules</span>/.<span class=\"tok-id\">vite</span></code></pre>\n<p>If a write-up reports \"Vite is slow\" from a cold <code>npm create</code> on Wi-Fi, it measured the registry, not the bundler.</p>"
  },
  {
    "slug": "zero-js-by-default",
    "title": "Zero JS by default is a product decision",
    "date": "2026-01-04",
    "excerpt": "Most of a blog is dead HTML. Shipping a framework to paint it is a choice, not a law of physics.",
    "tags": [
      "runtime",
      "web",
      "performance"
    ],
    "html": "<p>A homepage number is only as honest as the page it describes. This bench is a blog because blogs are the most common \"we shipped a SPA by accident\" app: twenty-odd articles, a couple of interactive widgets, and a lot of prose that never needed a runtime.</p>\n<p><img src=\"/images/hero.svg\" alt=\"Abstract runtime graph\" /></p>\n<p>The zero-JS default is not an aesthetic. If a route has no event handlers, the HTML that left the compiler should be the HTML the browser paints. Islands exist so we can break that rule on purpose.</p>\n<h2>What \"default\" means</h2>\n<p>Default is the path you get when you do not opt in. A toggle and a newsletter form opt in. A tag listing does not.</p>\n<pre class=\"code\"><code class=\"language-js\"><span class=\"tok-kw\">export</span> <span class=\"tok-kw\">function</span> <span class=\"tok-id\">PostCard</span>({ <span class=\"tok-id\">post</span> }) {\n  <span class=\"tok-kw\">return</span> <span class=\"tok-str\">`&lt;article&gt;&lt;a href=\"/posts/${post.slug}\"&gt;${post.title}&lt;/a&gt;&lt;/article&gt;`</span>;\n}</code></pre>\n<p>If that function never reads <code>window</code>, never registers an effect, and never closes over a setter, it is documentation, not a program. Treating it as a program is how 180 kB of JavaScript shows up on a paragraph of text.</p>\n<h2>The audit you should run</h2>\n<ol><li>Disable JavaScript.</li><li>Read a post.</li><li>Follow two internal links.</li><li>Only then turn JS back on and use the widgets.</li></ol>\n<p>If step 2 fails, the \"blog\" is an application pretending to be a document.</p>"
  }
];
export const tagNames: string[] = ["compilers","javascript","language-design","performance","runtime","rust","tooling","web"];
export const pageNumbers: string[] = ["2","3","4","5"];
