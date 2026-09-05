// ui/router — App() dispatch for generated api/worker entries.
// Hosts materialize this file; generated .ds imports it rather than inlining
// path/method/Option lowering as source text.

const OPAQUE_500 = {
  status: 500,
  body: "Internal Server Error",
  headers: { location: "" },
};

function staticPrefix(route) {
  const parts = [];
  for (const part of String(route || "").replace(/^\/+|\/+$/g, "").split("/")) {
    if (!part) continue;
    if (part.startsWith("[")) break;
    parts.push(part);
  }
  if (parts.length === 0) return "/";
  return "/" + parts.join("/") + "/";
}

function oneSegmentAfter(path, prefix) {
  const p = String(path || "");
  const pre = String(prefix || "");
  if (!p.startsWith(pre)) return false;
  const rest = p.slice(pre.length);
  if (!rest || rest.indexOf("/") >= 0) return false;
  return true;
}

function routeMatches(route, path) {
  if (route === path) return true;
  if (String(route).indexOf("[") < 0) return false;
  return oneSegmentAfter(path, staticPrefix(route));
}

function callHandler(fn, request) {
  try {
    return fn(request);
  } catch (_) {
    return OPAQUE_500;
  }
}

export function runApiRouter(request, routes) {
  const path = request && request.pathname === "" ? "/" : (request && request.pathname) || "/";
  const method = String((request && request.method) || "GET");
  const table = routes || {};
  let handlers = null;
  for (const route of Object.keys(table)) {
    if (routeMatches(route, path)) {
      handlers = table[route];
      break;
    }
  }
  if (!handlers) {
    if (path === "/api" || path.startsWith("/api/")) {
      return { status: 404, body: "Not found", headers: { location: "" } };
    }
    return { status: 404, body: "Not found", headers: { location: "" } };
  }
  let fn = handlers[method];
  if (typeof fn !== "function" && method === "HEAD" && typeof handlers.GET === "function") {
    fn = handlers.GET;
  }
  if (typeof fn !== "function") {
    return { status: 405, body: "Method not allowed", headers: { location: "" } };
  }
  return callHandler(fn, request);
}
