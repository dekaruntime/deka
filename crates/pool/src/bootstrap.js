const __print = (Deno && Deno.core && typeof Deno.core.print === 'function')
    ? Deno.core.print.bind(Deno.core)
    : function() {};
const __ops = (Deno && Deno.core && Deno.core.ops) ? Deno.core.ops : {};

if (typeof globalThis.__dekaPrint !== 'function') {
    globalThis.__dekaPrint = (value, isErr = false) => {
        const text = value == null ? '' : String(value);
        __print(text, !!isErr);
    };
}

// `process` is a capability-gated host bridge, not a web
// standard. Keep it in the host bootstrap rather than the
// WinterTC surface.
if (typeof __ops.op_php_env_capability_granted === 'function'
    && __ops.op_php_env_capability_granted()) {
    if (!globalThis.process) globalThis.process = {};
    if (!globalThis.process.env) globalThis.process.env = {};
    if (!globalThis.process.cwd) {
        globalThis.process.cwd = () => __ops.op_php_cwd();
    }
} else if (globalThis.process) {
    delete globalThis.process;
}

// Enum prelude is injected below from crate::prelude (deka#582).
/*__DEKA_POOL_ENUM_PRELUDE__*/

// Signal-based reactivity primitives (deka#142)
if (typeof globalThis.deka === 'undefined') {
    globalThis.deka = {};
}

if (typeof globalThis.deka.panic !== 'function') {
    globalThis.deka.panic = (msg) => { throw new Error(String(msg)); };
}

if (typeof globalThis.crypto === 'undefined' || typeof globalThis.crypto.randomUUID !== 'function') {
    const fill = (bytes) => {
        if (typeof __ops.op_php_random_bytes === 'function') {
            const raw = __ops.op_php_random_bytes(bytes.length);
            if (raw && typeof raw.length === 'number') {
                bytes.set(raw);
                return;
            }
        }
        for (let i = 0; i < bytes.length; i++) bytes[i] = (Math.random() * 256) | 0;
    };
    globalThis.crypto = Object.assign({}, globalThis.crypto || {}, {
        getRandomValues: (arr) => {
            fill(new Uint8Array(arr.buffer, arr.byteOffset, arr.byteLength));
            return arr;
        },
        randomUUID: () => {
            const b = new Uint8Array(16);
            fill(b);
            b[6] = (b[6] & 0x0f) | 0x40;
            b[8] = (b[8] & 0x3f) | 0x80;
            const h = Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
            return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
        }
    });
}
const __dekaSignalContextStack = [];
function __dekaGetSignalContext() {
    return __dekaSignalContextStack[__dekaSignalContextStack.length - 1] || null;
}
function __dekaCreateSignal(initialValue) {
    let value = initialValue;
    const subscribers = new Set();
    function read() {
        const ctx = __dekaGetSignalContext();
        if (ctx) {
            subscribers.add(ctx);
            ctx.onCleanup(() => { subscribers.delete(ctx); });
        }
        return value;
    }
    function write(nextValue) {
        if (Object.is(value, nextValue)) return;
        value = nextValue;
        for (const ctx of Array.from(subscribers)) ctx.execute();
    }
    // Callable pair: `const count = signal(0); count()` reads,
    // and `const [get, set] = signal(0)` still destructures.
    // JSX `{count()}` must prerender the initial value (dsc#82).
    // A Proxy around an array is not callable (apply traps
    // require a function target), so stamp the tuple onto the
    // getter itself.
    read[0] = read;
    read[1] = write;
    read[Symbol.iterator] = function* () { yield read; yield write; };
    return read;
}
function __dekaLive(fn) {
    if (typeof fn === "function") return fn();
    return fn;
}
function __dekaCreateEffect(fn) {
    let userCleanup;
    const dependencyCleanups = new Set();
    function execute() {
        for (const c of dependencyCleanups) c();
        dependencyCleanups.clear();
        if (typeof userCleanup === 'function') { const c = userCleanup; userCleanup = undefined; try { c(); } catch (e) {} }
        __dekaSignalContextStack.push(context);
        try {
            const maybeCleanup = fn();
            if (typeof maybeCleanup === 'function') userCleanup = maybeCleanup;
        } finally { __dekaSignalContextStack.pop(); }
    }
    const context = { execute, onCleanup(c) { dependencyCleanups.add(c); } };
    execute();
    return function dispose() {
        for (const c of dependencyCleanups) c();
        dependencyCleanups.clear();
        if (typeof userCleanup === 'function') { const c = userCleanup; userCleanup = undefined; c(); }
    };
}
function __dekaCreateMemo(fn) {
    const [getValue, setValue] = __dekaCreateSignal(undefined);
    __dekaCreateEffect(() => { setValue(fn()); });
    return getValue;
}
globalThis.createSignal = __dekaCreateSignal;
globalThis.createEffect = __dekaCreateEffect;
globalThis.createMemo = __dekaCreateMemo;
globalThis.live = __dekaLive;
globalThis.deka.ui = Object.freeze({
    signal: __dekaCreateSignal,
    effect: __dekaCreateEffect,
    memo: __dekaCreateMemo,
    live: __dekaLive,
    createSignal: __dekaCreateSignal,
    createEffect: __dekaCreateEffect,
    createMemo: __dekaCreateMemo,
    renderToString(node) {
        const fn = globalThis[Symbol.for("deka.react.renderToString")];
        if (typeof fn !== "function") {
            throw new Error("deka.ui.renderToString requires React SSR");
        }
        return { html: fn(node) };
    },
    renderToStreamHtml(node) {
        const fn = globalThis[Symbol.for("deka.react.renderToString")];
        if (typeof fn !== "function") {
            return Promise.reject(new Error("deka.ui.renderToStreamHtml requires React SSR"));
        }
        return Promise.resolve(fn(node));
    },
});

// Performance API polyfill
if (typeof globalThis.performance === 'undefined') {
    const startTime = Date.now();
    globalThis.performance = {
        now() {
            return Date.now() - startTime;
        }
    };
}

// Minimal URL polyfill for parsing URLs lives in wintertc.js
// (inlined by the WinterTC marker further down); keeping it out
// of this file preserves the file-size gate baseline (deka#391).

// Runtime bridge helpers for PHPX stdlib (JS runtime path)
if (typeof globalThis.function_exists !== 'function') {
    globalThis.function_exists = function(name) {
        const n = String(name || '');
        if (n === '__bridge') {
            return true;
        }
        return typeof globalThis[n] === 'function';
    };
}

if (typeof globalThis.is_array !== 'function') {
    globalThis.is_array = function(value) {
        return Array.isArray(value);
    };
}
if (typeof globalThis.is_string !== 'function') {
    globalThis.is_string = function(value) {
        return typeof value === 'string';
    };
}
if (typeof globalThis.is_int !== 'function') {
    globalThis.is_int = function(value) {
        return typeof value === 'number' && Number.isInteger(value);
    };
}
if (typeof globalThis.is_float !== 'function') {
    globalThis.is_float = function(value) {
        return typeof value === 'number' && !Number.isNaN(value) && !Number.isInteger(value);
    };
}
if (typeof globalThis.is_bool !== 'function') {
    globalThis.is_bool = function(value) {
        return typeof value === 'boolean';
    };
}
if (typeof globalThis.is_object !== 'function') {
    globalThis.is_object = function(value) {
        return value !== null && typeof value === 'object' && !Array.isArray(value);
    };
}
if (typeof globalThis.is_numeric !== 'function') {
    globalThis.is_numeric = function(value) {
        if (typeof value === 'number') {
            return !Number.isNaN(value) && Number.isFinite(value);
        }
        if (typeof value === 'string') {
            if (value.trim() === '') return false;
            const num = Number(value);
            return !Number.isNaN(num) && Number.isFinite(num);
        }
        return false;
    };
}
if (typeof globalThis.is_callable !== 'function') {
    globalThis.is_callable = function(value) {
        return typeof value === 'function';
    };
}
if (typeof globalThis.gettype !== 'function') {
    globalThis.gettype = function(value) {
        if (value === null || value === undefined) return 'NULL';
        if (Array.isArray(value)) return 'array';
        const t = typeof value;
        if (t === 'string') return 'string';
        if (t === 'boolean') return 'boolean';
        if (t === 'number') return Number.isInteger(value) ? 'integer' : 'double';
        if (t === 'object') return 'object';
        if (t === 'function') return 'object';
        return 'unknown';
    };
}

if (!globalThis[Symbol.for('deka.host.internal')]) {
    const ops = __ops;
    const routeHostCall = (kind, action, payload) => {
        if (kind === 'db') {
            if (typeof ops.op_php_db_call_proto === 'function' && typeof ops.op_php_db_proto_encode === 'function' && typeof ops.op_php_db_proto_decode === 'function') {
                const request = ops.op_php_db_proto_encode(String(action || ''), payload || {});
                const response = ops.op_php_db_call_proto(request);
                return ops.op_php_db_proto_decode(response);
            }
            return { ok: false, error: 'db protobuf bridge ops unavailable' };
        }
        if (kind === 'shard') {
            if (typeof ops.op_shard_for === 'function') {
                const p = payload || {};
                const act = String(action || '');
                const shardKey = (act === 'self' || !p.shop_id)
                    ? globalThis.__shardKey || globalThis.__shopId || ''
                    : p.shop_id;
                return ops.op_shard_for(String(shardKey || ''));
            }
            return { ok: false, error: 'shard bridge op unavailable' };
        }
        if (kind === 'vault') {
            const act = String(action || '');
            const secrets = globalThis.__dekaShopSecrets || {};
            if (!globalThis.__shopId) {
                return Object.entries({ ok: false, error: 'no_shop_context' });
            }
            if (act === 'get') {
                const req = payload || {};
                const name = String(req.name || req.key || '');
                if (!name || name.includes('/')) {
                    return Object.entries({ ok: false, error: 'invalid_key' });
                }
                if (Object.prototype.hasOwnProperty.call(secrets, name)) {
                    return Object.entries({ ok: true, value: String(secrets[name]) });
                }
                return Object.entries({ ok: false, error: 'not_found' });
            }
            if (act === 'list') {
                return Object.entries({ ok: true, keys: Object.keys(secrets).sort() });
            }
            return Object.entries({ ok: false, error: `unknown vault action '${act}'` });
        }
        if (kind === 'net') {
            if (typeof ops.op_php_net_call_proto === 'function' && typeof ops.op_php_net_proto_encode === 'function' && typeof ops.op_php_net_proto_decode === 'function') {
                const request = ops.op_php_net_proto_encode(String(action || ''), payload || {});
                const response = ops.op_php_net_call_proto(request);
                // Serde-backed ops decode to host objects. Cross the PHPX
                // boundary as key/value entries so the consumer can build a
                // keyed PHPX array, matching the fs bridge contract.
                const decoded = ops.op_php_net_proto_decode(response);
                return Object.entries(decoded || {});
            }
            return { ok: false, error: 'net protobuf bridge ops unavailable' };
        }
        if (kind === 'fs') {
            if (typeof ops.op_php_fs_call_proto === 'function' && typeof ops.op_php_fs_proto_encode === 'function' && typeof ops.op_php_fs_proto_decode === 'function') {
                const request = ops.op_php_fs_proto_encode(String(action || ''), payload || {});
                const response = ops.op_php_fs_call_proto(request);
                const decoded = ops.op_php_fs_proto_decode(response);
                return Object.entries(decoded || {});
            }
            return { ok: false, error: 'fs protobuf bridge ops unavailable' };
        }
        if (kind === 'time') {
            const act = String(action || '');
            const req = payload || {};
            if (act === 'now_ms') {
                return Object.entries({ ok: true, now_ms: Date.now() });
            }
            if (act === 'sleep_ms') {
                const msRaw = Number(req.milliseconds ?? req.ms ?? 0);
                const ms = Number.isFinite(msRaw) ? Math.max(0, Math.floor(msRaw)) : 0;
                try {
                    if (ms > 0) {
                        if (typeof SharedArrayBuffer !== 'undefined' && typeof Atomics !== 'undefined' && typeof Atomics.wait === 'function') {
                            const sab = new SharedArrayBuffer(4);
                            const arr = new Int32Array(sab);
                            Atomics.wait(arr, 0, 0, ms);
                        } else {
                            const end = Date.now() + ms;
                            while (Date.now() < end) {}
                        }
                    }
                    return Object.entries({ ok: true, slept_ms: ms });
                } catch (err) {
                    return Object.entries({ ok: false, error: err && err.message ? err.message : String(err) });
                }
            }
            return { ok: false, error: `unknown time action '${act}'` };
        }
        if (kind === 'crypto') {
            const act = String(action || '');
            const toBytes = (v) => {
                if (v instanceof Uint8Array) return v;
                if (typeof v === 'string') {
                    return (typeof TextEncoder !== 'undefined')
                        ? new TextEncoder().encode(v)
                        : Uint8Array.from(Array.from(v).map((ch) => ch.charCodeAt(0) & 0xff));
                }
                if (Array.isArray(v)) return new Uint8Array(v);
                if (v && typeof v.length === 'number') return new Uint8Array(Array.from(v));
                return null;
            };
            if (act === 'random_bytes') {
                const req = payload || {};
                const n = Number(req.length ?? req.len ?? 0);
                if (!Number.isFinite(n) || n <= 0) {
                    return { ok: false, error: 'length must be > 0' };
                }
                const bytes = new Uint8Array(Math.floor(n));
                let filled = false;
                if (!filled && typeof ops.op_php_random_bytes === 'function') {
                    const raw = ops.op_php_random_bytes(Math.floor(n));
                    if (raw && typeof raw.length === 'number') {
                        bytes.set(raw);
                        filled = true;
                    }
                }
                if (!filled && globalThis.crypto && typeof globalThis.crypto.getRandomValues === 'function') {
                    globalThis.crypto.getRandomValues(bytes);
                    filled = true;
                }
                if (!filled) {
                    return { ok: false, error: 'secure random source unavailable' };
                }
                return Object.entries({ ok: true, data: Array.from(bytes) });
            }
            // AES-256-GCM — used by @deka/payments for at-rest
            // encryption of provider OAuth tokens (issue #119).
            // Inputs come in as byte arrays (each entry 0..255).
            if (act === 'aes_256_gcm_encrypt' || act === 'aes_256_gcm_decrypt') {
                const req = payload || {};
                const toU8 = (v) => {
                    if (v instanceof Uint8Array) return v;
                    if (Array.isArray(v)) return new Uint8Array(v);
                    if (v && typeof v.length === 'number') return new Uint8Array(Array.from(v));
                    return new Uint8Array();
                };
                const key = toU8(req.key);
                const nonce = toU8(req.nonce);
                const input = toU8(act === 'aes_256_gcm_encrypt' ? req.plaintext : req.ciphertext);
                const aad = toU8(req.aad || []);
                const opName = act === 'aes_256_gcm_encrypt' ? 'op_php_aes_256_gcm_encrypt' : 'op_php_aes_256_gcm_decrypt';
                if (typeof ops[opName] !== 'function') {
                    return { ok: false, error: `${opName} unavailable` };
                }
                const raw = ops[opName](key, nonce, input, aad);
                // Rust op already returns {ok, data|error}. Normalize
                // data into a plain array for the PHPX caller.
                const okVal = raw && raw.ok === true;
                if (okVal) {
                    const data = raw.data;
                    const arr = data instanceof Uint8Array ? Array.from(data) : (Array.isArray(data) ? data : Array.from(data || []));
                    return Object.entries({ ok: true, data: arr });
                }
                return Object.entries({ ok: false, error: (raw && raw.error) || 'aes_op_failed' });
            }
            if (act === 'bcrypt_verify') {
                const req = payload || {};
                const password = String(req.password ?? '');
                const hash = String(req.hash ?? '');
                if (typeof ops.op_php_bcrypt_verify !== 'function') {
                    return { ok: false, error: 'op_php_bcrypt_verify unavailable' };
                }
                const raw = ops.op_php_bcrypt_verify(password, hash);
                return Object.entries(raw || { ok: false, error: 'bcrypt_verify_failed' });
            }
            if (act === 'digest') {
                const req = payload || {};
                const algorithm = String(req.algorithm ?? req.alg ?? '');
                const data = toBytes(req.data);
                if (data === null) {
                    return { ok: false, error: 'data must be bytes' };
                }
                if (typeof ops.op_php_digest !== 'function') {
                    return { ok: false, error: 'op_php_digest unavailable' };
                }
                const raw = ops.op_php_digest(algorithm, data);
                const okVal = raw && raw.ok === true;
                if (okVal) {
                    const arr = raw.data instanceof Uint8Array ? Array.from(raw.data) : (Array.isArray(raw.data) ? raw.data : Array.from(raw.data || []));
                    return Object.entries({ ok: true, data: arr });
                }
                return Object.entries({ ok: false, error: (raw && raw.error) || 'digest_failed' });
            }
            if (act === 'hmac') {
                const req = payload || {};
                const algorithm = String(req.algorithm ?? req.alg ?? '');
                const key = toBytes(req.key);
                const data = toBytes(req.data);
                if (key === null || data === null) {
                    return { ok: false, error: 'key and data must be bytes' };
                }
                if (typeof ops.op_php_hmac !== 'function') {
                    return { ok: false, error: 'op_php_hmac unavailable' };
                }
                const raw = ops.op_php_hmac(algorithm, key, data);
                const okVal = raw && raw.ok === true;
                if (okVal) {
                    const arr = raw.data instanceof Uint8Array ? Array.from(raw.data) : (Array.isArray(raw.data) ? raw.data : Array.from(raw.data || []));
                    return Object.entries({ ok: true, data: arr });
                }
                return Object.entries({ ok: false, error: (raw && raw.error) || 'hmac_failed' });
            }
            if (act === 'secure_compare') {
                const req = payload || {};
                const a = toBytes(req.a);
                const b = toBytes(req.b);
                if (a === null || b === null) {
                    return { ok: false, error: 'operands must be bytes' };
                }
                if (typeof ops.op_php_secure_compare === 'function') {
                    const raw = ops.op_php_secure_compare(a, b);
                    const okVal = raw && raw.ok === true;
                    if (okVal) {
                        return Object.entries({ ok: true, data: raw.data === true });
                    }
                    return Object.entries({ ok: false, error: (raw && raw.error) || 'secure_compare_failed' });
                }
                if (a.length !== b.length) {
                    return Object.entries({ ok: true, data: false });
                }
                let diff = 0;
                for (let i = 0; i < a.length; i++) diff |= a[i] ^ b[i];
                return Object.entries({ ok: true, data: diff === 0 });
            }
            return { ok: false, error: `unknown crypto action '${act}'` };
        }
        if (kind === 'http') {
            // @deka/http — outbound HTTP/1.1, HTTP/2
            // (ALPN h2), streaming req/resp bodies,
            // opt-in cookie jars, WebSocket client.
            // All dispatched through a single Rust op;
            // see crates/deka_host/src/modules/http.rs
            // for the action list and issue #128 for
            // the DoD.
            if (typeof ops.op_deka_http_call !== 'function') {
                return { ok: false, error: 'op_deka_http_call unavailable' };
            }
            const act = String(action || '');
            const req = payload || {};
            const raw = ops.op_deka_http_call(act, req);
            return Object.entries(raw || {});
        }
        if (kind === 'concurrency') {
            const act = String(action || '');
            const req = payload || {};
            if (act === 'lock_acquire') {
                if (typeof ops.op_php_concurrency_lock_acquire !== 'function') {
                    return { ok: false, error: 'op_php_concurrency_lock_acquire unavailable' };
                }
                const name = String(req.name || '');
                const timeout_ms = Number(req.timeout_ms ?? 30000);
                return ops.op_php_concurrency_lock_acquire(name, timeout_ms);
            }
            if (act === 'lock_release') {
                if (typeof ops.op_php_concurrency_lock_release !== 'function') {
                    return { ok: false, error: 'op_php_concurrency_lock_release unavailable' };
                }
                const token = Number(req.token ?? 0);
                return ops.op_php_concurrency_lock_release(token);
            }
            return { ok: false, error: `unknown concurrency action '${act}'` };
        }
        if (kind === 'json') {
            const act = String(action || '');
            const req = payload || {};
            if (act === 'encode') {
                try {
                    return Object.entries({ ok: true, json: JSON.stringify(req.value ?? null) });
                } catch (err) {
                    return Object.entries({ ok: false, error: err && err.message ? err.message : String(err) });
                }
            }
            if (act === 'decode') {
                try {
                    const src = String(req.json ?? '');
                    return Object.entries({ ok: true, value: JSON.parse(src) });
                } catch (err) {
                    return Object.entries({ ok: false, error: err && err.message ? err.message : String(err) });
                }
            }
            if (act === 'validate') {
                try {
                    const src = String(req.json ?? '');
                    JSON.parse(src);
                    return Object.entries({ ok: true, valid: true });
                } catch (_err) {
                    return Object.entries({ ok: true, valid: false });
                }
            }
            return Object.entries({ ok: false, error: `unknown json action '${act}'` });
        }
        return { ok: false, error: `unknown bridge kind '${kind}'` };
    };

    // Normalize serde_v8 results: Rust ops may return objects
    // created via Object.create(null) which lack toString/valueOf.
    // Deep-copy into regular JS objects so PHPX string coercion
    // (e.g. '' . $value) works as expected.
    const __dekaFixProto = (val) => {
        if (val === null || val === undefined || typeof val !== 'object') return val;
        if (Array.isArray(val)) return val.map(__dekaFixProto);
        if (Object.getPrototypeOf(val) === null) {
            const fixed = {};
            for (const k of Object.keys(val)) fixed[k] = __dekaFixProto(val[k]);
            return fixed;
        }
        return val;
    };

    // DS bridge Result tagging (deka#578): one shared helper
    // instead of a per-call IIFE. The expression is injected
    // from crate::prelude (deka#582) so the envelope carries
    // __enum/name exactly like the prelude's Result constructors.
    const __deka_to_result = __DEKA_TO_RESULT__;

    const __bridge = (kind, action, payload) => {
        try {
            return __dekaFixProto(routeHostCall(String(kind || ''), String(action || ''), payload || {}));
        } catch (err) {
            return { ok: false, error: err && err.message ? String(err.message) : String(err) };
        }
    };
    // DS `bridge kind.action(args)` emit (RFD 27). Positional args;
    // PHPX __bridge still takes a payload object.
    // The catalog allowlist is injected at bootstrap from
    // permissions::host_bridge::js_catalog_json() — the
    // authoritative Rust catalog, never a hand-maintained JS
    // list (deka#620 drift). The `async` flags in it MUST match
    // the pinned dsc emit (dsc 0.8.1): catalog-async actions
    // are awaited by DS code with `.then(__deka_to_result)` and
    // every other action is called synchronously, so
    // `__deka_host` returns the {ok, value|error} envelope
    // synchronously for those and the async op's Promise for
    // the fs four (which never rejects).
    const DS_HOST_CATALOG = /*__DEKA_HOST_CATALOG__*/;
    // Single source of truth for the structured-denial wire
    // marker: injected from the Rust PERMISSION_DENIED_MARKER
    // constant so the two sides can never drift.
    const DEKA_PERMISSION_DENIED_MARKER = "__DEKA_PERMISSION_DENIED_MARKER__";
    const __dekaDenyFromMessage = (message) => {
        const text = String(message);
        if (text.indexOf(DEKA_PERMISSION_DENIED_MARKER) !== 0) return null;
        try {
            const parsed = JSON.parse(text.slice(DEKA_PERMISSION_DENIED_MARKER.length));
            if (parsed && typeof parsed === 'object') {
                return { ok: false, error: { name: 'PermissionDenied', capability: parsed.capability, target: parsed.target } };
            }
        } catch (_err) {}
        return null;
    };
    // The @deka/fs public error is an enum. Keep the conversion
    // at the bridge boundary: package wrappers merely return the
    // granted action's Result and must never parse error text or
    // make authorization decisions (deka#758).
    const __dekaFsError = (error) => {
        if (error && typeof error === 'object' && error.__enum === 'FsError') return error;
        const name = error && typeof error === 'object' ? String(error.name || '') : '';
        if (name === 'PermissionDenied') {
            return {
                __enum: 'FsError', __case: 'PermissionDenied', name: 'PermissionDenied',
                value: {
                    __deka_struct: 'FsPermission',
                    capability: String(error.capability || 'read'),
                    target: String(error.target || '*'),
                },
            };
        }
        if (name === 'UnsupportedHost') {
            return { __enum: 'FsError', __case: 'UnsupportedHost', name: 'UnsupportedHost' };
        }
        if (name === 'InvalidPayload') {
            return { __enum: 'FsError', __case: 'InvalidPayload', name: 'InvalidPayload' };
        }
        const message = typeof error === 'string'
            ? error
            : (error && error.message ? String(error.message) : 'filesystem operation failed');
        return { __enum: 'FsError', __case: 'Failed', name: 'Failed', value: message };
    };
    const __dekaErrorEnvelope = (kind, err) => {
        const message = err && err.message ? String(err.message) : String(err);
        const denial = __dekaDenyFromMessage(message);
        if (denial) {
            if (kind === 'fs') denial.error = __dekaFsError(denial.error);
            return denial;
        }
        if (kind === 'fs') return { ok: false, error: __dekaFsError(message) };
        return { ok: false, error: message };
    };
    const __deka_host = (kind, action, args, grants) => {
        // Keep the kind visible to catch for typed fs errors.
        let k = '';
        try {
            k = String(kind || '');
            const a = String(action || '');
            const cat = DS_HOST_CATALOG[k];
            if (!cat || typeof cat[a] === 'undefined') {
                return { ok: false, error: `unknown bridge action '${k}.${a}'` };
            }
            // RFD 27 grant gate. An Array `grants` is the frozen
            // kind list the ESM loader preamble injects per
            // DekaScript module: the module may only touch
            // kinds its package was granted. undefined/null is
            // the platform inline-handler path
            // (wrap_with_host_bindings), which is not
            // DekaScript-from-disk and carries no per-module
            // grants yet — the catalog gate above is the only
            // check there.
            if (Array.isArray(grants) && grants.indexOf(k) < 0) {
                const denial = { ok: false, error: { name: 'HostGrantDenied', kind: k, action: a } };
                // Catalog-async emit chains `.then(...)` on the
                // result and never catches; hand back a
                // resolved Promise so a grant denial of an
                // async action is still a Result.Err, never a
                // throw (same contract as the permission path
                // below).
                if (cat[a].async === true) return Promise.resolve(denial);
                return denial;
            }
            const list = Array.isArray(args) ? args : [];
            const toByteArray = (v) => {
                if (v instanceof Uint8Array) return Array.from(v);
                if (Array.isArray(v)) return v;
                if (typeof v === 'string') {
                    return Array.from(new TextEncoder().encode(v));
                }
                return [];
            };
            const fsBytePayload = (v) => v instanceof Uint8Array ? Array.from(v) : null;
            const fsWritesBytes = k === 'fs' && (a === 'write_file' || a === 'write_file_sync' || a === 'write');
            if (fsWritesBytes && fsBytePayload(list[1]) === null) {
                const invalid = { ok: false, error: __dekaFsError({ name: 'InvalidPayload' }) };
                return cat[a].async === true ? Promise.resolve(invalid) : invalid;
            }
            // db_call_impl (crates/deka_host/src/modules/db.rs)
            // opens with {driver, config}, not a URL string; the
            // catalog's db.open arg is a connection URL, parsed
            // here into the driver/config shape the host expects.
            const dbOpenPayload = (url) => {
                const text = String(url || '');
                const schemeEnd = text.indexOf('://');
                const driver = schemeEnd > 0 ? text.slice(0, schemeEnd).toLowerCase() : '';
                if (driver.startsWith('sqlite')) {
                    return { driver, config: { path: text.slice(schemeEnd + 3) } };
                }
                const match = text.match(/^[a-z][a-z0-9+.-]*:\/\/(?:([^:@/?#]*)(?::([^@/?#]*))?@)?([^/:?#]+)(?::(\d+))?(?:\/([^?#]*))?/i);
                if (!match) return { driver, config: {} };
                const config = { host: match[4] };
                if (typeof match[5] !== 'undefined') config.port = Number(match[5]);
                if (typeof match[2] !== 'undefined') config.user = decodeURIComponent(match[2]);
                if (typeof match[3] !== 'undefined') config.password = decodeURIComponent(match[3]);
                if (match[6]) config.database = decodeURIComponent(match[6]);
                return { driver, config };
            };
            const payload = (() => {
                if (k === 'crypto' && a === 'random_bytes') return { length: list[0] };
                if (k === 'crypto' && a === 'digest') return { algorithm: list[0], data: list[1] };
                if (k === 'crypto' && a === 'hmac') return { algorithm: list[0], key: list[1], data: list[2] };
                if (k === 'crypto' && a === 'secure_compare') return { a: list[0], b: list[1] };
                if (k === 'crypto' && a === 'aes_256_gcm_encrypt') return { key: list[0], nonce: list[1], plaintext: list[2], aad: list[3] };
                if (k === 'crypto' && a === 'aes_256_gcm_decrypt') return { key: list[0], nonce: list[1], ciphertext: list[2], aad: list[3] };
                if (k === 'crypto' && a === 'bcrypt_verify') return { password: list[0], hash: list[1] };
                if (k === 'fs' && (a === 'read_file' || a === 'read_file_sync')) return { path: list[0] };
                if (k === 'fs' && (a === 'write_file' || a === 'write_file_sync')) return { path: list[0], data: fsBytePayload(list[1]) };
                if (k === 'fs' && (a === 'read_dir' || a === 'read_dir_sync')) return { path: list[0] };
                if (k === 'fs' && (a === 'mkdirs' || a === 'mkdirs_sync')) return { path: list[0] };
                // fs.open's second catalog arg is a write bool;
                // both the JSON impl and the proto encoder key
                // the mode string ("r"/"w" style) off `mode`.
                if (k === 'fs' && a === 'open') return { path: list[0], mode: list[1] ? 'w' : 'r' };
                if (k === 'fs' && a === 'read') return { handle: list[0], max_bytes: list[1] };
                if (k === 'fs' && a === 'write') return { handle: list[0], data: fsBytePayload(list[1]) };
                if (k === 'fs' && a === 'close') return { handle: list[0] };
                if (k === 'db' && a === 'open') return dbOpenPayload(list[0]);
                if (k === 'db' && a === 'query') return { handle: list[0], sql: list[1], params: Array.isArray(list[2]) ? list[2] : [] };
                if (k === 'db' && a === 'exec') return { handle: list[0], sql: list[1], params: Array.isArray(list[2]) ? list[2] : [] };
                if (k === 'db' && a === 'close') return { handle: list[0] };
                if (k === 'db' && a === 'stats') return { handle: list[0] };
                // concurrency is PHPX-only (see permissions::host_bridge::PHPX_ONLY_ACTIONS).
                if (k === 'time' && a === 'sleep_ms') return { milliseconds: list[0] };
                if (k === 'net' && a === 'connect') return { host: list[0], port: list[1] };
                if (k === 'net' && a === 'listen') return { host: list[0], port: list[1] };
                if (k === 'net' && a === 'accept') return { handle: list[0] };
                if (k === 'net' && a === 'read') return { handle: list[0], max_bytes: list[1] };
                if (k === 'net' && a === 'write') return { handle: list[0], data: toByteArray(list[1]) };
                if (k === 'net' && a === 'close') return { handle: list[0] };
                if (k === 'net' && a === 'set_deadline') return { handle: list[0], millis: list[1] };
                if (k === 'tls' && a === 'upgrade') return { handle: list[0], server_name: list[1] };
                if (list.length === 1 && list[0] && typeof list[0] === 'object' && !Array.isArray(list[0])) {
                    return list[0];
                }
                return { args: list };
            })();
            const routeKind = (k === 'tls' && a === 'upgrade') ? 'net' : k;
            const routeAction = (k === 'tls' && a === 'upgrade') ? 'tls_upgrade'
                : (k === 'fs' && a.endsWith('_sync') ? a.slice(0, -5) : a);
            const finish = (raw) => {
                const assoc = (Array.isArray(raw) && raw.length && Array.isArray(raw[0]))
                    ? Object.fromEntries(raw)
                    : (raw || {});
                if (assoc && assoc.ok === true && typeof assoc.data === 'undefined') {
                    if (typeof assoc.handle !== 'undefined') assoc.data = assoc.handle;
                    else if (typeof assoc.written === 'number') assoc.data = assoc.written;
                    else if (typeof assoc.valid === 'boolean') assoc.data = assoc.valid;
                    else if (a === 'read_dir' && Array.isArray(assoc.entries)) assoc.data = assoc.entries;
                    else if (typeof assoc.slept_ms === 'number') assoc.data = assoc.slept_ms;
                    // db query/exec decode shapes (op_php_db_proto_decode →
                    // db_proto_response_to_json): rows / affected_rows.
                    else if (a === 'query' && Array.isArray(assoc.rows)) assoc.data = assoc.rows;
                    else if (a === 'exec' && typeof assoc.affected_rows === 'number') assoc.data = assoc.affected_rows;
                    // db stats decodes to a flat stats object; surface it as one value.
                    else if (k === 'db' && a === 'stats' && typeof assoc.active_handles === 'number') assoc.data = {
                        active_handles: assoc.active_handles,
                        handles_by_driver: assoc.handles_by_driver,
                        statement_cache_entries: assoc.statement_cache_entries,
                        statement_cache_hits: assoc.statement_cache_hits,
                        statement_cache_misses: assoc.statement_cache_misses,
                        metrics: assoc.metrics,
                    };
                    // concurrency lock ops return {ok, token} (the
                    // lock_release unit path falls to data=true).
                    else if (a === 'lock_acquire' && typeof assoc.token === 'number') assoc.data = assoc.token;
                    else assoc.data = true;
                }
                if (assoc && assoc.ok === true && a !== 'read_dir' && Array.isArray(assoc.data)
                    && assoc.data.every((v) => typeof v === 'number')
                    && typeof Uint8Array !== 'undefined') {
                    assoc.data = new Uint8Array(assoc.data);
                }
                // v2 bridge emit expects { ok, value }; legacy PHPX bridge uses { ok, data }.
                if (assoc && assoc.ok === true && typeof assoc.value === 'undefined' && typeof assoc.data !== 'undefined') {
                    assoc.value = assoc.data;
                }
                if (assoc && assoc.ok !== true && k === 'fs') {
                    assoc.error = __dekaFsError(assoc.error);
                }
                return assoc;
            };
            // Catalog-async entries (exactly fs.{read_file,
            // write_file, read_dir, mkdirs} in dsc 0.8.1): the fs
            // bridge runs std::fs IO on the tokio blocking pool
            // via op_php_fs_call_proto_async, so a read or write
            // no longer stalls the isolate. The Promise is handed
            // back to the caller; DS emit chains
            // `.then(__deka_to_result)` and the source-level
            // `await` resolves it. The Promise must NEVER reject
            // (dsc does not catch): the .catch below folds every
            // failure — including structured permission denials —
            // into the envelope. When the async op is
            // unavailable we fall back to the sync
            // routeHostCall path below: correctness over speed.
            if (cat[a].async === true && typeof ops.op_php_fs_call_proto_async === 'function') {
                const request = ops.op_php_fs_proto_encode(routeAction, payload);
                return Promise.resolve(ops.op_php_fs_call_proto_async(request))
                    .then((response) => finish(Object.entries(ops.op_php_fs_proto_decode(response) || {})))
                    .catch((err) => __dekaErrorEnvelope(k, err));
            }
            const raw = __dekaFixProto(routeHostCall(routeKind, routeAction, payload));
            if (raw && typeof raw.then === 'function') {
                // Catalog-sync actions must return the envelope
                // synchronously — dsc 0.8.1 sync emit has no
                // await. The concurrency host ops are async-only
                // Rust ops, so they cannot be served on the sync
                // bridge contract; report honestly instead of
                // leaking a Promise into DekaScript.
                return { ok: false, error: `bridge action '${k}.${a}' resolved asynchronously but is catalogued as sync` };
            }
            return finish(raw);
        } catch (err) {
            return __dekaErrorEnvelope(k, err);
        }
    };
    // RFD 27: these names are not user globals. Handlers get them
    // as IIFE parameters. unsafe hides the symbol key. The raw
    // Deno.core.ops table is deliberately NOT exposed here — user
    // code must go through `host` (catalog + grant gated) or
    // `bridge` (PHPX compatibility).
    // `import.meta.resolve()` (rfd#12 amendment, deka#1139)
    // needs the same lockfile-first resolver `import`
    // statements use, which lives in Rust
    // (`PhpxEsmLoader::resolve_path`). Captured here, on the
    // same frozen gateway object as the RFD 27 bridge, for
    // the same reason: the raw `Deno.core.ops` table is
    // never exposed to module code directly.
    const __opsForImportMeta = (Deno && Deno.core && Deno.core.ops) ? Deno.core.ops : {};
    globalThis[Symbol.for('deka.host.internal')] = Object.freeze({
        host: __deka_host,
        bridge: __bridge,
        toResult: __deka_to_result,
        resolveImportMeta: typeof __opsForImportMeta.op_deka_import_meta_resolve === 'function'
            ? __opsForImportMeta.op_deka_import_meta_resolve
            : undefined,
    });
    /*__DEKA_WINTERTC__*/
    try {
        Object.defineProperty(globalThis, 'Deno', {
            value: undefined,
            configurable: true,
            writable: true,
        });
    } catch (_err) {
        try { globalThis.Deno = undefined; } catch (_err2) {}
    }
    globalThis.__dekaPrint = (value, isErr = false) => {
        const text = value == null ? '' : String(value);
        __print(text, !!isErr);
    };
}

if (typeof globalThis.__dekaRuntime !== 'object') {
    globalThis.__dekaRuntime = {
        executePhpx: async function(_source, file, _props) {
            if (!(globalThis.__dekaPhp && typeof globalThis.__dekaPhp.runFile === 'function')) {
                throw new Error('runtime.executePhpx requires __dekaPhp.runFile');
            }
            const result = await globalThis.__dekaPhp.runFile(String(file || ''));
            const stdout = result && result.stdout ? String(result.stdout) : "";
            let stderr = result && result.stderr ? String(result.stderr) : "";
            if (!stderr && result && result.error) {
                stderr = String(result.error);
            }
            if (stdout) __print(stdout, false);
            if (stderr) __print(stderr, true);
            const ok = result && result.ok !== false;
            let exitCode = result && typeof result.exit_code === 'number' ? result.exit_code : 0;
            if (!ok && exitCode === 0) exitCode = 1;
            if (exitCode) globalThis.__dekaExitCode = exitCode;
            return result;
        }
    };
}

// The deka/router module is already loaded as an extension
// and exposes itself as globalThis.__dekaRouter automatically
