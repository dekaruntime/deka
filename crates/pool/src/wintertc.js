// Minimal WinterTC (Minimum Common Web API) for Workers-style handlers:
//   export default { async fetch(request) { return new Response("ok"); } }
// Host HTTP still serializes to { status, headers, body }. This file only
// gives user JS Request/Response/Headers and a fetch dispatch.

if (typeof globalThis.Headers !== "function") {
  class Headers {
    constructor(init) {
      this._map = Object.create(null);
      if (!init) return;
      if (typeof init.forEach === "function") {
        init.forEach((value, key) => this.append(key, value));
      } else if (Array.isArray(init)) {
        for (const pair of init) {
          if (pair && pair.length >= 2) this.append(pair[0], pair[1]);
        }
      } else {
        for (const key in init) this.append(key, init[key]);
      }
    }
    _key(name) {
      return String(name).toLowerCase();
    }
    append(name, value) {
      const key = this._key(name);
      const next = String(value);
      this._map[key] = this._map[key] ? this._map[key] + ", " + next : next;
      this[key] = this._map[key];
    }
    set(name, value) {
      const key = this._key(name);
      this._map[key] = String(value);
      this[key] = this._map[key];
    }
    get(name) {
      const value = this._map[this._key(name)];
      return value === undefined ? null : value;
    }
    has(name) {
      return Object.prototype.hasOwnProperty.call(this._map, this._key(name));
    }
    delete(name) {
      delete this._map[this._key(name)];
    }
    forEach(callback, thisArg) {
      for (const key in this._map) {
        callback.call(thisArg, this._map[key], key, this);
      }
    }
  }
  globalThis.Headers = Headers;
}

if (typeof globalThis.Request !== "function") {
  class Request {
    constructor(input, init) {
      init = init || {};
      if (typeof input === "string") {
        this.url = input;
      } else if (input && typeof input.url === "string") {
        this.url = input.url;
        if (init.method == null) init.method = input.method;
        if (init.headers == null) init.headers = input.headers;
        if (init.body == null && input._body != null) init.body = input._body;
      } else {
        this.url = "http://localhost/";
      }
      this.method = String(init.method || "GET").toUpperCase();
      this.headers =
        init.headers instanceof globalThis.Headers
          ? init.headers
          : new globalThis.Headers(init.headers || {});
      this._body = init.body == null ? "" : String(init.body);
      // Deka generated handlers (and CF-style fetch) read these as fields.
      this.body = this._body;
      try {
        const parsed = new URL(this.url, "http://localhost");
        this.pathname = parsed.pathname || "/";
        this.path = parsed.pathname + parsed.search || "/";
      } catch (_err) {
        this.pathname = "/";
        this.path = "/";
      }
      if (init.pathname) this.pathname = String(init.pathname);
      if (init.path) this.path = String(init.path);
    }
    async text() {
      return this._body;
    }
    async json() {
      if (!this._body) return null;
      return JSON.parse(this._body);
    }
    clone() {
      return new Request(this.url, {
        method: this.method,
        headers: this.headers,
        body: this._body,
      });
    }
  }
  globalThis.Request = Request;
}

if (typeof globalThis.Response !== "function") {
  class Response {
    constructor(body, init) {
      init = init || {};
      this.status = typeof init.status === "number" ? init.status : 200;
      this.statusText = init.statusText != null ? String(init.statusText) : "";
      this.ok = this.status >= 200 && this.status < 300;
      this.headers =
        init.headers instanceof globalThis.Headers
          ? init.headers
          : new globalThis.Headers(init.headers || {});
      if (body == null) {
        this._body = "";
        this.body = "";
      } else if (typeof body === "string") {
        this._body = body;
        this.body = body;
      } else if (body instanceof Uint8Array) {
        this._bytes = body;
        this.body = body;
        this._body = null;
      } else if (body instanceof ArrayBuffer) {
        this._bytes = new Uint8Array(body);
        this.body = this._bytes;
        this._body = null;
      } else {
        this._body = JSON.stringify(body);
        this.body = this._body;
      }
    }
    async text() {
      if (this._body != null) return this._body;
      if (!this._bytes) return "";
      let out = "";
      for (let i = 0; i < this._bytes.length; i++) {
        out += String.fromCharCode(this._bytes[i]);
      }
      return out;
    }
    async json() {
      return JSON.parse(await this.text());
    }
    async arrayBuffer() {
      if (this._bytes) return this._bytes.buffer;
      const text = this._body || "";
      const bytes = new Uint8Array(text.length);
      for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i) & 0xff;
      return bytes.buffer;
    }
  }
  globalThis.Response = Response;
}

if (typeof globalThis.__dekaExecuteRequest !== "function") {
  globalThis.__dekaExecuteRequest = async function () {
    function base64Encode(bytes) {
      if (typeof btoa === "function") {
        let binary = "";
        for (let i = 0; i < bytes.length; i += 1) {
          binary += String.fromCharCode(bytes[i]);
        }
        return btoa(binary);
      }
      const alphabet =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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

    const requestData = globalThis.__requestData || {};
    requestData.__body = requestData.body ?? "";
    requestData.params = requestData.params || {};
    if (typeof requestData.json !== "function") {
      requestData.json = async function () {
        const body = this.__body || "";
        if (!body) return null;
        return JSON.parse(body);
      };
    }
    if (typeof requestData.text !== "function") {
      requestData.text = async function () {
        return this.__body || "";
      };
    }

    const method = String(requestData.method || "GET").toUpperCase();
    const url = requestData.url || "http://localhost/";
    const init = { method, headers: requestData.headers || {} };
    if (requestData.__body && method !== "GET" && method !== "HEAD") {
      init.body = requestData.__body;
    }
    if (requestData.pathname) init.pathname = requestData.pathname;
    if (requestData.path) init.path = requestData.path;
    const request = new Request(url, init);

    const context = globalThis.__requestContext || requestData.context || null;
    const handler = globalThis.app;
    if (!handler) {
      throw new Error(
        'Handler did not export default { fetch } (or define globalThis.app)'
      );
    }

    const wsEvent = requestData.__dekaWsEvent;
    if (wsEvent) {
      const wsHandler = handler.websocket || globalThis.__dekaWebsocket;
      if (wsHandler) {
        const ws = globalThis.__dekaWsCreate
          ? globalThis.__dekaWsCreate(
              requestData.__dekaWsId,
              requestData.__dekaWsData
            )
          : null;
        if (
          wsEvent === "message" &&
          requestData.__dekaWsBinary &&
          Array.isArray(requestData.__dekaWsMessage)
        ) {
          requestData.__dekaWsMessage = new Uint8Array(
            requestData.__dekaWsMessage
          );
        }
        if (wsEvent === "open" && typeof wsHandler.open === "function") {
          wsHandler.open(ws);
        } else if (
          wsEvent === "message" &&
          typeof wsHandler.message === "function"
        ) {
          wsHandler.message(ws, requestData.__dekaWsMessage);
        } else if (wsEvent === "close" && typeof wsHandler.close === "function") {
          wsHandler.close(
            ws,
            requestData.__dekaWsCode,
            requestData.__dekaWsReason
          );
        } else if (wsEvent === "drain" && typeof wsHandler.drain === "function") {
          wsHandler.drain(ws);
        }
      }
      return { status: 204, headers: {}, body: "" };
    }

    let response;
    if (typeof handler.fetch === "function") {
      response = await handler.fetch(request, context);
    } else if (typeof handler === "function") {
      response = await handler(request, context);
    } else {
      throw new Error("Handler is not callable and has no fetch()");
    }

    const normalized = globalThis.__dekaResponse ||
      (globalThis.__dekaResponse = {
        status: 200,
        headers: {},
        body: "",
        body_base64: undefined,
        upgrade: undefined,
      });
    normalized.status = 200;
    normalized.body = "";
    normalized.body_base64 = undefined;
    normalized.upgrade = undefined;
    const headerTarget = normalized.headers;
    for (const key in headerTarget) delete headerTarget[key];

    const applyHeaders = (headers) => {
      if (!headers) return;
      if (typeof headers.forEach === "function") {
        headers.forEach((value, key) => {
          headerTarget[key] = String(value);
        });
        return;
      }
      for (const key in headers) headerTarget[key] = String(headers[key]);
    };

    if (response && typeof response.text === "function") {
      if (typeof response.status === "number") {
        normalized.status = response.status;
      }
      applyHeaders(response.headers);
      if (response.upgrade) normalized.upgrade = response.upgrade;
      const bodyValue = response.body;
      if (bodyValue instanceof Uint8Array) {
        normalized.body_base64 = base64Encode(bodyValue);
      } else if (bodyValue instanceof ArrayBuffer) {
        normalized.body_base64 = base64Encode(new Uint8Array(bodyValue));
      } else {
        const contentType = String(
          headerTarget["content-type"] || headerTarget["Content-Type"] || ""
        ).toLowerCase();
        const isTextLike =
          contentType.startsWith("text/") ||
          contentType.includes("json") ||
          contentType.includes("javascript") ||
          contentType.includes("xml") ||
          contentType.includes("svg") ||
          contentType.includes("x-www-form-urlencoded") ||
          !contentType;
        if (!isTextLike && typeof response.arrayBuffer === "function") {
          const bytes = new Uint8Array(await response.arrayBuffer());
          normalized.body_base64 = base64Encode(bytes);
        } else {
          normalized.body = await response.text();
        }
      }
    } else if (response && typeof response === "object") {
      if (typeof response.status === "number") {
        normalized.status = response.status;
      }
      applyHeaders(response.headers);
      if (typeof response.body_base64 === "string") {
        normalized.body_base64 = response.body_base64;
      }
      if (response.body != null) {
        if (response.body instanceof Uint8Array) {
          normalized.body_base64 = base64Encode(response.body);
        } else if (response.body instanceof ArrayBuffer) {
          normalized.body_base64 = base64Encode(new Uint8Array(response.body));
        } else if (typeof response.body === "string") {
          normalized.body = response.body;
        } else {
          normalized.body = JSON.stringify(response.body);
        }
      }
      if (response.upgrade) normalized.upgrade = response.upgrade;
    } else if (response != null) {
      normalized.body = String(response);
    }

    return normalized;
  };
}
