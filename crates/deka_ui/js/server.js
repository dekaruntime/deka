// ui/server — render ComponentNodes to HTML. Never imported from ui/jsx.
// Function tags are invoked here. Text and attributes are escaped.

import { Fragment, isComponentNode } from "./jsx.js";
import { Suspense } from "./suspense.js";

function escapeHtml(text) {
  return String(text)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function base64Encode(str) {
  if (typeof btoa === "function") return btoa(str);
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let output = "";
  for (let i = 0; i < str.length; i += 3) {
    const a = str.charCodeAt(i);
    const b = i + 1 < str.length ? str.charCodeAt(i + 1) : 0;
    const c = i + 2 < str.length ? str.charCodeAt(i + 2) : 0;
    const triple = (a << 16) | (b << 8) | c;
    output += alphabet[(triple >> 18) & 63];
    output += alphabet[(triple >> 12) & 63];
    output += i + 1 < str.length ? alphabet[(triple >> 6) & 63] : "=";
    output += i + 2 < str.length ? alphabet[triple & 63] : "=";
  }
  return output;
}

const VOID = new Set([
  "area", "base", "br", "col", "embed", "hr", "img", "input",
  "link", "meta", "param", "source", "track", "wbr",
]);

function extractDirectives(props) {
  const rest = {};
  const directives = [];
  for (const [key, value] of Object.entries(props ?? {})) {
    if (key.startsWith("client:") && value !== false && value != null) {
      directives.push(key.slice(7));
    } else if (typeof value === "function" && key.startsWith("on")) {
      // event handlers are client-only
    } else {
      rest[key] = value;
    }
  }
  return { rest, directives };
}

function renderAttributes(props) {
  let attrs = "";
  for (const [key, value] of Object.entries(props ?? {})) {
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

function isLive(node) {
  return node != null && typeof node === "object" && node.__live === true && typeof node.read === "function";
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

function createCtx() {
  return { boundaryId: 0, stack: [], pending: [] };
}

function handlePromiseSync(ctx, promise) {
  if (ctx.stack.length === 0) return "";
  const id = ctx.stack[ctx.stack.length - 1];
  ctx.pending.push({ id, promise });
  return "";
}

function renderNode(node, ctx) {
  if (node == null || typeof node === "boolean") return "";
  if (isLive(node)) return renderNode(node.read(), ctx);
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
    const { rest, directives } = extractDirectives(props);
    const result = tag({ ...rest, children });
    if (isPromise(result)) return handlePromiseSync(ctx, result);
    const html = forwardClass(renderNode(result, ctx), rest.class);
    if (directives.length === 0) return html;
    const islandName = tag.name || "Anonymous";
    const directive = directives[0];
    const serializedProps = JSON.stringify(rest);
    return `<!--deka-island start:${base64Encode(islandName)} directive:${base64Encode(directive)} props:${base64Encode(serializedProps)}-->${html}<!--deka-island end:${base64Encode(islandName)}-->`;
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
  if (isLive(node)) return await renderNodeAsync(node.read());
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
    const { rest, directives } = extractDirectives(props);
    let result = tag({ ...rest, children });
    if (isPromise(result)) result = await result;
    const html = forwardClass(await renderNodeAsync(result), rest.class);
    if (directives.length === 0) return html;
    const islandName = tag.name || "Anonymous";
    const directive = directives[0];
    const serializedProps = JSON.stringify(rest);
    return `<!--deka-island start:${base64Encode(islandName)} directive:${base64Encode(directive)} props:${base64Encode(serializedProps)}-->${html}<!--deka-island end:${base64Encode(islandName)}-->`;
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
  const ctx = createCtx();
  return { html: renderNode(node, ctx), boundaries: ctx.pending.map((item) => item.id) };
}

export async function renderToStringAsync(node) {
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
  const ctx = createCtx();
  yield renderNode(node, ctx);
  const queue = ctx.pending.slice();
  ctx.pending.length = 0;
  while (queue.length > 0) {
    const selected = await nextResolved(queue);
    const index = queue.indexOf(selected.item);
    if (index >= 0) queue.splice(index, 1);
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

export { escapeHtml, Suspense };
