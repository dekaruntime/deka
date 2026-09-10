use super::*;

/// Bind host dispatchers as locals so PHPX/DS emit can name `__bridge` /
/// `__deka_host` without those identifiers living on user `globalThis`.
fn wrap_with_host_bindings(body: &str) -> String {
    format!(
        "(function() {{\nconst __h = globalThis[Symbol.for('deka.host.internal')];\n(function(__deka_host, __bridge, __bridge_async, __deka_wasm_call, __deka_wasm_call_async, __deka_to_result) {{\n{body}\n}})(__h && __h.host, __h && __h.bridge, __h && __h.bridgeAsync, __h && __h.wasmCall, __h && __h.wasmCallAsync, __h && __h.toResult);\n}})();"
    )
}

/// Assemble the bootstrap script, injecting the shared enum prelude and the
/// `__deka_to_result` helper (deka#582). Shapes must match dsc's emitted
/// `Result`/`Option` constructors (see `crate::prelude`).
fn bootstrap_source(template: &str) -> String {
    let source = template
        .replace(
            "/*__DEKA_POOL_ENUM_PRELUDE__*/",
            &crate::prelude::pool_prelude(),
        )
        .replace("__DEKA_TO_RESULT__", &crate::prelude::to_result_helper())
        .replace("/*__DEKA_WINTERTC__*/", include_str!("../wintertc.js"));
    // assert!, not debug_assert!: release is what ships, and a marker that
    // fails to substitute there fails silently. The prelude marker sits inside
    // a /* */ comment, so an un-replaced one simply vanishes and the isolate
    // boots with no Result/Option at all — surfacing much later as
    // `Ok is not defined` on a request. That is precisely the silent
    // divergence this change exists to remove, so the check has to run in the
    // build that matters. Two substring scans once per worker bootstrap is
    // nothing next to creating the isolate.
    assert!(
        !source.contains("__DEKA_POOL_ENUM_PRELUDE__")
            && !source.contains("__DEKA_TO_RESULT__")
            && !source.contains("__DEKA_WINTERTC__"),
        "bootstrap prelude markers must all be injected"
    );
    source
}

impl WorkerThread {
    /// Execute a request in the warm isolate
    pub(super) async fn execute_in_isolate(
        &mut self,
        key: &HandlerKey,
        request: &WorkerRequest,
    ) -> (ExecutionOutcome, ExecutionProfile) {
        // Get mutable reference to isolate
        let use_code_cache = self.config.enable_code_cache;
        let secrets_cache = Arc::clone(&self.secrets_cache);
        let (isolates, code_cache) = (&mut self.isolates, &mut self.code_cache);
        let isolate = isolates
            .get_mut(key)
            .ok_or_else(|| "Isolate not found".to_string());

        let isolate = match isolate {
            Ok(isolate) => isolate,
            Err(err) => return (ExecutionOutcome::Err(err), ExecutionProfile::empty()),
        };

        // Ensure THIS isolate is the currently-entered one on this thread.
        //
        // Each `OwnedIsolate` in deno_core is `Enter()`ed at construction and
        // `Exit()`ed on drop, so with N isolates on one worker only the most
        // recently constructed one is `v8__Isolate__GetCurrent()`. When we
        // dispatch a request to an older isolate the current-isolate mismatch
        // makes `ContextScope::new` panic with
        //   "PinnedRef<HandleScope<()>> and Context do not belong to the same Isolate".
        //
        // V8 allows re-entering an already-entered isolate; the matching
        // `Exit()` simply restores the previous top-of-stack. `_enter_guard`
        // does the exit on drop (covering every early return in this method
        // without having to thread cleanup through each match arm).
        let _enter_guard = IsolateEntryGuard::enter(isolate.runtime.v8_isolate());

        isolate.active_requests = 1;
        isolate.state = IsolateState::Executing {
            request_id: request.request_id.clone(),
            started_at: Instant::now(),
        };

        // Bootstrap on first use (load Web APIs polyfills if needed)
        if !isolate.bootstrapped {
            let bootstrap_start = Instant::now();

            // Basic Web API polyfills
            const BOOTSTRAP: &str = r#"
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
                    return [read, write];
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
                globalThis.deka.ui = Object.freeze({
                    signal: __dekaCreateSignal,
                    effect: __dekaCreateEffect,
                    memo: __dekaCreateMemo,
                    createSignal: __dekaCreateSignal,
                    createEffect: __dekaCreateEffect,
                    createMemo: __dekaCreateMemo,
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

                // Minimal URL polyfill for parsing URLs
                if (typeof globalThis.URL === 'undefined') {
                    globalThis.URL = class URL {
                        constructor(url) {
                            this.href = url;

                            // Parse protocol
                            const protocolMatch = url.match(/^([a-z][a-z0-9+.-]*):\/\//i);
                            this.protocol = protocolMatch ? protocolMatch[1] + ':' : '';

                            // Remove protocol
                            let remaining = protocolMatch ? url.slice(protocolMatch[0].length) : url;

                            // Remove hostname/port (everything before first / or ?, or end of string)
                            const hostMatch = remaining.match(/^([^\/\\?#]*)/);
                            this.host = hostMatch ? hostMatch[1] : '';
                            remaining = remaining.slice(this.host.length);

                            // If nothing left after host, pathname is '/'
                            if (!remaining) {
                                this.pathname = '/';
                                this.search = '';
                                this.hash = '';
                                return;
                            }

                            // Extract pathname, search, and hash
                            const pathMatch = remaining.match(/^([^?#]*)(\\?[^#]*)?(#.*)?$/);
                            if (pathMatch) {
                                this.pathname = pathMatch[1] || '/';
                                this.search = pathMatch[2] || '';
                                this.hash = pathMatch[3] || '';
                            } else {
                                this.pathname = '/';
                                this.search = '';
                                this.hash = '';
                            }
                        }
                    };
                }

                // Runtime bridge helpers for PHPX stdlib (JS runtime path)
                if (typeof globalThis.function_exists !== 'function') {
                    globalThis.function_exists = function(name) {
                        const n = String(name || '');
                        if (n === '__bridge' || n === '__bridge_async' || n === '__deka_wasm_call' || n === '__deka_wasm_call_async') {
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
                        if (kind === 'neo4j') {
                            const shopId = globalThis.__shopId;
                            if (shopId && typeof ops.op_zega_backend === 'function' && ops.op_zega_backend(shopId) === 'zega') {
                                if (typeof ops.op_zega_cql_call === 'function') {
                                    return ops.op_zega_cql_call(shopId, String(action || ''), payload || {});
                                }
                                return { ok: false, error: 'zega CQL bridge op unavailable' };
                            }
                            if (typeof ops.op_neo4j_call === 'function') {
                                const p = payload || {};
                                // Shard routing: always stamp the Host-derived
                                // shop slug on connect(). The Rust op uses the
                                // legacy __account_id payload key as its shard
                                // key for two things:
                                //  (1) when no explicit URL was passed, pick
                                //      the owning shard's Neo4j URL;
                                //  (2) when an explicit URL looks like a
                                //      single-machine dev default
                                //      (`localhost`/`127.0.0.1`), override it
                                //      with the shop's shard URL so
                                //      migrated tenants on non-router shards
                                //      don't try to hit a port their stack
                                //      doesn't expose.
                                if (action === 'connect') {
                                    const shardKey = globalThis.__shardKey || globalThis.__shopId;
                                    if (shardKey) {
                                        p.__account_id = shardKey;
                                    }
                                }
                                return ops.op_neo4j_call(String(action || ''), p);
                            }
                            return { ok: false, error: 'neo4j bridge op unavailable' };
                        }
                        if (kind === 'redis') {
                            const redisAction = String(action || '').toLowerCase();
                            if (redisAction === 'flush' || redisAction === 'flushdb' || redisAction === 'flushall') {
                                return { ok: false, error: 'redis admin action blocked for tenant code' };
                            }
                            if (redisAction === 'scan' || redisAction === 'config' || redisAction === 'randomkey') {
                                return { ok: false, error: 'redis unscoped action blocked for tenant code' };
                            }
                            const shopId = globalThis.__shopId;
                            if (shopId && typeof ops.op_zega_backend === 'function' && ops.op_zega_backend(shopId) === 'zega') {
                                if (typeof ops.op_zega_kv_call === 'function') {
                                    return ops.op_zega_kv_call(shopId, redisAction, payload || {});
                                }
                                return { ok: false, error: 'zega KV bridge op unavailable' };
                            }
                            if (typeof ops.op_redis_call === 'function') {
                                const p = payload || {};
                                // Auto-prefix Redis keys with tenant ID (transparent to PHPX code)
                                if (shopId && p.key && redisAction !== 'connect' && redisAction !== 'close' && redisAction !== 'keys') {
                                    p.key = shopId + ':' + p.key;
                                }
                                // KEYS is an enumeration primitive; tenant calls must never fall back
                                // to native Redis's implicit `*` pattern against the shared DB.
                                if (shopId && redisAction === 'keys') {
                                    p.pattern = shopId + ':' + (p.pattern || '*');
                                }
                                // Shard routing: always stamp the Host-derived
                                // shop slug on connect() so the Rust op can (a) pick the
                                // owning shard when no URL was passed, or (b)
                                // override a dev-default localhost URL with
                                // the shop's shard URL. See the neo4j
                                // branch above for the full reasoning.
                                if (redisAction === 'connect') {
                                    const shardKey = globalThis.__shardKey || globalThis.__shopId;
                                    if (shardKey) {
                                        p.__account_id = shardKey;
                                    }
                                }
                                return ops.op_redis_call(redisAction, p);
                            }
                            return { ok: false, error: 'redis bridge op unavailable' };
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
                    const __bridge_async = async (kind, action, payload) => {
                        try {
                            return __dekaFixProto(await routeHostCall(String(kind || ''), String(action || ''), payload || {}));
                        } catch (err) {
                            return { ok: false, error: err && err.message ? String(err.message) : String(err) };
                        }
                    };
                    const __deka_wasm_call = (moduleId, exportName, payload) => {
                        const name = String(moduleId || '');
                        if (name.startsWith('__deka_')) {
                            const kind = name.replace(/^__deka_/, '');
                            return __dekaFixProto(routeHostCall(kind, exportName, payload || {}));
                        }
                        return { ok: false, error: `unknown host bridge module '${name}'` };
                    };
                    const __deka_wasm_call_async = async (moduleId, exportName, payload) => {
                        const name = String(moduleId || '');
                        if (name.startsWith('__deka_')) {
                            const kind = name.replace(/^__deka_/, '');
                            return __dekaFixProto(routeHostCall(kind, exportName, payload || {}));
                        }
                        return { ok: false, error: `unknown host bridge module '${name}'` };
                    };
                    // DS `bridge kind.action(args)` emit (RFD 27). Positional args;
                    // PHPX __bridge still takes a payload object.
                    // Catalog allowlist is the runtime gate: a leaked global cannot
                    // reach PHPX-only kinds (db/redis/vault/json/...). Keep in sync
                    // with `host_bridge.rs` CATALOG.
                    const DS_HOST_CATALOG = {
                        crypto: ['random_bytes', 'digest', 'hmac', 'secure_compare', 'aes_256_gcm_encrypt', 'aes_256_gcm_decrypt', 'bcrypt_verify'],
                        fs: ['read_file', 'write_file', 'read_dir', 'mkdirs'],
                        net: ['connect', 'listen', 'accept', 'read', 'write', 'close', 'set_deadline'],
                        tls: ['upgrade'],
                        time: ['sleep_ms'],
                    };
                    const __deka_host = (kind, action, args) => {
                        try {
                            const k = String(kind || '');
                            const a = String(action || '');
                            if (!DS_HOST_CATALOG[k] || DS_HOST_CATALOG[k].indexOf(a) < 0) {
                                return { ok: false, error: `unknown bridge action '${k}.${a}'` };
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
                            const payload = (() => {
                                if (k === 'crypto' && a === 'random_bytes') return { length: list[0] };
                                if (k === 'crypto' && a === 'digest') return { algorithm: list[0], data: list[1] };
                                if (k === 'crypto' && a === 'hmac') return { algorithm: list[0], key: list[1], data: list[2] };
                                if (k === 'crypto' && a === 'secure_compare') return { a: list[0], b: list[1] };
                                if (k === 'crypto' && a === 'aes_256_gcm_encrypt') return { key: list[0], nonce: list[1], plaintext: list[2], aad: list[3] };
                                if (k === 'crypto' && a === 'aes_256_gcm_decrypt') return { key: list[0], nonce: list[1], ciphertext: list[2], aad: list[3] };
                                if (k === 'crypto' && a === 'bcrypt_verify') return { password: list[0], hash: list[1] };
                                if (k === 'fs' && a === 'read_file') return { path: list[0] };
                                if (k === 'fs' && a === 'write_file') return { path: list[0], data: toByteArray(list[1]) };
                                if (k === 'fs' && a === 'read_dir') return { path: list[0] };
                                if (k === 'fs' && a === 'mkdirs') return { path: list[0] };
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
                            const routeAction = (k === 'tls' && a === 'upgrade') ? 'tls_upgrade' : a;
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
                                    else assoc.data = true;
                                }
                                if (assoc && assoc.ok === true && a !== 'read_dir' && Array.isArray(assoc.data)
                                    && typeof Uint8Array !== 'undefined') {
                                    assoc.data = new Uint8Array(assoc.data);
                                }
                                // v2 bridge emit expects { ok, value }; legacy PHPX bridge uses { ok, data }.
                                if (assoc && assoc.ok === true && typeof assoc.value === 'undefined' && typeof assoc.data !== 'undefined') {
                                    assoc.value = assoc.data;
                                }
                                return assoc;
                            };
                            // Async catalog entries (deka#578): fs ops run
                            // std::fs IO on the tokio blocking pool via
                            // op_php_fs_call_proto_async, so a read or write
                            // no longer stalls the isolate. The Promise is
                            // handed back to the caller; DS emit chains
                            // `.then(__deka_to_result)` and the source-level
                            // `await` resolves it. Keep the flag in sync with
                            // the compiler-side catalog in dsc.
                            if (k === 'fs' && typeof ops.op_php_fs_call_proto_async === 'function') {
                                const request = ops.op_php_fs_proto_encode(routeAction, payload);
                                return Promise.resolve(ops.op_php_fs_call_proto_async(request))
                                    .then((response) => finish(Object.entries(ops.op_php_fs_proto_decode(response) || {})))
                                    .catch((err) => ({ ok: false, error: err && err.message ? String(err.message) : String(err) }));
                            }
                            return finish(__dekaFixProto(routeHostCall(routeKind, routeAction, payload)));
                        } catch (err) {
                            return { ok: false, error: err && err.message ? String(err.message) : String(err) };
                        }
                    };
                    // RFD 27: these names are not user globals. Handlers get them
                    // as IIFE parameters. unsafe hides the symbol key.
                    globalThis[Symbol.for('deka.host.internal')] = Object.freeze({
                        host: __deka_host,
                        bridge: __bridge,
                        bridgeAsync: __bridge_async,
                        wasmCall: __deka_wasm_call,
                        wasmCallAsync: __deka_wasm_call_async,
                        toResult: __deka_to_result,
                        ops: __ops,
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
            "#;

            if let Err(err) = isolate.runtime.execute_script(
                "bootstrap.js",
                ModuleCodeString::from(bootstrap_source(BOOTSTRAP)),
            ) {
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                return (
                    ExecutionOutcome::Err(format!("Bootstrap failed: {}", err)),
                    ExecutionProfile::empty(),
                );
            }

            isolate.bootstrapped = true;
            tracing::debug!(
                "Worker {} bootstrapped {} in {:?}",
                self.worker_id,
                key.name,
                bootstrap_start.elapsed()
            );
        }

        if handler_is_unsupported_script(&key.name) {
            isolate.active_requests = 0;
            isolate.state = IsolateState::Idle;
            return (
                ExecutionOutcome::Err(
                    "TypeScript handlers are not supported; emit JavaScript (export default { fetch }) or serve .ds via dsc.".to_string(),
                ),
                ExecutionProfile::empty(),
            );
        }

        // Enforce the resolved dynamic-code policy before any inline user
        // handler source reaches V8. Validate once per warm isolate rather
        // than on every request; a source hash change creates a new isolate.
        //
        // Scope: this covers the *platform* path, whose tenant bundles arrive
        // as inline source from `PlatformState::resolve_handler`. `deka run`
        // and `deka serve` are ESM and set `handler_code` to the empty string
        // (runtime/src/run.rs, runtime/src/serve.rs), so the guard below is
        // false for them; those are gated by the module-graph validator in
        // #425. The multi-tenant storefronts are the case this protects.
        if !request.request_data.handler_code.trim().is_empty() && !isolate.dynamic_code_validated {
            if let Err(err) = validation::validate_dynamic_code_from_process_env(
                &request.request_data.handler_code,
                &key.name,
            ) {
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                return (ExecutionOutcome::Err(err), ExecutionProfile::empty());
            }
            isolate.dynamic_code_validated = true;
        }

        let use_esm = request.request_data.handler_entry.is_some()
            && std::env::var("DEKA_RUNTIME_ESM")
                .map(|value| value != "0" && value != "false")
                .unwrap_or(true);

        // Check if the handler code is already a self-contained async IIFE
        // produced by the bundler (e.g. `(async function() { ... })()`).
        // These bundles set globalThis.app internally — double-wrapping them
        // in another sync IIFE breaks top-level await and prevents the async
        // code from resolving before __dekaExecuteRequest checks globalThis.app.
        let is_pre_bundled_iife = {
            let trimmed = request.request_data.handler_code.trim_start();
            trimmed.starts_with("(async function()") || trimmed.starts_with("(function()")
        };

        let wrapped_handler_code = if !use_esm {
            if is_pre_bundled_iife {
                // Pre-bundled IIFE: run directly without re-wrapping.
                // The bundler already stripped exports and wrapped in an
                // async IIFE that sets globalThis.app.
                let setup_code =
                    "globalThis.app = undefined; globalThis.Deka = globalThis.Deka || {};";
                if let Err(err) = isolate
                    .runtime
                    .execute_script("setup.js", ModuleCodeString::from(setup_code.to_string()))
                {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                        ExecutionProfile::empty(),
                    );
                }

                // Execute the pre-bundled handler with host dispatchers closed over.
                let handler_result = isolate.runtime.execute_script(
                    "handler.js",
                    ModuleCodeString::from(wrap_with_host_bindings(
                        &request.request_data.handler_code,
                    )),
                );
                match handler_result {
                    Ok(_value) => {
                        // The async IIFE returns a Promise. Run the event
                        // loop unconditionally so globalThis.app gets set
                        // before __dekaExecuteRequest checks it.
                        if let Err(err) = isolate
                            .runtime
                            .run_event_loop(deno_core::PollEventLoopOptions::default())
                            .await
                        {
                            isolate.active_requests = 0;
                            isolate.state = IsolateState::Idle;
                            return (
                                ExecutionOutcome::Err(format!(
                                    "Handler async init failed: {}",
                                    err
                                )),
                                ExecutionProfile::empty(),
                            );
                        }
                    }
                    Err(err) => {
                        let raw = err.to_string();
                        if parse_exit_code(&raw).is_none() {
                            isolate.active_requests = 0;
                            isolate.state = IsolateState::Idle;
                            let formatted = validation::format_runtime_syntax_error(
                                &raw,
                                &request.request_data.handler_code,
                                &key.name,
                            );
                            return (
                                ExecutionOutcome::Err(formatted.unwrap_or_else(|| {
                                    format!("Handler execution failed: {}", err)
                                })),
                                ExecutionProfile::empty(),
                            );
                        }
                    }
                }

                None // Already loaded — skip the wrapped-handler path below
            } else {
                // Execute the handler - transform import/export statements
                // Replace ES6 import with global access
                let handler_code = request
                    .request_data
                    .handler_code
                    .replace(
                        "import { Router, cors, logger, prettyJSON } from 'deka/router'",
                        "const { Router, cors, logger, prettyJSON } = globalThis.__dekaRouter;",
                    )
                    .replace(
                        "import { Router, cors, logger, prettyJSON } from \"deka/router\"",
                        "const { Router, cors, logger, prettyJSON } = globalThis.__dekaRouter;",
                    )
                    .replace(
                        "import { Router } from 'deka/router'",
                        "const { Router } = globalThis.__dekaRouter;",
                    )
                    .replace(
                        "import { Router } from \"deka/router\"",
                        "const { Router } = globalThis.__dekaRouter;",
                    )
                    .replace(
                        "import { Database, Statement } from 'deka/sqlite'",
                        "const { Database, Statement } = globalThis.__dekaSqlite;",
                    )
                    .replace(
                        "import { Database, Statement } from \"deka/sqlite\"",
                        "const { Database, Statement } = globalThis.__dekaSqlite;",
                    )
                    .replace(
                        "import { Database } from 'deka/sqlite'",
                        "const { Database } = globalThis.__dekaSqlite;",
                    )
                    .replace(
                        "import { Database } from \"deka/sqlite\"",
                        "const { Database } = globalThis.__dekaSqlite;",
                    )
                    .replace(
                        "import { t4, T4Client, T4File, write } from 'deka/t4'",
                        "const { t4, T4Client, T4File, write } = globalThis.__dekaT4;",
                    )
                    .replace(
                        "import { t4, T4Client, T4File, write } from \"deka/t4\"",
                        "const { t4, T4Client, T4File, write } = globalThis.__dekaT4;",
                    )
                    .replace(
                        "import { t4 } from 'deka/t4'",
                        "const { t4 } = globalThis.__dekaT4;",
                    )
                    .replace(
                        "import { t4 } from \"deka/t4\"",
                        "const { t4 } = globalThis.__dekaT4;",
                    )
                    .replace(
                        "import { Mesh, IsolatePool, Isolate, serve } from 'deka'",
                        "const { Mesh, IsolatePool, Isolate, serve } = globalThis.__deka;",
                    )
                    .replace(
                        "import { Mesh, IsolatePool, Isolate, serve } from \"deka\"",
                        "const { Mesh, IsolatePool, Isolate, serve } = globalThis.__deka;",
                    )
                    // Remove export default statement - we'll capture 'app' variable directly
                    .replace("export default app", "// export default app")
                    .replace("export default ", "const __dekaDefault = ");

                let wrapped = wrap_with_host_bindings(&format!(
                    "{}\nif (typeof globalThis.app === 'undefined') {{ if (typeof __dekaDefault !== 'undefined') {{ if (typeof __dekaDefault === 'function' && typeof globalThis.__dekaNodeExpressAdapter === 'function' && (typeof __dekaDefault.handle === 'function' || typeof __dekaDefault.listen === 'function')) {{ globalThis.app = globalThis.__dekaNodeExpressAdapter(__dekaDefault); }} else if (__dekaDefault && typeof __dekaDefault === 'object' && typeof __dekaDefault.fetch === 'function') {{ globalThis.app = __dekaDefault; }} else if (__dekaDefault && typeof __dekaDefault === 'object' && !__dekaDefault.__dekaServer && typeof __dekaDefault.routes === 'object' && globalThis.__deka && typeof globalThis.__deka.serve === 'function') {{ globalThis.app = globalThis.__deka.serve(__dekaDefault); }} else {{ globalThis.app = __dekaDefault; }} }} else if (typeof app !== 'undefined') {{ if (typeof app === 'function' && typeof globalThis.__dekaNodeExpressAdapter === 'function' && (typeof app.handle === 'function' || typeof app.listen === 'function')) {{ globalThis.app = globalThis.__dekaNodeExpressAdapter(app); }} else {{ globalThis.app = app; }} }} }}",
                    handler_code
                ));

                let setup_code =
                    "globalThis.app = undefined; globalThis.Deka = globalThis.Deka || {};";
                if let Err(err) = isolate
                    .runtime
                    .execute_script("setup.js", ModuleCodeString::from(setup_code.to_string()))
                {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                        ExecutionProfile::empty(),
                    );
                }

                Some(wrapped)
            }
        } else {
            None
        };

        let tenant_info = request
            .request_data
            .request_parts
            .as_ref()
            .and_then(|parts| resolve_request_tenant(&parts.headers));
        let shop_secrets = if let Some(info) = tenant_info.as_ref() {
            if info.shop_id.is_empty() {
                HashMap::new()
            } else {
                match secrets_cache.get_secrets_for_shop(&info.shop_id).await {
                    Ok(secrets) => secrets,
                    Err(err) => {
                        tracing::warn!(
                            shop_id = %info.shop_id,
                            error = %err,
                            "failed to fetch shop secrets from harar"
                        );
                        HashMap::new()
                    }
                }
            }
        } else {
            HashMap::new()
        };

        if request.request_data.mode != ExecutionMode::Build {
            if let Err(err) = set_request_globals(
                &mut isolate.runtime,
                &request.request_data.request_value,
                request.request_data.request_parts.as_ref(),
                &self.deka_args,
                tenant_info.as_ref(),
                &shop_secrets,
            ) {
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                return (
                    ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                    ExecutionProfile::empty(),
                );
            }
        }

        let exec_mode = match request.request_data.mode {
            ExecutionMode::Module => "module",
            ExecutionMode::StaticRender => "static-render",
            ExecutionMode::Build => "build",
            _ => "request",
        };
        if let Err(err) = isolate.runtime.execute_script(
            "exec_mode.js",
            ModuleCodeString::from(format!(
                "globalThis.__dekaExecMode = {};",
                serde_json::to_string(exec_mode).unwrap_or_else(|_| "\"request\"".to_string())
            )),
        ) {
            isolate.active_requests = 0;
            isolate.state = IsolateState::Idle;
            return (
                ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                ExecutionProfile::empty(),
            );
        }

        if let Some(wrapped_handler_code) = wrapped_handler_code.as_ref() {
            if use_code_cache {
                let source_hash = Self::hash_source(&request.request_data.handler_code);
                if let Err(err) = Self::compile_handler(
                    &mut isolate.runtime,
                    code_cache,
                    source_hash,
                    wrapped_handler_code,
                ) {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    let formatted = validation::format_runtime_syntax_error(
                        &err,
                        &request.request_data.handler_code,
                        &key.name,
                    );
                    return (
                        ExecutionOutcome::Err(formatted.unwrap_or(err)),
                        ExecutionProfile::empty(),
                    );
                }
            } else if let Err(err) = isolate.runtime.execute_script(
                "handler.js",
                ModuleCodeString::from(wrapped_handler_code.to_string()),
            ) {
                let raw = err.to_string();
                if parse_exit_code(&raw).is_none() {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    let formatted = validation::format_runtime_syntax_error(
                        &raw,
                        &request.request_data.handler_code,
                        &key.name,
                    );
                    return (
                        ExecutionOutcome::Err(
                            formatted
                                .unwrap_or_else(|| format!("Handler execution failed: {}", err)),
                        ),
                        ExecutionProfile::empty(),
                    );
                }
            }
        } else if use_esm {
            if !isolate.handler_loaded {
                let spec = match isolate.entry_specifier.as_ref() {
                    Some(spec) => spec,
                    None => {
                        isolate.active_requests = 0;
                        isolate.state = IsolateState::Idle;
                        return (
                            ExecutionOutcome::Err("missing module entry specifier".to_string()),
                            ExecutionProfile::empty(),
                        );
                    }
                };
                let module_id = match isolate.runtime.load_main_es_module(spec).await {
                    Ok(id) => id,
                    Err(err) => {
                        isolate.active_requests = 0;
                        isolate.state = IsolateState::Idle;
                        return (
                            ExecutionOutcome::Err(format!("Failed to load module: {}", err)),
                            ExecutionProfile::empty(),
                        );
                    }
                };
                let eval = isolate.runtime.mod_evaluate(module_id);
                if let Err(err) = isolate
                    .runtime
                    .run_event_loop(deno_core::PollEventLoopOptions::default())
                    .await
                {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Err(format!("Module event loop failed: {}", err)),
                        ExecutionProfile::empty(),
                    );
                }
                if let Err(err) = eval.await {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Err(format!("Module evaluation failed: {}", err)),
                        ExecutionProfile::empty(),
                    );
                }
                isolate.handler_loaded = true;
            }
        }

        if request.request_data.mode == ExecutionMode::Module {
            let exit_value = isolate.runtime.execute_script(
                "handler.js",
                ModuleCodeString::from(
                    "const __code = globalThis.__dekaExitCode; globalThis.__dekaExitCode = undefined; __code ?? null".to_string(),
                ),
            );
            if let Ok(value) = exit_value {
                deno_core::scope!(scope, &mut isolate.runtime);
                let local = deno_core::v8::Local::new(scope, &value);
                if let Ok(parsed) = serde_v8::from_v8::<serde_json::Value>(scope, local) {
                    if let Some(code) = parsed.as_i64() {
                        isolate.active_requests = 0;
                        isolate.state = IsolateState::Idle;
                        return (
                            ExecutionOutcome::Ok(serde_json::json!({ "exit_code": code })),
                            ExecutionProfile::empty(),
                        );
                    }
                }
            }
        }

        if !isolate.handler_loaded {
            isolate.handler_loaded = true;
            if std::env::var("DEKA_DEBUG").is_ok() {
                deka_stdio::log(
                    "handler",
                    &format!("loaded {} on worker {}", key.name, self.worker_id),
                );
            }
        }

        let heap_before_bytes = isolate
            .runtime
            .v8_isolate()
            .get_heap_statistics()
            .used_heap_size();

        // Track CPU time for this execution
        let cpu_start = get_thread_cpu_time();
        let timeout_ms = self.config.request_timeout_ms;
        let timeout_flag = Arc::new(AtomicUsize::new(0));
        let timeout_flag_handle = Arc::clone(&timeout_flag);
        let isolate_handle = isolate.runtime.v8_isolate().thread_safe_handle();

        let watchdog = if timeout_ms > 0 {
            Some(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
                timeout_flag_handle.store(1, Ordering::Relaxed);
                isolate_handle.terminate_execution();
            }))
        } else {
            None
        };

        let exec_start = Instant::now();
        let mut needs_event_loop = false;
        let result = if request.request_data.mode == ExecutionMode::Module {
            isolate
                .runtime
                .execute_script(
                    "handler.js",
                    ModuleCodeString::from("undefined".to_string()),
                )
                .map_err(|err| err.to_string())
        } else if request.request_data.mode == ExecutionMode::StaticRender {
            isolate
                .runtime
                .execute_script(
                    "handler.js",
                    ModuleCodeString::from("globalThis.__dekaStaticRender()".to_string()),
                )
                .map_err(|err| err.to_string())
        } else if request.request_data.mode == ExecutionMode::Build {
            isolate
                .runtime
                .execute_script(
                    "handler.js",
                    ModuleCodeString::from(
                        "(async () => { if (typeof globalThis.__dekaBuild !== 'function') { throw new Error('build entry must export a default async function'); } const value = await globalThis.__dekaBuild(); return JSON.stringify(value, (_key, item) => item instanceof Uint8Array ? { __deka_bytes: Array.from(item) } : item); })()".to_string(),
                    ),
                )
                .map_err(|err| err.to_string())
        } else {
            // Execute the handler fetch using the globals
            const EXEC_CALL: &str = r#"globalThis.__dekaExecuteRequest()"#;
            let code = EXEC_CALL.to_string();
            isolate
                .runtime
                .execute_script("handler.js", ModuleCodeString::from(code))
                .map_err(|err| err.to_string())
        };

        let result = match result {
            Ok(value) => value,
            Err(err) => {
                if let Some(watchdog) = watchdog {
                    watchdog.abort();
                }
                if let Some(code) = parse_exit_code(&err) {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Ok(serde_json::json!({ "exit_code": code })),
                        ExecutionProfile::empty(),
                    );
                }
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                let profile = finalize_profile(
                    heap_before_bytes,
                    isolate,
                    exec_start.elapsed().as_millis() as u64,
                    0,
                    0,
                );
                return (
                    ExecutionOutcome::Err(format!("Handler execution failed: {}", err)),
                    profile,
                );
            }
        };
        let exec_script_ms = exec_start.elapsed().as_millis() as u64;

        if matches!(
            request.request_data.mode,
            ExecutionMode::Request
                | ExecutionMode::StaticRender
                | ExecutionMode::Build
                | ExecutionMode::Module
        ) {
            // Run event loop to complete async operations
            {
                deno_core::scope!(scope, &mut isolate.runtime);
                let local = deno_core::v8::Local::new(scope, &result);
                if let Ok(promise) = deno_core::v8::Local::<deno_core::v8::Promise>::try_from(local)
                {
                    match promise.state() {
                        deno_core::v8::PromiseState::Pending => {
                            needs_event_loop = true;
                        }
                        _ => needs_event_loop = false,
                    }
                }
            }
        }

        let event_loop_ms = if needs_event_loop {
            let event_start = Instant::now();
            if let Err(err) = isolate
                .runtime
                .run_event_loop(deno_core::PollEventLoopOptions::default())
                .await
            {
                if let Some(watchdog) = watchdog {
                    watchdog.abort();
                }
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                let profile = finalize_profile(
                    heap_before_bytes,
                    isolate,
                    exec_script_ms,
                    event_start.elapsed().as_millis() as u64,
                    0,
                );
                return (
                    ExecutionOutcome::Err(format!("Event loop failed: {}", err)),
                    profile,
                );
            }
            event_start.elapsed().as_millis() as u64
        } else {
            0
        };

        if let Some(watchdog) = watchdog {
            watchdog.abort();
        }

        // Calculate CPU time consumed
        let cpu_elapsed = get_thread_cpu_time() - cpu_start;
        isolate.total_cpu_time += cpu_elapsed;

        update_heap_stats(isolate);

        // Get the result from the promise
        let decode_start = Instant::now();
        let outcome = if request.request_data.mode == ExecutionMode::Module {
            let exit_value = isolate.runtime.execute_script(
                "handler.js",
                ModuleCodeString::from(
                    "const __code = globalThis.__dekaExitCode; globalThis.__dekaExitCode = undefined; __code ?? null".to_string(),
                ),
            );
            if let Ok(value) = exit_value {
                deno_core::scope!(scope, &mut isolate.runtime);
                let local = deno_core::v8::Local::new(scope, &value);
                if let Ok(parsed) = serde_v8::from_v8::<serde_json::Value>(scope, local) {
                    if let Some(code) = parsed.as_i64() {
                        ExecutionOutcome::Ok(serde_json::json!({ "exit_code": code }))
                    } else {
                        ExecutionOutcome::Ok(serde_json::Value::Null)
                    }
                } else {
                    ExecutionOutcome::Ok(serde_json::Value::Null)
                }
            } else {
                ExecutionOutcome::Ok(serde_json::Value::Null)
            }
        } else {
            deno_core::scope!(scope, &mut isolate.runtime);
            let local = deno_core::v8::Local::new(scope, &result);

            let value_result: Result<deno_core::v8::Local<deno_core::v8::Value>, String> =
                if let Ok(promise) = deno_core::v8::Local::<deno_core::v8::Promise>::try_from(local)
                {
                    match promise.state() {
                        deno_core::v8::PromiseState::Fulfilled => Ok(promise.result(scope)),
                        deno_core::v8::PromiseState::Rejected => {
                            let reason = promise.result(scope);
                            Err(format!(
                                "Handler rejected: {}",
                                reason.to_rust_string_lossy(scope)
                            ))
                        }
                        deno_core::v8::PromiseState::Pending => {
                            Err("Handler promise still pending after event loop".to_string())
                        }
                    }
                } else {
                    Ok(local)
                };

            match value_result {
                Ok(value) => match serde_v8::from_v8::<serde_json::Value>(scope, value) {
                    Ok(value) => ExecutionOutcome::Ok(value),
                    Err(err) => ExecutionOutcome::Err(format!(
                        "Handler returned non-serializable result: {}",
                        err
                    )),
                },
                Err(err) => ExecutionOutcome::Err(err),
            }
        };

        isolate.active_requests = 0;
        isolate.state = IsolateState::Idle;
        let outcome = if timeout_flag.load(Ordering::Relaxed) == 1 {
            isolate.state = IsolateState::Stuck {
                request_id: request.request_id.clone(),
                started_at: Instant::now(),
                timeout_triggered: true,
            };
            ExecutionOutcome::TimedOut
        } else {
            outcome
        };

        let result_decode_ms = decode_start.elapsed().as_millis() as u64;
        let profile = finalize_profile(
            heap_before_bytes,
            isolate,
            exec_script_ms,
            event_loop_ms,
            result_decode_ms,
        );
        (outcome, profile)
    }
}
