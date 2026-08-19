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
  return Object.freeze({
    tag,
    props: Object.freeze(rest),
    children: Object.freeze(normalizedChildren),
  });
}

function uiJsx(tag, props) {
  return createComponentNode(tag, props);
}

function uiJsxs(tag, props) {
  return createComponentNode(tag, props);
}

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

// Legacy test-harness JSX renderer (string-concat HTML). The current compiler
// emits `jsx`/`jsxs` as globals imported from `component/core`; issue #146 moves
// the compiler to the `deka.ui.*` namespace. These legacy shims keep the
// existing runtime-suite fixtures rendering to stdout until the compiler switch
// lands. They are NOT the public JSX runtime contract.
function legacyJsx(type, props) {
  const resolvedProps = props ?? {};
  if (typeof type === "function") {
    return String(type(resolvedProps) ?? "");
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
    },
    output,
    errorOutput,
  };
}
