use super::*;

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
                // Basic console implementation
                if (typeof globalThis.console === 'undefined') {
                    globalThis.console = {
                        log(...args) { Deno.core.print(args.join(' ') + '\n'); },
                        error(...args) { Deno.core.print('[ERROR] ' + args.join(' ') + '\n'); },
                        warn(...args) { Deno.core.print('[WARN] ' + args.join(' ') + '\n'); },
                        info(...args) { Deno.core.print('[INFO] ' + args.join(' ') + '\n'); },
                        debug(...args) { Deno.core.print('[DEBUG] ' + args.join(' ') + '\n'); },
                    };
                }

                if (typeof globalThis.__dekaPrint !== 'function') {
                    globalThis.__dekaPrint = (value, isErr = false) => {
                        const text = value == null ? '' : String(value);
                        Deno.core.print(text, !!isErr);
                    };
                }

                if (!globalThis.TextEncoder) {
                    globalThis.TextEncoder = class TextEncoder {
                        encode(input) {
                            const str = String(input);
                            const utf8 = [];
                            for (let i = 0; i < str.length; i++) {
                                let charCode = str.charCodeAt(i);
                                if (charCode < 0x80) {
                                    utf8.push(charCode);
                                } else if (charCode < 0x800) {
                                    utf8.push(0xc0 | (charCode >> 6), 0x80 | (charCode & 0x3f));
                                } else if (charCode < 0xd800 || charCode >= 0xe000) {
                                    utf8.push(0xe0 | (charCode >> 12), 0x80 | ((charCode >> 6) & 0x3f), 0x80 | (charCode & 0x3f));
                                } else {
                                    i++;
                                    charCode = 0x10000 + (((charCode & 0x3ff) << 10) | (str.charCodeAt(i) & 0x3ff));
                                    utf8.push(
                                        0xf0 | (charCode >> 18),
                                        0x80 | ((charCode >> 12) & 0x3f),
                                        0x80 | ((charCode >> 6) & 0x3f),
                                        0x80 | (charCode & 0x3f)
                                    );
                                }
                            }
                            return new Uint8Array(utf8);
                        }
                    };
                }

                if (!globalThis.TextDecoder) {
                    globalThis.TextDecoder = class TextDecoder {
                        decode(bytes) {
                            if (!bytes) return '';
                            const arr = new Uint8Array(bytes);
                            let str = '';
                            let i = 0;
                            while (i < arr.length) {
                                let byte = arr[i++];
                                if (byte < 0x80) {
                                    str += String.fromCharCode(byte);
                                } else if (byte < 0xe0) {
                                    str += String.fromCharCode(((byte & 0x1f) << 6) | (arr[i++] & 0x3f));
                                } else if (byte < 0xf0) {
                                    str += String.fromCharCode(
                                        ((byte & 0x0f) << 12) | ((arr[i++] & 0x3f) << 6) | (arr[i++] & 0x3f)
                                    );
                                } else {
                                    const code =
                                        ((byte & 0x07) << 18) |
                                        ((arr[i++] & 0x3f) << 12) |
                                        ((arr[i++] & 0x3f) << 6) |
                                        (arr[i++] & 0x3f);
                                    const high = ((code - 0x10000) >> 10) | 0xd800;
                                    const low = ((code - 0x10000) & 0x3ff) | 0xdc00;
                                    str += String.fromCharCode(high, low);
                                }
                            }
                            return str;
                        }
                    };
                }

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
                        return typeof globalThis[name] === 'function';
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

                if (typeof globalThis.__bridge !== 'function') {
                    const ops = (Deno && Deno.core && Deno.core.ops) ? Deno.core.ops : {};
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
                            const shopId = globalThis.__shopId;
                            if (shopId && typeof ops.op_zega_backend === 'function' && ops.op_zega_backend(shopId) === 'zega') {
                                if (typeof ops.op_zega_kv_call === 'function') {
                                    return ops.op_zega_kv_call(shopId, String(action || ''), payload || {});
                                }
                                return { ok: false, error: 'zega KV bridge op unavailable' };
                            }
                            if (typeof ops.op_redis_call === 'function') {
                                const p = payload || {};
                                // Auto-prefix Redis keys with tenant ID (transparent to PHPX code)
                                if (shopId && p.key && action !== 'connect' && action !== 'close' && action !== 'flush' && action !== 'keys') {
                                    p.key = shopId + ':' + p.key;
                                }
                                // For 'keys' action, prefix the pattern
                                if (shopId && action === 'keys' && p.pattern) {
                                    p.pattern = shopId + ':' + p.pattern;
                                }
                                // Shard routing: always stamp the Host-derived
                                // shop slug on connect() so the Rust op can (a) pick the
                                // owning shard when no URL was passed, or (b)
                                // override a dev-default localhost URL with
                                // the shop's shard URL. See the neo4j
                                // branch above for the full reasoning.
                                if (action === 'connect') {
                                    const shardKey = globalThis.__shardKey || globalThis.__shopId;
                                    if (shardKey) {
                                        p.__account_id = shardKey;
                                    }
                                }
                                return ops.op_redis_call(String(action || ''), p);
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
                                return ops.op_php_net_proto_decode(response);
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
                            return { ok: false, error: `unknown crypto action '${act}'` };
                        }
                        if (kind === 'http') {
                            // @deka/http — outbound HTTP/1.1, HTTP/2
                            // (ALPN h2), streaming req/resp bodies,
                            // opt-in cookie jars, WebSocket client.
                            // All dispatched through a single Rust op;
                            // see crates/modules_php/src/modules/http.rs
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

                    globalThis.__bridge = (kind, action, payload) => {
                        try {
                            return __dekaFixProto(routeHostCall(String(kind || ''), String(action || ''), payload || {}));
                        } catch (err) {
                            return { ok: false, error: err && err.message ? String(err.message) : String(err) };
                        }
                    };
                    globalThis.__bridge_async = async (kind, action, payload) => {
                        try {
                            return __dekaFixProto(routeHostCall(String(kind || ''), String(action || ''), payload || {}));
                        } catch (err) {
                            return { ok: false, error: err && err.message ? String(err.message) : String(err) };
                        }
                    };
                    globalThis.__deka_wasm_call = (moduleId, exportName, payload) => {
                        const name = String(moduleId || '');
                        if (name.startsWith('__deka_')) {
                            const kind = name.replace(/^__deka_/, '');
                            return __dekaFixProto(routeHostCall(kind, exportName, payload || {}));
                        }
                        return { ok: false, error: `unknown host bridge module '${name}'` };
                    };
                    globalThis.__deka_wasm_call_async = async (moduleId, exportName, payload) => {
                        const name = String(moduleId || '');
                        if (name.startsWith('__deka_')) {
                            const kind = name.replace(/^__deka_/, '');
                            return __dekaFixProto(routeHostCall(kind, exportName, payload || {}));
                        }
                        return { ok: false, error: `unknown host bridge module '${name}'` };
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
                            if (stdout) Deno.core.print(stdout, false);
                            if (stderr) Deno.core.print(stderr, true);
                            const ok = result && result.ok !== false;
                            let exitCode = result && typeof result.exit_code === 'number' ? result.exit_code : 0;
                            if (!ok && exitCode === 0) exitCode = 1;
                            if (exitCode) globalThis.__dekaExitCode = exitCode;
                            return result;
                        }
                    };
                }

                if (typeof globalThis.__dekaExecuteRequest !== 'function') {
                    globalThis.__dekaExecuteRequest = async function() {
                        function base64Encode(bytes) {
                            if (typeof btoa === "function") {
                                let binary = "";
                                for (let i = 0; i < bytes.length; i += 1) {
                                    binary += String.fromCharCode(bytes[i]);
                                }
                                return btoa(binary);
                            }
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

                        const requestData = globalThis.__requestData || {};
                        requestData.__body = requestData.body ?? "";
                        requestData.params = requestData.params || {};
                        if (typeof requestData.json !== "function") {
                            requestData.json = async function() {
                                const body = this.__body || "";
                                if (!body) return null;
                                return JSON.parse(body);
                            };
                        }
                        if (typeof requestData.text !== "function") {
                            requestData.text = async function() {
                                return this.__body || "";
                            };
                        }
                        const context = globalThis.__requestContext || requestData.context || null;
                        const handler = globalThis.app;

                        if (!handler) {
                            throw new Error('Handler did not define "app" variable');
                        }

                        const wsEvent = requestData.__dekaWsEvent;
                        if (wsEvent) {
                            const wsHandler = handler.websocket || globalThis.__dekaWebsocket;
                            if (wsHandler) {
                                const ws = globalThis.__dekaWsCreate
                                    ? globalThis.__dekaWsCreate(requestData.__dekaWsId, requestData.__dekaWsData)
                                    : null;
                                if (wsEvent === "message" && requestData.__dekaWsBinary && Array.isArray(requestData.__dekaWsMessage)) {
                                    requestData.__dekaWsMessage = new Uint8Array(requestData.__dekaWsMessage);
                                }

                                if (wsEvent === "open" && typeof wsHandler.open === "function") {
                                    wsHandler.open(ws);
                                } else if (wsEvent === "message" && typeof wsHandler.message === "function") {
                                    wsHandler.message(ws, requestData.__dekaWsMessage);
                                } else if (wsEvent === "close" && typeof wsHandler.close === "function") {
                                    wsHandler.close(ws, requestData.__dekaWsCode, requestData.__dekaWsReason);
                                } else if (wsEvent === "drain" && typeof wsHandler.drain === "function") {
                                    wsHandler.drain(ws);
                                }
                            }

                            return { status: 204, headers: {}, body: "" };
                        }

                        let response;
                        if (typeof handler.fetch === "function") {
                            response = await handler.fetch(requestData, context);
                        } else if (typeof handler === "function") {
                            response = await handler(requestData, context);
                        } else {
                            throw new Error('Handler is not callable');
                        }

                        const normalized = globalThis.__dekaResponse || (globalThis.__dekaResponse = {
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
                        for (const key in headerTarget) {
                            delete headerTarget[key];
                        }

                        const applyHeaders = (headers) => {
                            if (!headers) return;
                            if (typeof headers.forEach === "function") {
                                headers.forEach((value, key) => {
                                    headerTarget[key] = String(value);
                                });
                                return;
                            }
                            for (const key in headers) {
                                headerTarget[key] = String(headers[key]);
                            }
                        };

                        if (response && typeof response.text === "function") {
                            if (typeof response.status === "number") {
                                normalized.status = response.status;
                            }
                            applyHeaders(response.headers);
                            if (response.upgrade) {
                                normalized.upgrade = response.upgrade;
                            }
                            const bodyValue = response.body;
                            if (bodyValue instanceof Uint8Array) {
                                normalized.body_base64 = base64Encode(bodyValue);
                            } else if (bodyValue instanceof ArrayBuffer) {
                                normalized.body_base64 = base64Encode(new Uint8Array(bodyValue));
                            } else {
                                const contentType = String(headerTarget["content-type"] || headerTarget["Content-Type"] || "").toLowerCase();
                                const isTextLike = contentType.startsWith("text/")
                                    || contentType.includes("json")
                                    || contentType.includes("javascript")
                                    || contentType.includes("xml")
                                    || contentType.includes("svg")
                                    || contentType.includes("x-www-form-urlencoded");
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
                                    const bodyObj = response.body;
                                    if (bodyObj && typeof bodyObj === "object") {
                                        const keys = Object.keys(bodyObj);
                                        if (keys.length > 0 && keys.every((k) => /^\d+$/.test(k))) {
                                            const bytes = keys
                                                .sort((a, b) => Number(a) - Number(b))
                                                .map((k) => Number(bodyObj[k]) || 0);
                                            normalized.body_base64 = base64Encode(new Uint8Array(bytes));
                                        } else {
                                            normalized.body = JSON.stringify(bodyObj);
                                        }
                                    } else {
                                        normalized.body = JSON.stringify(response.body);
                                    }
                                }
                            }
                            if (response.upgrade) {
                                normalized.upgrade = response.upgrade;
                            }
                        } else if (response != null) {
                            normalized.body = String(response);
                        }

                        return normalized;
                    };
                }

                // The deka/router module is already loaded as an extension
                // and exposes itself as globalThis.__dekaRouter automatically
            "#;

            if let Err(err) = isolate.runtime.execute_script(
                "bootstrap.js",
                ModuleCodeString::from(BOOTSTRAP.to_string()),
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
                    "JavaScript/TypeScript handlers are not supported in reboot MVP. Use .php/.phpx handlers or serve JS/TS files as static assets.".to_string(),
                ),
                ExecutionProfile::empty(),
            );
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

                // Execute the pre-bundled handler directly
                let handler_result = isolate.runtime.execute_script(
                    "handler.js",
                    ModuleCodeString::from(request.request_data.handler_code.clone()),
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

                let wrapped = format!(
                    "(function() {{\n{}\nif (typeof globalThis.app === 'undefined') {{ if (typeof __dekaDefault !== 'undefined') {{ if (typeof __dekaDefault === 'function' && typeof globalThis.__dekaNodeExpressAdapter === 'function' && (typeof __dekaDefault.handle === 'function' || typeof __dekaDefault.listen === 'function')) {{ globalThis.app = globalThis.__dekaNodeExpressAdapter(__dekaDefault); }} else if (__dekaDefault && typeof __dekaDefault === 'object' && !__dekaDefault.__dekaServer && (typeof __dekaDefault.fetch === 'function' || typeof __dekaDefault.routes === 'object')) {{ globalThis.app = globalThis.__deka.serve(__dekaDefault); }} else {{ globalThis.app = __dekaDefault; }} }} else if (typeof app !== 'undefined') {{ if (typeof app === 'function' && typeof globalThis.__dekaNodeExpressAdapter === 'function' && (typeof app.handle === 'function' || typeof app.listen === 'function')) {{ globalThis.app = globalThis.__dekaNodeExpressAdapter(app); }} else {{ globalThis.app = app; }} }} }}\n}})();",
                    handler_code
                );

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
                            "failed to fetch shop secrets from gild-vault"
                        );
                        HashMap::new()
                    }
                }
            }
        } else {
            HashMap::new()
        };

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

        let exec_mode = match request.request_data.mode {
            ExecutionMode::Module => "module",
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
            ExecutionMode::Request | ExecutionMode::Module
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
