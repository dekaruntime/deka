// Runtime globals injected into compiled DekaScript JS for headless execution.
// Mirrors the browser tour globals but trimmed to the test harness surface.

import { createRequire } from "node:module";
import { readFileSync, existsSync, mkdirSync, writeFileSync, statSync } from "node:fs";
import { dirname, resolve, isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";

const hostFetch = globalThis.fetch;
const hostJSON = JSON;
const hostURL = URL;
const hostURLSearchParams = URLSearchParams;
const hostTextEncoder = TextEncoder;
const hostTextDecoder = TextDecoder;
const hostBlob = Blob;
const hostFormData = FormData;
const hostHeaders = Headers;
const hostRequest = Request;
const hostResponse = Response;
const hostAtob = atob;
const hostBtoa = btoa;
const hostStructuredClone = structuredClone;
const hostCrypto = globalThis.crypto;
const hostSetTimeout = setTimeout;
const hostSetInterval = setInterval;
const hostClearTimeout = clearTimeout;
const hostClearInterval = clearInterval;
const hostQueueMicrotask = queueMicrotask;

function ok(value) {
  return { ok: true, value };
}

function err(error) {
  return { ok: false, error };
}

function wrapConstructorResult(Ctor) {
  return (...args) => {
    try {
      return ok(new Ctor(...args));
    } catch (error) {
      return err(error);
    }
  };
}

function wrapTimer(timer) {
  return (handler, delay, ...args) => {
    const wrapped =
      typeof handler === "function"
        ? (...timerArgs) => {
            try {
              handler(...timerArgs);
            } catch (error) {
              deka.panic(error);
            }
          }
        : handler;
    return timer(wrapped, delay, ...args);
  };
}

function wrapMicrotask(task) {
  return (callback) => {
    return task(() => {
      try {
        callback();
      } catch (error) {
        deka.panic(error);
      }
    });
  };
}

// ---------------------------------------------------------------------------
// Deka UI JSX runtime primitives
// ---------------------------------------------------------------------------
// These are the immutable node factories that the DekaScript compiler emits for
// JSX (issue #141). The contract is intentionally small and platform-agnostic:
// server and client renderers consume these nodes later; this module does not
// perform rendering or DOM creation.
//
// Compiler contract (issue #146):
//   deka.ui.jsx(tag, props)  -> ComponentNode   // dynamic children
//   deka.ui.jsxs(tag, props) -> ComponentNode   // static children; same runtime behavior
//   deka.ui.Fragment         -> symbol used as tag for <></>
//
// ComponentNode shape (immutable value):
//   {
//     tag:      string | function | symbol,  // primitive tag, function component, or Fragment
//     props:    Object,                      // frozen plain object of props (children excluded)
//     children: Array                        // frozen array of normalized children
//   }
//
// Children normalization rules:
//   - null / undefined are omitted
//   - nested arrays are flattened one level
//   - strings, numbers, booleans, and existing ComponentNodes are preserved

const Fragment = Symbol.for("deka.ui.Fragment");

function isComponentNode(value) {
  return value != null && typeof value === "object" && value.__componentNode === true;
}

function normalizeJsxChildren(children) {
  if (children == null) return [];
  if (Array.isArray(children)) {
    const out = [];
    for (const child of children) {
      if (Array.isArray(child)) {
        for (const inner of normalizeJsxChildren(child)) {
          out.push(inner);
        }
      } else if (child != null) {
        out.push(child);
      }
    }
    return out;
  }
  return [children];
}

function createComponentNode(tag, props) {
  const { children, ...rest } = props ?? {};
  const normalizedChildren = normalizeJsxChildren(children);
  const node = {
    tag,
    props: Object.freeze(rest),
    children: Object.freeze(normalizedChildren),
    toString() {
      return renderNodeSync(this, createRendererContext("sync"));
    },
  };
  Object.defineProperty(node, "__componentNode", {
    value: true,
    enumerable: false,
    writable: false,
    configurable: false,
  });
  return Object.freeze(node);
}

function uiJsx(tag, props) {
  return createComponentNode(tag, props);
}

function uiJsxs(tag, props) {
  return createComponentNode(tag, props);
}

// ---------------------------------------------------------------------------
// Suspense / ErrorBoundary primitives
// ---------------------------------------------------------------------------
// These are special component tags consumed by the server renderer. They are
// functions so they can be referenced directly in JSX, but the renderer treats
// them as boundary markers rather than ordinary function components.

function Suspense(props) {
  return createComponentNode(Suspense, props);
}
Suspense.__dekaTag = "Suspense";

function ErrorBoundary(props) {
  return createComponentNode(ErrorBoundary, props);
}
ErrorBoundary.__dekaTag = "ErrorBoundary";

// ---------------------------------------------------------------------------
// Server renderer for immutable ComponentNodes
// ---------------------------------------------------------------------------
// No VDOM diffing: walk the tree, render to HTML strings, and record Suspense
// boundary metadata so resolved content can be streamed or swapped in later.

function isPromiseLike(value) {
  return value != null && typeof value === "object" && typeof value.then === "function";
}

function isResultErr(value) {
  return (
    value != null &&
    typeof value === "object" &&
    value.__enum === "Result" &&
    value.__case === "Err"
  );
}

function escapeHtml(text) {
  return String(text)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function renderAttributes(props) {
  const attrs = [];
  for (const [key, value] of Object.entries(props ?? {})) {
    if (value === true) {
      attrs.push(` ${escapeHtml(key)}`);
    } else if (value === false || value == null) {
      // omitted boolean/falsy attribute
    } else {
      attrs.push(` ${escapeHtml(key)}="${escapeHtml(value)}"`);
    }
  }
  return attrs.join("");
}

function renderFallbackSync(fallback, error, ctx) {
  let node = fallback;
  if (typeof fallback === "function") {
    try {
      node = error === undefined ? fallback() : fallback(error);
    } catch (_) {
      return "";
    }
  }
  if (node == null || typeof node === "boolean") return "";
  if (isComponentNode(node)) return renderNodeSync(node, ctx);
  if (typeof node === "string" || typeof node === "number") return escapeHtml(String(node));
  return escapeHtml(String(node));
}

async function renderFallbackAsync(fallback, error, ctx) {
  let node = fallback;
  if (typeof fallback === "function") {
    try {
      node = error === undefined ? fallback() : fallback(error);
    } catch (_) {
      return "";
    }
  }
  if (node == null || typeof node === "boolean") return "";
  if (isComponentNode(node)) return await renderNodeAsync(node, ctx);
  if (typeof node === "string" || typeof node === "number") return escapeHtml(String(node));
  return escapeHtml(String(node));
}

function handleRenderErrorSync(error, ctx) {
  const stack = ctx.errorStack || [];
  if (stack.length === 0) {
    throw error;
  }
  const boundary = stack[stack.length - 1];
  return renderFallbackSync(boundary.fallback, error, ctx);
}

async function handleRenderErrorAsync(error, ctx) {
  const stack = ctx.errorStack || [];
  if (stack.length === 0) {
    throw error;
  }
  const boundary = stack[stack.length - 1];
  return await renderFallbackAsync(boundary.fallback, error, ctx);
}

function handlePendingSync(promise, ctx) {
  const stack = ctx.suspenseStack || [];
  if (stack.length === 0) {
    // No Suspense ancestor: drop the async work for the sync server pass.
    return "";
  }
  const boundary = stack[stack.length - 1];
  const record = ctx.boundaries.find((b) => b.id === boundary.id);
  if (record) {
    record.promise = promise;
    record.state = "pending";
  }
  return renderFallbackSync(boundary.fallback, undefined, ctx);
}

async function handlePendingAsync(promise, ctx) {
  const stack = ctx.suspenseStack || [];
  if (stack.length === 0) {
    try {
      return await promise;
    } catch (error) {
      return await handleRenderErrorAsync(error, ctx);
    }
  }
  const boundary = stack[stack.length - 1];
  const record = ctx.boundaries.find((b) => b.id === boundary.id);
  if (record) {
    record.promise = promise;
    record.state = "pending";
  }
  try {
    const value = await promise;
    if (record) record.state = "resolved";
    return value;
  } catch (error) {
    if (record) record.state = "rejected";
    throw error;
  }
}

function renderChildrenSync(children, ctx) {
  if (children == null) return "";
  if (Array.isArray(children)) {
    let out = "";
    for (const child of children) {
      out += renderNodeSync(child, ctx);
    }
    return out;
  }
  return renderNodeSync(children, ctx);
}

async function renderChildrenAsync(children, ctx) {
  if (children == null) return "";
  if (Array.isArray(children)) {
    let out = "";
    for (const child of children) {
      out += await renderNodeAsync(child, ctx);
    }
    return out;
  }
  return await renderNodeAsync(children, ctx);
}

const voidElements = new Set([
  "area", "base", "br", "col", "embed", "hr", "img", "input",
  "link", "meta", "param", "source", "track", "wbr",
]);

function renderNodeSync(node, ctx) {
  if (node == null || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") {
    return escapeHtml(String(node));
  }
  if (Array.isArray(node)) {
    let out = "";
    for (const child of node) {
      out += renderNodeSync(child, ctx);
    }
    return out;
  }
  if (!isComponentNode(node)) {
    return escapeHtml(String(node));
  }

  const { tag, props, children } = node;

  if (tag === Fragment) {
    return renderChildrenSync(children, ctx);
  }

  if (tag === Suspense) {
    const id = `S:${++ctx.boundaryId}`;
    const fallback = props?.fallback;
    ctx.boundaries.push({ id, state: "resolved", fallback: null, promise: null });
    ctx.suspenseStack = ctx.suspenseStack || [];
    ctx.suspenseStack.push({ id, fallback });
    try {
      const html = renderChildrenSync(children, ctx);
      ctx.suspenseStack.pop();
      return html;
    } catch (error) {
      ctx.suspenseStack.pop();
      return handleRenderErrorSync(error, ctx);
    }
  }

  if (tag === ErrorBoundary) {
    ctx.errorStack = ctx.errorStack || [];
    ctx.errorStack.push({ fallback: props?.fallback });
    try {
      const html = renderChildrenSync(children, ctx);
      ctx.errorStack.pop();
      return html;
    } catch (error) {
      ctx.errorStack.pop();
      return renderFallbackSync(props?.fallback, error, ctx);
    }
  }

  if (typeof tag === "function") {
    let result;
    try {
      result = tag({ ...props, children });
    } catch (error) {
      return handleRenderErrorSync(error, ctx);
    }

    if (isPromiseLike(result)) {
      return handlePendingSync(result, ctx);
    }

    if (isResultErr(result)) {
      return handleRenderErrorSync(result.error ?? new Error(String(result)), ctx);
    }

    return renderNodeSync(result, ctx);
  }

  if (typeof tag === "string") {
    const attrs = renderAttributes(props);
    const childHtml = renderChildrenSync(children, ctx);
    if (childHtml === "" && voidElements.has(tag)) {
      return `<${tag}${attrs} />`;
    }
    return `<${tag}${attrs}>${childHtml}</${tag}>`;
  }

  return "";
}

async function renderNodeAsync(node, ctx) {
  if (node == null || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") {
    return escapeHtml(String(node));
  }
  if (Array.isArray(node)) {
    let out = "";
    for (const child of node) {
      out += await renderNodeAsync(child, ctx);
    }
    return out;
  }
  if (!isComponentNode(node)) {
    return escapeHtml(String(node));
  }

  const { tag, props, children } = node;

  if (tag === Fragment) {
    return await renderChildrenAsync(children, ctx);
  }

  if (tag === Suspense) {
    const id = `S:${++ctx.boundaryId}`;
    const fallback = props?.fallback;
    ctx.boundaries.push({ id, state: "resolved", fallback: null, promise: null });
    ctx.suspenseStack = ctx.suspenseStack || [];
    ctx.suspenseStack.push({ id, fallback });
    try {
      const html = await renderChildrenAsync(children, ctx);
      ctx.suspenseStack.pop();
      return html;
    } catch (error) {
      ctx.suspenseStack.pop();
      return await handleRenderErrorAsync(error, ctx);
    }
  }

  if (tag === ErrorBoundary) {
    ctx.errorStack = ctx.errorStack || [];
    ctx.errorStack.push({ fallback: props?.fallback });
    try {
      const html = await renderChildrenAsync(children, ctx);
      ctx.errorStack.pop();
      return html;
    } catch (error) {
      ctx.errorStack.pop();
      return await renderFallbackAsync(props?.fallback, error, ctx);
    }
  }

  if (typeof tag === "function") {
    let result;
    try {
      result = tag({ ...props, children });
    } catch (error) {
      return await handleRenderErrorAsync(error, ctx);
    }

    if (isPromiseLike(result)) {
      try {
        result = await handlePendingAsync(result, ctx);
      } catch (error) {
        return await handleRenderErrorAsync(error, ctx);
      }
    }

    if (isResultErr(result)) {
      return await handleRenderErrorAsync(result.error ?? new Error(String(result)), ctx);
    }

    return await renderNodeAsync(result, ctx);
  }

  if (typeof tag === "string") {
    const attrs = renderAttributes(props);
    const childHtml = await renderChildrenAsync(children, ctx);
    if (childHtml === "" && voidElements.has(tag)) {
      return `<${tag}${attrs} />`;
    }
    return `<${tag}${attrs}>${childHtml}</${tag}>`;
  }

  return "";
}

function createRendererContext(mode) {
  return {
    mode,
    boundaryId: 0,
    boundaries: [],
    suspenseStack: [],
    errorStack: [],
  };
}

function renderToString(node) {
  const ctx = createRendererContext("sync");
  const html = renderNodeSync(node, ctx);
  return { html, boundaries: ctx.boundaries };
}

async function renderToStringAsync(node) {
  const ctx = createRendererContext("async");
  const html = await renderNodeAsync(node, ctx);
  return { html, boundaries: ctx.boundaries };
}

// ---------------------------------------------------------------------------
// Signal-based reactivity primitives (deka#142)
// ---------------------------------------------------------------------------

const signalContextStack = [];

function getCurrentSignalContext() {
  return signalContextStack[signalContextStack.length - 1] || null;
}

function createSignal(initialValue) {
  let value = initialValue;
  const subscribers = new Set();

  function read() {
    const ctx = getCurrentSignalContext();
    if (ctx) {
      subscribers.add(ctx);
      ctx.onCleanup(() => {
        subscribers.delete(ctx);
      });
    }
    return value;
  }

  function write(nextValue) {
    if (Object.is(value, nextValue)) {
      return;
    }
    value = nextValue;
    const snapshot = Array.from(subscribers);
    for (const ctx of snapshot) {
      ctx.execute();
    }
  }

  return [read, write];
}

function createEffect(fn) {
  let userCleanup;
  const dependencyCleanups = new Set();

  const context = {
    execute,
    onCleanup(cleanup) {
      dependencyCleanups.add(cleanup);
    },
  };

  function execute() {
    for (const cleanup of dependencyCleanups) {
      cleanup();
    }
    dependencyCleanups.clear();

    if (typeof userCleanup === "function") {
      const previousCleanup = userCleanup;
      userCleanup = undefined;
      try {
        previousCleanup();
      } catch (error) {
        // User cleanup errors are intentionally swallowed so a misbehaving
        // cleanup does not prevent the effect from re-subscribing.
      }
    }

    signalContextStack.push(context);
    try {
      const maybeCleanup = fn();
      if (typeof maybeCleanup === "function") {
        userCleanup = maybeCleanup;
      }
    } finally {
      signalContextStack.pop();
    }
  }

  execute();

  return function dispose() {
    for (const cleanup of dependencyCleanups) {
      cleanup();
    }
    dependencyCleanups.clear();
    if (typeof userCleanup === "function") {
      const cleanup = userCleanup;
      userCleanup = undefined;
      cleanup();
    }
  };
}

function createMemo(fn) {
  const [getValue, setValue] = createSignal(undefined);
  createEffect(() => {
    setValue(fn());
  });
  return getValue;
}

// ---------------------------------------------------------------------------
// Zustand-style state store primitive (deka#143)
// ---------------------------------------------------------------------------
// Built on top of createSignal so multiple components / effects share the same
// reactive backing value. Actions are plain functions that receive the current
// state and optional arguments and return the next state.
//
//   const useCounter = State.create({ count: 0 }, {
//     increment: (s) => ({ count: s.count + 1 }),
//     add: (s, n) => ({ count: s.count + n }),
//   });
//
//   const state = useCounter();   // read current state
//   useCounter.increment();       // dispatch action

function createState(initialState, actions = {}) {
  const [getState, setState] = createSignal(initialState);

  function useStore() {
    return getState();
  }

  for (const key of Object.keys(actions)) {
    const action = actions[key];
    useStore[key] = function storeAction(...args) {
      setState(action(getState(), ...args));
    };
  }

  return useStore;
}

const State = Object.freeze({
  create: createState,
});

const deka = {
  unsafe: (tryFn, catchFn, finallyFn) => {
    try {
      return tryFn();
    } catch (error) {
      if (typeof catchFn === "function") {
        return catchFn(error);
      }
      throw error;
    } finally {
      if (typeof finallyFn === "function") {
        finallyFn();
      }
    }
  },

  panic: (message) => {
    throw new Error(String(message));
  },

  ui: Object.freeze({
    jsx: uiJsx,
    jsxs: uiJsxs,
    Fragment,
    Suspense,
    ErrorBoundary,
    renderToString,
    renderToStringAsync,
    signal: createSignal,
    effect: createEffect,
    memo: createMemo,
    State,
    createSignal,
    createEffect,
    createMemo,
    createState,
  }),
};

const unsafeGlobals = {
  fetch: hostFetch,
  JSON: hostJSON,
  URL: hostURL,
  URLSearchParams: hostURLSearchParams,
  TextEncoder: hostTextEncoder,
  TextDecoder: hostTextDecoder,
  Blob: hostBlob,
  FormData: hostFormData,
  Headers: hostHeaders,
  Request: hostRequest,
  Response: hostResponse,
  atob: hostAtob,
  btoa: hostBtoa,
  structuredClone: hostStructuredClone,
  crypto: hostCrypto,
  setTimeout: hostSetTimeout,
  setInterval: hostSetInterval,
  clearTimeout: hostClearTimeout,
  clearInterval: hostClearInterval,
  queueMicrotask: hostQueueMicrotask,
};

const safeFetch = async (input, init) => {
  try {
    return ok(await hostFetch(input, init));
  } catch (error) {
    return err(error);
  }
};

const safeJSON = {
  parse: (text) => {
    try {
      return ok(hostJSON.parse(text));
    } catch (error) {
      return err(error);
    }
  },
  stringify: hostJSON.stringify.bind(hostJSON),
};

function renderJsxChildren(children) {
  if (children == null) return "";
  if (typeof children === "string" || typeof children === "number") return String(children);
  if (Array.isArray(children)) return children.map(renderJsxChildren).join("");
  return String(children ?? "");
}

// Legacy test-harness JSX renderer (string-concat HTML). These shims keep any
// source that directly calls the old bare `jsx`/`jsxs` globals working. The
// compiler now emits `deka.ui.*`, so new code should use that namespace.
function legacyJsx(type, props) {
  const resolvedProps = props ?? {};
  if (type === Fragment) {
    return renderJsxChildren(resolvedProps.children);
  }
  if (typeof type === "function") {
    const result = type(resolvedProps);
    if (isComponentNode(result)) {
      // Boundary components (Suspense / ErrorBoundary) return immutable nodes
      // that must be rendered by the server renderer, even through the legacy
      // JSX shim that the current compiler slice emits.
      return renderToString(result).html;
    }
    return String(result ?? "");
  }
  const { children, ...attributes } = resolvedProps;
  const attrs = Object.entries(attributes)
    .map(([key, value]) => {
      if (value === true) return ` ${key}`;
      if (value === false || value == null) return "";
      const escaped = String(value).replace(/&/g, "&amp;").replace(/"/g, "&quot;");
      return ` ${key}="${escaped}"`;
    })
    .join("");
  const childHtml = renderJsxChildren(children);
  return childHtml === "" ? `<${type}${attrs} />` : `<${type}${attrs}>${childHtml}</${type}>`;
}

function legacyJsxs(type, props) {
  return legacyJsx(type, props);
}

export function createRuntimeGlobals(stdout, stderr, cwd = "/", env = {}) {
  const output = [];
  const errorOutput = [];

  function format(args) {
    return args.map((a) => (typeof a === "string" ? a : String(a))).join(" ");
  }

  const stdoutWriter = stdout ?? {
    write: (value) => {
      output.push(value);
    },
  };

  const stderrWriter = stderr ?? {
    write: (value) => {
      errorOutput.push(value);
    },
  };

  const fs = {
    readFile: (path) => {
      const resolvedPath = isAbsolute(path) ? path : resolve(cwd, path);
      if (!existsSync(resolvedPath)) {
        return { ok: false, error: new Error(`File not found: ${resolvedPath}`) };
      }
      try {
        return { ok: true, value: readFileSync(resolvedPath, "utf-8") };
      } catch (error) {
        return { ok: false, error };
      }
    },
    exists: (path) => {
      const resolvedPath = isAbsolute(path) ? path : resolve(cwd, path);
      return existsSync(resolvedPath);
    },
    writeFile: (path, content) => {
      const resolvedPath = isAbsolute(path) ? path : resolve(cwd, path);
      try {
        mkdirSync(dirname(resolvedPath), { recursive: true });
        writeFileSync(resolvedPath, content, "utf-8");
        return { ok: true, value: undefined };
      } catch (error) {
        return { ok: false, error };
      }
    },
    isDirectory: (path) => {
      const resolvedPath = isAbsolute(path) ? path : resolve(cwd, path);
      return existsSync(resolvedPath) && statSync(resolvedPath).isDirectory();
    },
  };

  return {
    globals: {
      __dekaPrint: (value) => {
        stdoutWriter.write(String(value));
      },

      console: {
        log: (...args) => stdoutWriter.write(format(args) + "\n"),
        info: (...args) => stdoutWriter.write(format(args) + "\n"),
        warn: (...args) => stderrWriter.write(format(args) + "\n"),
        error: (...args) => stderrWriter.write(format(args) + "\n"),
      },

      process: {
        env,
        cwd: () => cwd,
      },

      __dekaFs: fs,

      deka,

      Option: Object.freeze({
        Some: (value) => Object.freeze({ __enum: "Option", __case: "Some", value }),
        None: Object.freeze({ __enum: "Option", __case: "None" }),
      }),

      Result: Object.freeze({
        Ok: (value) => Object.freeze({ __enum: "Result", __case: "Ok", value }),
        Err: (error) => Object.freeze({ __enum: "Result", __case: "Err", error }),
      }),

      unsafe: unsafeGlobals,

      fetch: safeFetch,
      JSON: hostJSON,
      URL: wrapConstructorResult(hostURL),
      URLSearchParams: wrapConstructorResult(hostURLSearchParams),
      TextEncoder: wrapConstructorResult(hostTextEncoder),
      TextDecoder: wrapConstructorResult(hostTextDecoder),
      Blob: wrapConstructorResult(hostBlob),
      FormData: wrapConstructorResult(hostFormData),
      Headers: wrapConstructorResult(hostHeaders),
      Request: wrapConstructorResult(hostRequest),
      Response: wrapConstructorResult(hostResponse),

      atob: hostAtob,
      btoa: hostBtoa,
      structuredClone: hostStructuredClone,
      crypto: hostCrypto,

      setTimeout: wrapTimer(hostSetTimeout),
      setInterval: wrapTimer(hostSetInterval),
      clearTimeout: hostClearTimeout,
      clearInterval: hostClearInterval,
      queueMicrotask: wrapMicrotask(hostQueueMicrotask),

      Math,
      Array,
      Object,
      Date,
      Map,
      Set,

      jsx: legacyJsx,
      jsxs: legacyJsxs,

      Suspense,
      ErrorBoundary,

      createSignal,
      createEffect,
      createMemo,

      State,
    },
    output,
    errorOutput,
  };
}
