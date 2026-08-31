// ui/server — render ComponentNodes to HTML. Never imported from ui/jsx.
// Function tags are invoked here. Text and attributes are escaped.

import { Fragment, isComponentNode } from "./jsx.js";
import { isLive } from "./reactive.js";
import { Suspense } from "./suspense.js";

function escapeHtml(text) {
  return String(text)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function utf8Bytes(str) {
  if (typeof TextEncoder === "function") return Array.from(new TextEncoder().encode(String(str)));
  const s = String(str);
  const out = [];
  for (let i = 0; i < s.length; i++) {
    let c = s.charCodeAt(i);
    if (c < 0x80) out.push(c);
    else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 0x3f));
    else if (c >= 0xd800 && c <= 0xdbff && i + 1 < s.length) {
      i += 1;
      c = 0x10000 + ((c & 0x3ff) << 10) + (s.charCodeAt(i) & 0x3ff);
      out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 0x3f), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
    } else {
      out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
    }
  }
  return out;
}

function base64Encode(str) {
  const bytes = utf8Bytes(str);
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let output = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const a = bytes[i];
    const b = i + 1 < bytes.length ? bytes[i + 1] : 0;
    const c = i + 2 < bytes.length ? bytes[i + 2] : 0;
    const triple = (a << 16) | (b << 8) | c;
    output += alphabet[(triple >> 18) & 63];
    output += alphabet[(triple >> 12) & 63];
    output += i + 1 < bytes.length ? alphabet[(triple >> 6) & 63] : "=";
    output += i + 2 < bytes.length ? alphabet[triple & 63] : "=";
  }
  return output;
}

function liveText(value) {
  if (value == null || typeof value === "boolean" || value === "") return "\u200b";
  if (typeof value === "string" || typeof value === "number") return String(value);
  return "\u200b";
}

function getDeferSecret() {
  try {
    if (typeof globalThis !== "undefined" && globalThis.__DEKA_DEFER_SECRET) {
      return String(globalThis.__DEKA_DEFER_SECRET);
    }
  } catch (_) {}
  return "";
}

function rotr(n, x) {
  return (x >>> n) | (x << (32 - n));
}

function sha256(bytes) {
  const K = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  ];
  const h = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
  const bitLen = bytes.length * 8;
  const withOne = bytes.concat([0x80]);
  while ((withOne.length % 64) !== 56) withOne.push(0);
  const hi = Math.floor(bitLen / 0x100000000);
  const lo = bitLen >>> 0;
  withOne.push((hi >>> 24) & 255, (hi >>> 16) & 255, (hi >>> 8) & 255, hi & 255);
  withOne.push((lo >>> 24) & 255, (lo >>> 16) & 255, (lo >>> 8) & 255, lo & 255);
  for (let i = 0; i < withOne.length; i += 64) {
    const w = new Array(64);
    for (let t = 0; t < 16; t++) {
      const o = i + t * 4;
      w[t] = ((withOne[o] << 24) | (withOne[o + 1] << 16) | (withOne[o + 2] << 8) | withOne[o + 3]) >>> 0;
    }
    for (let t = 16; t < 64; t++) {
      const s0 = rotr(7, w[t - 15]) ^ rotr(18, w[t - 15]) ^ (w[t - 15] >>> 3);
      const s1 = rotr(17, w[t - 2]) ^ rotr(19, w[t - 2]) ^ (w[t - 2] >>> 10);
      w[t] = (w[t - 16] + s0 + w[t - 7] + s1) >>> 0;
    }
    let a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
    for (let t = 0; t < 64; t++) {
      const S1 = rotr(6, e) ^ rotr(11, e) ^ rotr(25, e);
      const ch = (e & f) ^ (~e & g);
      const temp1 = (hh + S1 + ch + K[t] + w[t]) >>> 0;
      const S0 = rotr(2, a) ^ rotr(13, a) ^ rotr(22, a);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (S0 + maj) >>> 0;
      hh = g; g = f; f = e; e = (d + temp1) >>> 0; d = c; c = b; b = a; a = (temp1 + temp2) >>> 0;
    }
    h[0] = (h[0] + a) >>> 0; h[1] = (h[1] + b) >>> 0; h[2] = (h[2] + c) >>> 0; h[3] = (h[3] + d) >>> 0;
    h[4] = (h[4] + e) >>> 0; h[5] = (h[5] + f) >>> 0; h[6] = (h[6] + g) >>> 0; h[7] = (h[7] + hh) >>> 0;
  }
  let out = "";
  for (let i = 0; i < 8; i++) out += ("00000000" + h[i].toString(16)).slice(-8);
  return out;
}

function sha256Bytes(bytes) {
  const hex = sha256(bytes);
  const out = [];
  for (let i = 0; i < hex.length; i += 2) out.push(parseInt(hex.slice(i, i + 2), 16));
  return out;
}

function hmacSha256Hex(secret, message) {
  let key = utf8Bytes(secret);
  if (key.length > 64) key = sha256Bytes(key);
  while (key.length < 64) key.push(0);
  const oKey = key.map((b) => b ^ 0x5c);
  const iKey = key.map((b) => b ^ 0x36);
  const inner = sha256Bytes(iKey.concat(utf8Bytes(message)));
  return sha256(oKey.concat(inner));
}

export function signDeferIsland(name, propsJson, id) {
  const secret = getDeferSecret();
  if (!secret) return "";
  return hmacSha256Hex(secret, String(name) + "\n" + String(propsJson) + "\n" + String(id));
}

export function verifyDeferIsland(name, propsJson, id, mac) {
  const want = signDeferIsland(name, propsJson, id);
  if (!want) return false;
  const got = String(mac || "");
  if (got.length !== want.length) return false;
  let diff = 0;
  for (let i = 0; i < want.length; i++) diff |= want.charCodeAt(i) ^ got.charCodeAt(i);
  return diff === 0;
}

export function runDeferBatch(body, secret, registry, cacheControl) {
  if (secret) {
    try {
      globalThis.__DEKA_DEFER_SECRET = String(secret);
    } catch (_) {}
  }
  let payload = {};
  try {
    payload = JSON.parse(body || "{}");
  } catch (_) {
    return {
      status: 400,
      body: "{\"error\":\"invalid json\"}",
      headers: { "content-type": "application/json", "cache-control": "private, no-store" },
    };
  }
  const islands = Array.isArray(payload.islands) ? payload.islands.slice(0, 32) : [];
  const fragments = {};
  const seen = {};
  const handlers = registry || {};
  for (const item of islands) {
    const key = String((item && (item.id || item.name)) || "");
    if (!key || seen[key]) continue;
    seen[key] = true;
    const propsJson = JSON.stringify(item && item.props ? item.props : {});
    if (!verifyDeferIsland(item && item.name, propsJson, item && item.id, item && item.mac)) continue;
    const fn = handlers[item && item.name];
    if (typeof fn !== "function") continue;
    const tree = fn(item.props || {});
    const rendered = renderToString(tree);
    fragments[key] = rendered && rendered.html ? rendered.html : "";
  }
  const cache = cacheControl || "private, no-store";
  return {
    status: 200,
    body: JSON.stringify({ fragments }),
    headers: { "content-type": "application/json", "cache-control": cache, "vary": "cookie" },
  };
}

const VOID = new Set([
  "area", "base", "br", "col", "embed", "hr", "img", "input",
  "link", "meta", "param", "source", "track", "wbr",
]);

function jsonSafe(value) {
  if (value == null) return value;
  const t = typeof value;
  if (t === "string" || t === "number" || t === "boolean") return value;
  if (Array.isArray(value)) {
    return value.map(jsonSafe).filter((item) => item !== undefined);
  }
  if (t !== "object") return undefined;
  if (value.__componentNode || value.__live) return undefined;
  const out = {};
  for (const [key, child] of Object.entries(value)) {
    const next = jsonSafe(child);
    if (next !== undefined) out[key] = next;
  }
  return out;
}

function serializeIslandProps(props) {
  try {
    return JSON.stringify(jsonSafe(props) ?? {});
  } catch (_) {
    return "{}";
  }
}

function extractDirectives(props) {
  const rest = {};
  const directives = [];
  let cache = null;
  for (const [key, value] of Object.entries(props ?? {})) {
    if (key.startsWith("client:") && value !== false && value != null) {
      directives.push(key.slice(7));
    } else if (key === "server:defer" && value !== false && value != null) {
      directives.push("defer");
    } else if (key.startsWith("server:")) {
      // unknown server: axis
    } else if (key === "cache") {
      cache = String(value);
    } else if (typeof value === "function" && key.startsWith("on")) {
      // event handlers are client-only
    } else {
      rest[key] = value;
    }
  }
  return { rest, directives, cache };
}

function fallbackNodes(children) {
  const list = Array.isArray(children) ? children : children == null ? [] : [children];
  const out = [];
  for (const child of list) {
    if (!isComponentNode(child)) continue;
    if (child.props && child.props.slot === "fallback") out.push(child);
  }
  return out;
}

function wrapDeferred(name, directive, props, cache, id, html) {
  const cachePart = cache ? ` cache:${base64Encode(String(cache))}` : "";
  const propsJson = serializeIslandProps(props);
  const mac = signDeferIsland(name, propsJson, id);
  const macPart = mac ? ` mac:${base64Encode(mac)}` : "";
  return `<!--deka-island start:${base64Encode(name)} directive:${base64Encode(directive)} props:${base64Encode(propsJson)} id:${base64Encode(id)}${cachePart}${macPart}--><span data-deka-defer="${escapeHtml(id)}">${html}</span><!--deka-island end:${base64Encode(name)}-->`;
}

function renderAttributes(props) {
  let attrs = "";
  for (const [key, value] of Object.entries(props ?? {})) {
    if (typeof value === "function") continue;
    if (key.length > 2 && key.startsWith("on")) continue;
    if (value === true) {
      attrs += ` ${escapeHtml(key)}`;
    } else if (value === false || value == null) {
      continue;
    } else {
      attrs += ` ${escapeHtml(key)}="${escapeHtml(value)}"`;
    }
  }
  return attrs;
}

function forwardClass(html, className) {
  if (!className || typeof html !== "string" || html[0] !== "<") return html;
  const escaped = escapeHtml(className);
  return html.replace(/^<([^\s>\/]+)((?:\s[^>]*)?)(\/?>)/, (m, tag, attrs, close) => {
    if (/\sclass\s*=/.test(attrs)) return m;
    return `<${tag}${attrs} class="${escaped}"${close}`;
  });
}

function isPromise(value) {
  return value != null && (typeof value === "object" || typeof value === "function") && typeof value.then === "function";
}

function isSuspenseTag(tag) {
  return tag === Suspense || (typeof tag === "function" && tag.__dekaSuspense === true);
}

function wrapFallback(id, fallbackHtml) {
  return `<div id="${escapeHtml(id)}" data-deka-suspense="pending">${fallbackHtml}</div>`;
}

let deferSeq = 0;

function nextDeferId() {
  deferSeq += 1;
  return "D:" + deferSeq;
}

function createCtx() {
  return { boundaryId: 0, stack: [], pending: [], boundaryChildren: {} };
}

function handlePromiseSync(ctx, promise) {
  if (ctx.stack.length === 0) return "";
  const id = ctx.stack[ctx.stack.length - 1];
  const existing = ctx.pending.find((item) => item.id === id);
  if (existing) {
    existing.promise = Promise.all([existing.promise, promise]);
    return "";
  }
  ctx.pending.push({
    id,
    promise,
    children: ctx.boundaryChildren ? ctx.boundaryChildren[id] : null,
  });
  return "";
}

function renderNode(node, ctx) {
  if (node == null || typeof node === "boolean") return "";
  if (isLive(node)) {
    let value;
    try {
      value = node.read();
    } catch (_) {
      return "";
    }
    if (isComponentNode(value) || Array.isArray(value)) return renderNode(value, ctx);
    return escapeHtml(liveText(value));
  }
  if (typeof node === "string" || typeof node === "number") {
    return escapeHtml(String(node));
  }
  if (Array.isArray(node)) {
    let out = "";
    for (const child of node) out += renderNode(child, ctx);
    return out;
  }
  if (!isComponentNode(node)) {
    return escapeHtml(String(node));
  }

  const { tag, props, children } = node;

  if (tag === Fragment) {
    return renderNode(children, ctx);
  }

  if (typeof tag === "function") {
    if (isSuspenseTag(tag)) {
      return renderSuspenseSync(node, ctx);
    }
    const { rest, directives, cache } = extractDirectives(props);
    if (directives.includes("defer")) {
      const html = renderNode(fallbackNodes(children), ctx);
      return wrapDeferred(tag.name || "Anonymous", "defer", rest, cache, nextDeferId(), html);
    }
    const result = tag({ ...rest, children });
    if (isPromise(result)) return handlePromiseSync(ctx, result);
    const html = forwardClass(renderNode(result, ctx), rest.class);
    if (directives.length === 0) return html;
    const islandName = tag.name || "Anonymous";
    const directive = directives[0];
    return `<!--deka-island start:${base64Encode(islandName)} directive:${base64Encode(directive)} props:${base64Encode(serializeIslandProps(rest))}-->${html}<!--deka-island end:${base64Encode(islandName)}-->`;
  }

  if (typeof tag === "string") {
    const { rest, directives } = extractDirectives(props);
    const attrs = renderAttributes(rest);
    let markerAttrs = "";
    for (const directive of directives) {
      markerAttrs += ` data-client-${escapeHtml(directive)}`;
    }
    const childHtml = renderNode(children, ctx);
    if (childHtml === "" && VOID.has(tag)) {
      return `<${tag}${attrs}${markerAttrs} />`;
    }
    return `<${tag}${attrs}${markerAttrs}>${childHtml}</${tag}>`;
  }

  return "";
}

function renderSuspenseSync(node, ctx) {
  const id = "S:" + (++ctx.boundaryId);
  ctx.stack.push(id);
  ctx.boundaryChildren[id] = node.children;
  const inner = renderNode(node.children, ctx);
  ctx.stack.pop();
  if (ctx.pending.some((item) => item.id === id)) {
    const fallbackHtml = renderNode(node.props ? node.props.fallback : null, ctx);
    return wrapFallback(id, fallbackHtml);
  }
  return inner;
}

async function renderNodeAsync(node) {
  if (node == null || typeof node === "boolean") return "";
  if (isLive(node)) {
    let value;
    try {
      value = node.read();
    } catch (_) {
      return "";
    }
    if (isComponentNode(value) || Array.isArray(value)) return await renderNodeAsync(value);
    return escapeHtml(liveText(value));
  }
  if (typeof node === "string" || typeof node === "number") {
    return escapeHtml(String(node));
  }
  if (Array.isArray(node)) {
    let out = "";
    for (const child of node) out += await renderNodeAsync(child);
    return out;
  }
  if (!isComponentNode(node)) {
    return escapeHtml(String(node));
  }

  const { tag, props, children } = node;

  if (tag === Fragment) {
    return await renderNodeAsync(children);
  }

  if (typeof tag === "function") {
    if (isSuspenseTag(tag)) {
      return await renderNodeAsync(children);
    }
    const { rest, directives, cache } = extractDirectives(props);
    if (directives.includes("defer")) {
      const html = await renderNodeAsync(fallbackNodes(children));
      return wrapDeferred(tag.name || "Anonymous", "defer", rest, cache, nextDeferId(), html);
    }
    let result = tag({ ...rest, children });
    if (isPromise(result)) result = await result;
    const html = forwardClass(await renderNodeAsync(result), rest.class);
    if (directives.length === 0) return html;
    const islandName = tag.name || "Anonymous";
    const directive = directives[0];
    return `<!--deka-island start:${base64Encode(islandName)} directive:${base64Encode(directive)} props:${base64Encode(serializeIslandProps(rest))}-->${html}<!--deka-island end:${base64Encode(islandName)}-->`;
  }

  if (typeof tag === "string") {
    const { rest, directives } = extractDirectives(props);
    const attrs = renderAttributes(rest);
    let markerAttrs = "";
    for (const directive of directives) {
      markerAttrs += ` data-client-${escapeHtml(directive)}`;
    }
    const childHtml = await renderNodeAsync(children);
    if (childHtml === "" && VOID.has(tag)) {
      return `<${tag}${attrs}${markerAttrs} />`;
    }
    return `<${tag}${attrs}${markerAttrs}>${childHtml}</${tag}>`;
  }

  return "";
}

export function renderToString(node) {
  deferSeq = 0;
  const ctx = createCtx();
  return { html: renderNode(node, ctx), boundaries: ctx.pending.map((item) => item.id) };
}

export async function renderToStringAsync(node) {
  deferSeq = 0;
  return { html: await renderNodeAsync(node), boundaries: [] };
}

function swapChunk(id, html) {
  const templateId = "deka-swap-" + id;
  const tid = JSON.stringify(templateId);
  const sid = JSON.stringify(id);
  return `<template id="${escapeHtml(templateId)}">${html}</template><script>(() => { const t = document.getElementById(${tid}); const slot = document.getElementById(${sid}); if (slot && t) slot.replaceWith(t.content.cloneNode(true)); t && t.remove(); document.currentScript && document.currentScript.remove(); })();</script>`;
}

function encodeChunk(text) {
  if (typeof TextEncoder === "function") return new TextEncoder().encode(text);
  return text;
}

async function nextResolved(queue) {
  return await new Promise((resolve, reject) => {
    let settled = false;
    for (const item of queue) {
      Promise.resolve(item.promise).then(
        (value) => {
          if (settled) return;
          settled = true;
          resolve({ item, value });
        },
        (error) => {
          if (settled) return;
          settled = true;
          reject(error);
        }
      );
    }
  });
}

async function* iterateChunks(node) {
  deferSeq = 0;
  const ctx = createCtx();
  yield renderNode(node, ctx);
  const queue = ctx.pending.slice();
  ctx.pending.length = 0;
  while (queue.length > 0) {
    const selected = await nextResolved(queue);
    const index = queue.indexOf(selected.item);
    if (index >= 0) queue.splice(index, 1);
    // Stream the resolved tree with the same sync renderer so nested
    // Suspense can enqueue more boundaries. renderNodeAsync unwraps
    // Suspense and waits, which collapsed nested fallbacks (Hats
    // jsx_suspense_stream_swap).
    const html = renderNode(selected.value, ctx);
    for (const extra of ctx.pending) queue.push(extra);
    ctx.pending.length = 0;
    if (selected.item.id) yield swapChunk(selected.item.id, html);
  }
}

function createByteStream(start) {
  if (typeof ReadableStream === "function") {
    return new ReadableStream({ start });
  }
  const chunks = [];
  let done = false;
  let failure = null;
  let wake = null;
  const controller = {
    enqueue(chunk) {
      chunks.push(chunk);
      if (wake) {
        const w = wake;
        wake = null;
        w();
      }
    },
    close() {
      done = true;
      if (wake) {
        const w = wake;
        wake = null;
        w();
      }
    },
    error(err) {
      failure = err;
      done = true;
      if (wake) {
        const w = wake;
        wake = null;
        w();
      }
    },
  };
  const started = Promise.resolve(start(controller));
  return {
    getReader() {
      let i = 0;
      return {
        async read() {
          await started;
          while (i >= chunks.length && !done) {
            await new Promise((resolve) => {
              wake = resolve;
            });
          }
          if (failure) throw failure;
          if (i >= chunks.length) return { done: true, value: undefined };
          return { done: false, value: chunks[i++] };
        },
      };
    },
  };
}

export function renderToStream(node) {
  return createByteStream(async (controller) => {
    for await (const text of iterateChunks(node)) {
      controller.enqueue(encodeChunk(text));
    }
    controller.close();
  });
}

export async function collectStream(stream) {
  const reader = stream.getReader();
  const decoder = typeof TextDecoder === "function" ? new TextDecoder() : null;
  let out = "";
  for (;;) {
    const step = await reader.read();
    if (step.done) break;
    const value = step.value;
    if (typeof value === "string") out += value;
    else if (decoder) out += decoder.decode(value, { stream: true });
  }
  if (decoder) out += decoder.decode();
  return out;
}

export async function renderToStreamHtml(node) {
  let out = "";
  for await (const text of iterateChunks(node)) out += text;
  return out;
}

export { escapeHtml, liveText, Suspense };
