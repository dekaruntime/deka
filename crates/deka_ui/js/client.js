// ui/client — walk-and-attach hydrate. Never import ui/server.
// Registry is created lazily on first registerIsland.

import { isComponentNode, Fragment } from "./jsx.js";
import { isLive, effect } from "./reactive.js";

let registry;
function islands() {
  if (!registry) registry = new Map();
  return registry;
}

export function registerIsland(name, component) {
  if (typeof name !== "string" || typeof component !== "function") return;
  islands().set(name, component);
}

function decodeB64(value) {
  try {
    if (typeof atob === "function") return atob(value);
  } catch (_) {}
  return "";
}

function parseMarker(text) {
  const raw = String(text || "").trim();
  const match = raw.match(
    /^deka-island start:([A-Za-z0-9+/=]+) directive:([A-Za-z0-9+/=]+) props:([A-Za-z0-9+/=]+)(?: id:([A-Za-z0-9+/=]+))?(?: cache:([A-Za-z0-9+/=]+))?$/
  );
  if (!match) return null;
  let props = {};
  try {
    props = JSON.parse(decodeB64(match[3]) || "{}") || {};
  } catch (_) {
    props = {};
  }
  return {
    name: decodeB64(match[1]),
    directive: decodeB64(match[2]) || "load",
    props,
    id: match[4] ? decodeB64(match[4]) : "",
    cache: match[5] ? decodeB64(match[5]) : "",
  };
}

function nextElement(node) {
  let cur = node ? node.nextSibling : null;
  while (cur && cur.nodeType !== 1) cur = cur.nextSibling;
  return cur;
}

function collectComments(root, out) {
  if (!root) return;
  if (root.nodeType === 8) {
    out.push(root);
    return;
  }
  const kids = root.childNodes;
  if (!kids) return;
  for (let i = 0; i < kids.length; i++) collectComments(kids[i], out);
}

function findIslands(root) {
  const comments = [];
  collectComments(root, comments);
  const found = [];
  for (const comment of comments) {
    const meta = parseMarker(comment.nodeValue || comment.data || "");
    if (!meta) continue;
    const el = nextElement(comment);
    if (!el) continue;
    found.push({ ...meta, el, comment });
  }
  return found;
}

function schedule(directive, el, run) {
  const kind = String(directive || "load");
  if (kind === "idle") {
    if (typeof requestIdleCallback === "function") {
      requestIdleCallback(() => run());
    } else if (typeof setTimeout === "function") {
      setTimeout(run, 1);
    } else {
      run();
    }
    return;
  }
  if (kind === "visible") {
    if (typeof IntersectionObserver === "function" && el && el.nodeType === 1) {
      const io = new IntersectionObserver((entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          io.disconnect();
          run();
        }
      });
      io.observe(el);
      return;
    }
  }
  run();
}

function mismatch(diagnostics, message) {
  diagnostics.push(message);
  if (typeof console !== "undefined" && typeof console.warn === "function") {
    console.warn("[deka hydrate]", message);
  }
}

function walk(node, dom, diagnostics) {
  if (node == null || typeof node === "boolean") return;
  if (isLive(node)) {
    let value = "";
    try {
      value = String(node.read());
    } catch (_) {
      return;
    }
    if (!dom) return;
    const textNode =
      dom.nodeType === 3
        ? dom
        : dom.childNodes && dom.childNodes.length === 1 && dom.childNodes[0].nodeType === 3
          ? dom.childNodes[0]
          : null;
    if (!textNode) {
      mismatch(diagnostics, "live() binding has no text node");
      return;
    }
    if (textNode.textContent !== value) {
      mismatch(
        diagnostics,
        `mismatch at live() text: expected ${JSON.stringify(value)} got ${JSON.stringify(textNode.textContent)}`
      );
      return;
    }
    effect(() => {
      try {
        textNode.textContent = String(node.read());
      } catch (_) {}
    });
    return;
  }
  if (typeof node === "string" || typeof node === "number") {
    if (dom && dom.nodeType === 3 && dom.textContent !== String(node)) {
      mismatch(diagnostics, "mismatch at text node");
    }
    return;
  }
  if (Array.isArray(node)) {
    const kids = domChildElements(dom && dom.parentNode === null ? dom : dom);
    let i = 0;
    for (const child of node) {
      walk(child, kids[i] || null, diagnostics);
      i += 1;
    }
    return;
  }
  if (!isComponentNode(node)) return;

  const { tag, props, children } = node;
  if (tag === Fragment) {
    const kids = domChildElements(dom && dom.parentNode ? dom.parentNode : dom);
    let i = 0;
    for (const child of children || []) {
      walk(child, kids[i] || null, diagnostics);
      i += 1;
    }
    return;
  }
  if (typeof tag === "function") {
    const result = tag({ ...(props || {}), children });
    walk(result, dom, diagnostics);
    return;
  }
  if (typeof tag !== "string") return;
  if (!dom || dom.nodeType !== 1) {
    mismatch(diagnostics, `mismatch: missing <${tag}>`);
    return;
  }
  const expectedId = props && props["data-deka-id"];
  const gotId = typeof dom.getAttribute === "function" ? dom.getAttribute("data-deka-id") : null;
  if (expectedId && gotId && expectedId !== gotId) {
    mismatch(diagnostics, `mismatch at data-deka-id: expected ${expectedId} got ${gotId}`);
    return;
  }
  if (dom.tagName && dom.tagName.toLowerCase() !== tag.toLowerCase()) {
    mismatch(diagnostics, `mismatch at tag: expected <${tag}> got <${dom.tagName}>`);
    return;
  }
  for (const [key, value] of Object.entries(props || {})) {
    if (typeof value === "function" && key.length > 2 && key.startsWith("on")) {
      const type = key.slice(2).toLowerCase();
      if (typeof dom.addEventListener === "function") {
        dom.addEventListener(type, value);
      }
    }
  }
  const childDoms = elementChildNodes(dom);
  const childNodes = children || [];
  for (let i = 0; i < childNodes.length; i++) {
    walk(childNodes[i], childDoms[i] || null, diagnostics);
  }
}

function elementChildNodes(dom) {
  const out = [];
  if (!dom || !dom.childNodes) return out;
  for (let i = 0; i < dom.childNodes.length; i++) {
    const n = dom.childNodes[i];
    if (n.nodeType === 8) continue;
    out.push(n);
  }
  return out;
}

function domChildElements(dom) {
  return elementChildNodes(dom);
}

let observing = false;
let observer = null;

function startObserver() {
  if (observing) return;
  if (typeof MutationObserver !== "function" || typeof document === "undefined") return;
  observing = true;
  observer = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      for (const node of mutation.addedNodes) {
        hydrate(node);
      }
    }
  });
  observer.observe(document.documentElement || document.body, {
    childList: true,
    subtree: true,
  });
}

export function stopObserver() {
  if (observer) observer.disconnect();
  observer = null;
  observing = false;
}

function fetchDeferred(items) {
  if (typeof fetch !== "function" || items.length === 0) return;
  const payload = JSON.stringify({
    islands: items.map((item) => ({
      id: item.id || item.name,
      name: item.name,
      props: item.props || {},
    })),
  });
  fetch("/_deka/defer", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "same-origin",
    body: payload,
  })
    .then((res) => (res && res.ok ? res.json() : null))
    .then((data) => {
      const fragments = data && data.fragments ? data.fragments : {};
      for (const item of items) {
        const key = item.id || item.name;
        const html = fragments[key];
        if (typeof html !== "string" || !item.el) continue;
        const slot =
          item.el.getAttribute && item.el.getAttribute("data-deka-defer")
            ? item.el
            : item.el;
        try {
          slot.outerHTML = html;
        } catch (_) {}
      }
    })
    .catch(() => {});
}

export function hydrate(root) {
  if (typeof globalThis !== "undefined") {
    globalThis.deka = globalThis.deka || {};
    globalThis.deka.ui = Object.assign({}, globalThis.deka.ui || {}, { hydrate });
  }
  if (typeof document === "undefined") return;
  const scope = root && root.nodeType ? root : document;
  const found = findIslands(scope);
  const deferred = [];
  for (const island of found) {
    if (island.directive === "defer") {
      if (!island.el.__dekaHydrated) {
        island.el.__dekaHydrated = true;
        deferred.push(island);
      }
      continue;
    }
    if (island.el.__dekaHydrated) continue;
    const component = islands().get(island.name);
    if (typeof component !== "function") continue;
    const el = island.el;
    schedule(island.directive, el, () => {
      if (el.__dekaHydrated) return;
      const tree = component(island.props);
      const diagnostics = [];
      walk(tree, el, diagnostics);
      el.__dekaHydrated = true;
    });
  }
  if (deferred.length > 0) fetchDeferred(deferred);
  if (found.length > 0) startObserver();
}
