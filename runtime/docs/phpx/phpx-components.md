---
title: "PHPX Components (JSX + VNode)"
section: "phpx"
category: "components"
categoryLabel: "Components"
categoryOrder: 50
---

# PHPX Components (JSX + VNode)

This document defines the JSX + component runtime for PHPX.
The model is VNode-first: JSX lowers to VNodes, and rendering is a separate step.

## Goals
- Keep JSX as a PHPX-only syntax feature.
- Represent UI as a VNode tree (value semantics) instead of strings.
- Rendering is separate and pluggable.
- Default DOM integration uses replace-mode (partial HTML swap), no diffing.
- Provide React-style component context without global state.

## component/core (PHPX)

### VNode
```
struct VNode {
  $kind: string = 'component.element';
  $type: mixed; // string tag or callable component
  $props: mixed;
  $key: mixed = '';
}
```

### JSX runtime
```
export function jsx($type, $props, $key = ''): VNode;
export function jsxs($type, $props, $key = ''): VNode;
export function createElement($type, $props, ...$children): VNode;

export function isValidElement($value): bool;
export function childrenToArray($children): array<mixed>;
```

JSX auto-injects `jsx`/`jsxs` from `component/core` when the file contains JSX.
User code should not import them.

## Frontmatter templates (Astro-style)
PHPX supports an Astro-like template mode for `.phpx` files.
If the first non-empty line is `---`, the section above the second `---` is PHPX frontmatter,
and everything after is treated as template HTML (compiled to VNodes).

Example:
```
---
import { Link } from 'component/dom';
$title = 'Home';
---

<html>
  <body>
    <h1>{$title}</h1>
    <Link to="/about">About</Link>
  </body>
</html>
```

Template mode is always VNode-based:
- The HTML section compiles to JSX/VNodes.
- The runtime auto-renders it with `renderToString` (no manual echo needed).
- `<!doctype html>` at the top of the template is emitted automatically.
- `import` lines must live in frontmatter (and still appear before other code).

### Component-style frontmatter modules
If a frontmatter template lives under `php_modules/`, it becomes a component module.
Instead of auto-rendering, the compiler exports a single component function:
```
import { Component as Card } from 'ui/card';

echo renderToString(<Card title="Hello" />);
```

Example module (`php_modules/ui/card.phpx`):
```
---
$label = 'Card';
---

<div class="card">
  <h3>{$label}</h3>
  <p>{$props.message}</p>
</div>
```

Rules:
- The exported function is named `Component` (rename on import if you want).
- Frontmatter runs per render (inside `Component`).
- `export` statements inside frontmatter are not allowed in template files.

### Template expressions
- `{ ... }` accepts any PHPX expression.
- `if` / `foreach` blocks are supported inside `{}`:
```
{if ($user) { <p>Hello {$user.name}</p> } else { <p>Guest</p> }}

{foreach ($items as $item) {
  <li>{$item}</li>
}}
```

Notes:
- `foreach ($items as $key => $value)` is supported.
- Blocks compile to expressions (ternary / mapping) and return VNodes.
- Nested blocks are supported.

### JSX expressions
- `{ ... }` accepts any **PHPX expression** (no statements).
- Tight dot works inside JSX: `<Component id={ $user.id } />` (object/struct only).
- Object literals inside JSX require double braces: `{ { hello: 'world' } }`.

### Context (React-style)
```
export function createContext($defaultValue): Context;
export function useContext(Context $ctx): mixed;
export function ContextProvider($props): mixed;

import { createContext, ContextProvider } from 'component/core';
$AuthContext = createContext({ user: 'sam' });
<ContextProvider ctx={$AuthContext} value={{ user: 'sam' }}>
  <Nav />
</ContextProvider>
```

Context is scoped to the component subtree and does not require global state.

## component/dom (server + client)

### Replace mode (default)
- Server renders VNode -> HTML string.
- Client swaps a container with new HTML on navigation.
- No VDOM diffing or hydration required.

### Server API
```
export function createRoot($config): Root;
export function renderToString(VNode $node): string;
export function renderToStream(VNode $node): Stream;
export function renderPartial($node, $title = '', $head = ''): Object;
export function renderPartialJson($node, $title = '', $head = ''): string;
export function renderPartialResponse($node, $title = '', $head = ''): string;
export function Link($props): VNode;
export function Hydration($props): VNode;
```

Config:
- `container`: selector for the swap target (configured in app root)
- Replace-only for now (no mode switching).

Notes:
- `renderToStream` currently returns a full string (streaming hook is stubbed).
- `renderPartial` returns `{ html, title, head? }` (object literal).
- `renderPartialResponse` sets `Content-Type: application/json; charset=utf-8` and returns JSON.
- Components are resolved by name: uppercase tags call a function with the same name
  when it exists; otherwise the tag is rendered as a DOM element.
- Lowercase tags always render as DOM elements, even if a PHP function with the
  same name exists (e.g. `<header>` will not call `header()`).
- `ContextProvider` is handled by the renderer (push/pop around children).

### Full document via JSX
You can build a full document with JSX (no PHP template mode):
```
$style = "body{margin:0;}";
$script = "console.log('hi');";

$doc = <html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Home</title>
    <style>{$style}</style>
  </head>
  <body>
    <div id="app" dataLayout="users">{$body}</div>
    <script>{$script}</script>
  </body>
</html>;

echo "<!doctype html>\n" . renderToString($doc);
```

Notes:
- `<script>` and `<style>` are raw-text tags: children are not HTML-escaped.
- Regular string children are HTML-escaped.
- To insert pre-rendered HTML strings (e.g. server-rendered body), use
  `dangerouslySetInnerHTML` on the target element.

### Link helper (client-side routing opt-in)
```
export function Link(props: {
  to: string,
  target?: string,
  layout?: string,
  replace?: bool,
  children: array<mixed>
}): VNode;
```

`<Link>` renders an anchor with data attributes. The client runtime intercepts it
and performs partial navigation into the configured container. If the container
is missing, it falls back to a hard navigation. Plain `<a>` remains hard navigation.

Layout semantics:
- The container can declare a layout id via `data-layout="users"` (or legacy `dataComponentLayout`).
- `<Hydration layout="users" />` sets a default layout id when the container has none.
- Partial navigation only happens when the current container layout matches the
  link layout (or the hydration default). If missing/mismatched, the client
  performs a full navigation.

#### Layout-aware navigation
To avoid re-sending layout HTML, define a layout id on the container:
```
<div id="app" dataLayout="users">...</div>
```
Then annotate links that expect the same layout:
```
<Link to="/users/Sami" layout="users">Sami</Link>
```
If the client sees a layout mismatch, it performs a hard navigation instead of a
partial swap. Layout ids are required for partial navigation.
`data-layout` is the preferred attribute; `data-component-layout` is still
accepted for backwards compatibility.
You can set a default layout in `createRoot` so links can omit it:
```
createRoot({ container: '#app', layout: 'users' });
```

### Partial response format (JSON)
```
{
  "html": "<div>...</div>",
  "title": "Page title",
  "head": "<meta ...>" // optional
}
```
When the request includes `Accept: text/x-phpx-fragment` (or `?partial=1`
for manual testing), the PHP server responds with `application/json`.

### Client runtime (tiny JS)
- Intercepts `[data-component-link]` clicks.
- Fetches with `Accept: text/x-phpx-fragment` (no query param required).
- Swaps the configured container with returned HTML.
- Updates `document.title` and optional head.
- Handles `history.pushState` + `popstate`.
- Reference implementation: `php_modules/component/dom.client.js` (exports `createRoot`).

### Hydration helper
`<Hydration>` inlines the client runtime and boots it. Place it in `<head>` or
near the end of `<body>`.
```
<Hydration container="#app" layout="users" />
```

Props:
- `container`: selector for the swap target (default `#app`).
- `layout`: layout id to enforce partial navigation (required for partial swaps).
- `nonce`: optional CSP nonce for the inline script.

Example with custom container + nonce:
```
<Hydration container="#root" layout="users" nonce={$cspNonce} />
```

Static rendering note:
- If you omit `<Hydration />`, the page stays fully static (no client JS, no partial navigation).

## Islands directives
Use islands on components (capitalized tags), not DOM tags.

Supported directives:
- `client:load` or `clientLoad`: hydrate immediately after script boot.
- `client:idle` or `clientIdle`: hydrate when the browser is idle (`requestIdleCallback`, fallback to timeout).
- `client:visible` or `clientVisible`: hydrate when visible (`IntersectionObserver`, fallback immediate).
- `client:media` or `clientMedia`: hydrate when a media query matches.
- `client:only` or `clientOnly`: skip server body render; emit wrapper only and hydrate on load.

Canonical example:
```
---
import { jsx, jsxs } from 'component/core'
import { Hydration } from 'component/dom'

function SearchBox(: object) {
  return <input id="search" placeholder="Search packages" />
}

function Chart(: object) {
  return <div>Chart UI</div>
}
---
<div id="app" dataLayout="registry">
  <SearchBox client:idle={true} />
  <Chart client:visible={true} />
</div>
<Hydration container="#app" layout="registry" />
```

Anti-patterns:
- Do not attach `client:*` directives to DOM tags like `<div client:idle={true} />`.
- Do not apply multiple `client:*` directives on one component.
- Do not rely on partial navigation without matching layout ids (`dataLayout` + `Link layout`).

## Cookbook
1. Static SSR only (no JS): render HTML and omit `<Hydration />`.
2. SSR + partial navigation: use `<Link />` + `<Hydration container="#app" layout="..." />`.
3. SSR + islands: add `client:*` directives only to interactive components; keep page chrome static.

## Demo
See `examples/phpx-components/app.phpx` for a working client-side navigation demo.

## Notes
- JSX lowering always targets `component/core` (`jsx/jsxs`).
- Fragment shorthand (`<>...</>`) lowers to a special fragment tag.
- Rendering is optional and lives in `component/dom`.
- Patch/diff mode can be added later without changing component code.
