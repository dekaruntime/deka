// ui/server — render ComponentNodes to HTML. Never imported from ui/jsx.
// Function tags are invoked here. Text and attributes are escaped.

import { Fragment, isComponentNode } from "./jsx.js";

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

function renderNode(node) {
  if (node == null || typeof node === "boolean") return "";
  if (isLive(node)) return renderNode(node.read());
  if (typeof node === "string" || typeof node === "number") {
    return escapeHtml(String(node));
  }
  if (Array.isArray(node)) {
    let out = "";
    for (const child of node) out += renderNode(child);
    return out;
  }
  if (!isComponentNode(node)) {
    return escapeHtml(String(node));
  }

  const { tag, props, children } = node;

  if (tag === Fragment) {
    return renderNode(children);
  }

  if (typeof tag === "function") {
    const { rest, directives } = extractDirectives(props);
    const result = tag({ ...rest, children });
    const html = forwardClass(renderNode(result), rest.class);
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
    const childHtml = renderNode(children);
    if (childHtml === "" && VOID.has(tag)) {
      return `<${tag}${attrs}${markerAttrs} />`;
    }
    return `<${tag}${attrs}${markerAttrs}>${childHtml}</${tag}>`;
  }

  return "";
}

export function renderToString(node) {
  return { html: renderNode(node), boundaries: [] };
}

export async function renderToStringAsync(node) {
  return renderToString(node);
}

export { escapeHtml };
