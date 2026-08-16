use crate::{ImportDecl, ImportSpec, SourceModuleMeta};
use php_rs::parser::ast::{
    BinaryOp, ClassKind, ClassMember, Expr, ExprId, JsxChild, ObjectKey, Program, Stmt, StmtId,
    Type as AstType, UnaryOp,
};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug)]
struct EnumCaseDef {
    name: String,
    params: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JsValueKind {
    Array,
    Object,
    String,
}

enum AssignmentTarget {
    Direct(String),
    Append(String),
}

pub(crate) struct JsSubsetEmitter<'a> {
    source: &'a [u8],
    body: String,
    main_body: String,
    uses_jsx_runtime: bool,
    uses_include_stub: bool,
    scopes: Vec<HashSet<String>>,
    immutable_scopes: Vec<HashSet<String>>,
    /// Scope depth at which we entered the current function body.
    /// Variables first assigned inside a function should use `let`, not bare
    /// assignment, even if the name exists in an outer (module-level) scope —
    /// because in PHP, $var inside a function is always a NEW local, shadowing
    /// any outer binding.  We track this so `is_declared_in_function` only looks
    /// at scopes since the function entry.
    function_scope_entry: Vec<usize>,
    /// Variables that were `let`-declared inside a block scope that has since
    /// been popped.  When a variable reference hits this set (but is NOT in any
    /// live scope), the transpiler emits a warning: the variable was block-
    /// scoped and is no longer visible under JS scoping rules.
    popped_declarations: HashSet<String>,
    pub(crate) warnings: Vec<String>,
    meta: SourceModuleMeta,
    struct_schemas: Vec<(String, String)>,
    struct_names: HashSet<String>,
    struct_methods: HashMap<String, Vec<(String, String)>>,
    /// Default method bodies declared directly on a `trait`, keyed by trait
    /// name (NOT by any impl target). RFD 19: an `impl Trait for X { }` that
    /// doesn't override a trait's default method still needs that default's
    /// JS body to actually run at `x.method()` call sites -- the typechecker
    /// already allows this (a non-overridden default satisfies conformance),
    /// but codegen was only ever emitting what the impl block itself
    /// provided. Collected in a pre-pass (mirrors the enum-impl
    /// order-independence fix, deka#71) so trait declaration order relative
    /// to its impls doesn't matter.
    trait_default_methods: HashMap<String, Vec<(String, String)>>,
    enum_cases: HashMap<String, Vec<EnumCaseDef>>,
    value_kinds: HashMap<String, JsValueKind>,
    /// Tier B helpers needed by this module. Populated during AST traversal
    /// (try_rewrite_builtin inserts keys here). finish() emits each needed helper
    /// as a module-scoped `function __phpx_X(...)` declaration — NOT globalThis.
    /// This makes them tree-shakable: the bundler DCE can see that a helper is
    /// unreferenced and prune it, rather than assuming globalThis.X might be
    /// accessed from anywhere.
    needed_helpers: BTreeSet<&'static str>,
}

mod assignment;
mod builtins;
mod expr;
mod jsx;
mod names;
mod program;
mod schema;
mod scope;

impl<'a> JsSubsetEmitter<'a> {
    pub(crate) fn new(source: &'a [u8], meta: SourceModuleMeta) -> Self {
        Self {
            source,
            body: String::new(),
            main_body: String::new(),
            uses_jsx_runtime: false,
            uses_include_stub: false,
            function_scope_entry: Vec::new(),
            scopes: vec![HashSet::new()],
            immutable_scopes: vec![HashSet::new()],
            popped_declarations: HashSet::new(),
            warnings: Vec::new(),
            meta,
            struct_schemas: Vec::new(),
            struct_names: HashSet::new(),
            struct_methods: HashMap::new(),
            trait_default_methods: HashMap::new(),
            enum_cases: HashMap::new(),
            value_kinds: HashMap::new(),
            needed_helpers: BTreeSet::new(),
        }
    }

    // ======================================================================
    // Polyfill prelude classification (JS-first refactor, Phase 1)
    //
    // Each globalThis entry is classified as:
    //   (a) Compile-time rewrite — PHP-named function with clean inline JS.
    //       These are intercepted in emit_expr Expr::Call and emitted inline.
    //       The globalThis polyfill is DELETED from the prelude.
    //   (b) Runtime helper — genuinely complex, can't inline. Keep on
    //       globalThis (already __phpx_ or __deka_ mangled).
    //   (c) Mission helper — implements PHPX intentional JS divergences.
    //       Keep on globalThis.
    //
    // CLASS (a) — compile-time rewrite candidates (REMOVED from prelude):
    //   chr          -> String.fromCharCode(($n) & 0xff)
    //   ord          -> (String($s).length ? String($s).charCodeAt(0) : 0)
    //   strlen       -> $s.length
    //   substr       -> $s.slice(...)
    //   ltrim        -> String($s).trimStart() (no $chars arg) / keep polyfill for $chars
    //   rtrim        -> String($s).trimEnd()  (no $chars arg) / keep polyfill for $chars
    //   trim         -> String($s).trim()     (no $chars arg) / keep polyfill for $chars
    //   strpos       -> (() => { const __i = String($h).indexOf(String($n)); return __i >= 0 ? __i : false; })()
    //   strrpos      -> similar with lastIndexOf
    //   str_starts_with -> String($h).startsWith(String($n))
    //   str_ends_with   -> String($h).endsWith(String($n))
    //   str_contains    -> String($h).includes(String($n))
    //   strtolower   -> String($s).toLowerCase()
    //   strtoupper   -> String($s).toUpperCase()
    //   array_key_exists -> ($a != null && typeof $a === 'object' && Object.prototype.hasOwnProperty.call($a, $k))
    //   in_array     -> (Array.isArray($h) && $h.includes($n))
    //   array_map    -> $a.map($fn)
    //   array_filter -> $a.filter($fn)
    //   explode      -> String($s).split(String($sep))
    //   implode      -> $a.join(String($g))
    //   count        -> $x.length for known arrays/strings, Object.keys($x).length for known objects
    //   time         -> Math.floor(Date.now() / 1000)
    //   is_array     -> Array.isArray($x)
    //
    // CLASS (b) — runtime helpers (KEEP, already mangled):
    //   __phpx_is_struct       — struct type guard
    //   __phpx_func_num_args   — arguments.length proxy
    //   __phpx_func_get_args   — arguments slice proxy
    //   __phpx_func_get_arg    — arguments index proxy
    //   __deka_chr             — backing impl for chr (used by string/chr.phpx stdlib module)
    //   __deka_ord             — backing impl for ord (used by string/ord.phpx stdlib module)
    //   __deka_object_set      — mutable object field set helper
    //   __phpx_base64_table    — base64 lookup table
    //   base64_encode          — base64 encoding (too complex to inline)
    //   base64_decode          — base64 decoding (too complex to inline)
    //   __phpx_sha256_hex      — SHA-256 pure-JS impl
    //   __phpx_hex_to_binary   — hex-to-binary conversion
    //   __phpx_hmac_sha256_hex — HMAC-SHA256 pure-JS impl
    //   __phpx_node_crypto     — node:crypto lazy loader
    //   hash                   — multi-algo hash dispatcher
    //   hash_hmac              — multi-algo HMAC dispatcher
    //   hash_equals            — constant-time string compare
    //   __phpx_symbol_table    — runtime symbol table
    //   __deka_symbol_set      — symbol table write
    //   __deka_symbol_get      — symbol table read
    //   __deka_symbol_exists   — symbol table check
    //   __phpx_array_cursor    — WeakMap for array cursor state
    //   __deka_array_cursor    — array cursor ops (reset/next/prev/current/key/end)
    //
    // CLASS (c) — mission helpers (KEEP):
    //   panic            — PHPX intentional divergence: panics throw Error
    //   function_exists  — PHPX runtime reflection
    //   class_exists     — PHPX runtime reflection
    //   class_alias      — PHPX runtime reflection (stub)
    //   defined          — PHPX runtime reflection
    //   getenv           — PHPX process.env bridge returning false on missing
    //   is_promise       — PHPX async helper
    //   GLOBALS          — PHPX global variable bag
    //   JSON_ERROR_*     — PHPX JSON error constants (11 entries)
    //   __phpxStructMethods — struct method registry
    // ======================================================================
    pub(crate) fn finish(self) -> String {
        let mut out = String::new();
        out.push_str("// Generated by deka build. Do not edit manually.\n");
        out.push_str("// Target semantics: JavaScript runtime semantics.\n");
        out.push_str("export const phpxBuildMode = \"subset-ast\";\n");
        out.push_str("export const phpxTargetSemantics = \"js\";\n\n");

        // --- Demand-driven mission/runtime globalThis prelude (#47) ---
        //
        // Historically every entry below was emitted unconditionally on every
        // compiled program: ~74 lines of `globalThis.X ??= ...` plumbing even
        // for a 4-line program that referenced none of it (#47), and
        // `--treeshake` made it worse because DCE cannot see through
        // `globalThis.X ??= ...` writes.
        //
        // By the time `finish()` runs, AST traversal is complete and
        // `self.body` / `self.main_body` hold the program's fully emitted JS.
        // Every builtin-style call/reference this emitter doesn't otherwise
        // special-case resolves through the `Expr::Variable`/`Expr::Call`
        // fallback paths in emitter/expr.rs, which always emit the fully
        // qualified `globalThis.<name>` form (never a bare identifier) — see
        // `Expr::Variable`'s undeclared-identifier branch. So a plain
        // substring scan of the already-emitted body for `globalThis.<name>`
        // finds every real usage without needing per-call-site
        // instrumentation, exactly mirroring what `needed_helpers` already
        // does for Tier B helpers (base64/hash/date/pack) below.
        //
        // A handful of entries call each other internally purely through
        // their own `globalThis.*` bodies (e.g. `header` writes through
        // `globalThis.__phpxCurrentResponse`, `phpxWrapHandler` calls
        // `phpxStartBuffer`/`phpxEndBuffer`) — those internal edges can't
        // show up in the *user's* body text, so `GLOBAL_DEPS` below expands
        // the requested set transitively before anything is emitted.
        let program_text = format!("{}\n{}", self.body, self.main_body);
        let mut needed_globals: BTreeSet<&'static str> = LEAF_GLOBALS
            .iter()
            .copied()
            .filter(|&name| body_refs_global(&program_text, name))
            .collect();
        loop {
            let mut grew = false;
            for name in needed_globals.clone() {
                for &dep in global_deps(name) {
                    if needed_globals.insert(dep) {
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        let want = |name: &str| needed_globals.contains(name);

        // --- Class (c): mission helpers ---
        if want("panic") {
            out.push_str("globalThis.panic ??= (msg) => { throw new Error(String(msg)); };\n");
        }
        // function_exists / class_exists are compile-time rewrites in try_rewrite_builtin.
        // No globalThis polyfill emitted here.
        if want("class_alias") {
            out.push_str("globalThis.class_alias ??= () => false;\n\n");
        }
        if want("defined") {
            out.push_str("globalThis.defined ??= (name) => Object.prototype.hasOwnProperty.call(globalThis, String(name));\n\n");
        }

        // --- Class (b): runtime helpers ---
        if want("__phpx_is_struct") {
            out.push_str("globalThis.__phpx_is_struct ??= (value, name) => Boolean(value && typeof value === 'object' && value.__struct === name);\n\n");
        }
        // func_num_args/func_get_args/func_get_arg are emitted directly as
        // `globalThis.__phpx_func_X(...)` literal calls in expr.rs regardless
        // of try_rewrite_builtin, so the same body-text scan covers them.
        if want("__phpx_func_num_args") {
            out.push_str("globalThis.__phpx_func_num_args ??= (args) => args.length;\n");
        }
        if want("__phpx_func_get_args") {
            out.push_str(
                "globalThis.__phpx_func_get_args ??= (args) => Array.prototype.slice.call(args);\n",
            );
        }
        if want("__phpx_func_get_arg") {
            out.push_str("globalThis.__phpx_func_get_arg ??= (args, idx) => (idx >= 0 && idx < args.length ? args[idx] : null);\n\n");
        }

        // Class (a) entries REMOVED: chr, ord, strlen, substr, ltrim, rtrim, trim,
        // strpos, strrpos, str_starts_with, str_ends_with, str_contains,
        // strtolower, strtoupper, array_key_exists, in_array, explode, implode,
        // count, time, is_array, array_map, array_filter — now compile-time rewrites in emit_expr.

        // --- Class (c): mission helpers (continued) ---
        if want("getenv") {
            out.push_str("globalThis.getenv ??= (name) => { const key = String(name ?? ''); const env = globalThis.process && globalThis.process.env ? globalThis.process.env : null; if (!env || !Object.prototype.hasOwnProperty.call(env, key)) return false; const value = env[key]; return value === undefined || value === null ? false : String(value); };\n");
        }
        if want("is_promise") {
            out.push_str("globalThis.is_promise ??= (value) => Boolean(value && typeof value === 'object' && typeof value.then === 'function');\n");
        }
        if want("GLOBALS") {
            out.push_str("globalThis.GLOBALS ??= {};\n");
        }
        const JSON_ERROR_CONSTS: &[(&str, u8)] = &[
            ("JSON_ERROR_NONE", 0),
            ("JSON_ERROR_DEPTH", 1),
            ("JSON_ERROR_STATE_MISMATCH", 2),
            ("JSON_ERROR_CTRL_CHAR", 3),
            ("JSON_ERROR_SYNTAX", 4),
            ("JSON_ERROR_UTF8", 5),
            ("JSON_ERROR_RECURSION", 6),
            ("JSON_ERROR_INF_OR_NAN", 7),
            ("JSON_ERROR_UNSUPPORTED_TYPE", 8),
            ("JSON_ERROR_INVALID_PROPERTY_NAME", 9),
            ("JSON_ERROR_UTF16", 10),
        ];
        for &(name, value) in JSON_ERROR_CONSTS {
            if want(name) {
                out.push_str(&format!("globalThis.{} ??= {};\n", name, value));
            }
        }

        // --- Class (b): runtime helpers (continued) ---
        if want("__deka_chr") {
            out.push_str("globalThis.__deka_chr ??= (code) => String.fromCharCode((Number(code) || 0) & 0xff);\n");
        }
        if want("__deka_ord") {
            out.push_str("globalThis.__deka_ord ??= (s) => { const str = String(s ?? ''); return str.length > 0 ? str.charCodeAt(0) : 0; };\n");
        }
        if want("__deka_object_set") {
            out.push_str("globalThis.__deka_object_set ??= (obj, key, value) => { if (obj && typeof obj === 'object') { obj[key] = value; } return obj; };\n");
        }
        // --- Tier B helpers: module-scoped function declarations (DCE-visible) ---
        // Emitted only when needed (self.needed_helpers tracks which ones were
        // referenced during AST traversal). Plain `function` declarations are in
        // lexical scope — the bundler sees references and can prune unused helpers.
        // Inter-helper calls use plain names (not globalThis.X) so DCE chains work.
        // hash_equals is always an inline IIFE — no helper declaration needed.
        out.push_str(&emit_needed_helpers(&self.needed_helpers));

        // hash_equals is a compile-time inline IIFE rewrite in try_rewrite_builtin. No globalThis install.
        if want("__phpx_symbol_table") {
            out.push_str("globalThis.__phpx_symbol_table ??= Object.create(null);\n");
        }
        if want("__deka_symbol_set") {
            out.push_str("globalThis.__deka_symbol_set ??= (name, value) => { const key = String(name); globalThis.__phpx_symbol_table[key] = value; return true; };\n");
        }
        if want("__deka_symbol_get") {
            out.push_str("globalThis.__deka_symbol_get ??= (name) => { const key = String(name); return Object.prototype.hasOwnProperty.call(globalThis.__phpx_symbol_table, key) ? globalThis.__phpx_symbol_table[key] : null; };\n");
        }
        if want("__deka_symbol_exists") {
            out.push_str("globalThis.__deka_symbol_exists ??= (name) => { const key = String(name); return Object.prototype.hasOwnProperty.call(globalThis.__phpx_symbol_table, key); };\n");
        }
        if want("__phpx_array_cursor") {
            out.push_str("globalThis.__phpx_array_cursor ??= new WeakMap();\n");
        }
        if want("__deka_array_cursor") {
            out.push_str("globalThis.__deka_array_cursor ??= (arr, action) => { if (!arr || (typeof arr !== 'object' && !Array.isArray(arr))) return null; const map = globalThis.__phpx_array_cursor; let state = map.get(arr); if (!state) { state = { idx: 0 }; map.set(arr, state); } const keys = Object.keys(arr); if (keys.length === 0) return null; const clamp = () => { if (state.idx < 0) state.idx = 0; if (state.idx >= keys.length) state.idx = keys.length - 1; }; switch (String(action)) { case 'reset': state.idx = 0; break; case 'end': state.idx = keys.length - 1; break; case 'next': state.idx += 1; if (state.idx >= keys.length) return null; break; case 'prev': state.idx -= 1; if (state.idx < 0) return null; break; case 'pos': case 'current': break; case 'key': break; default: return null; } clamp(); const key = keys[state.idx]; if (String(action) === 'key') return key; return arr[key]; };\n");
        }
        // PHP filesystem builtins — polyfilled using __dekaFs (Deno/Node FS adapter injected by runtime).
        // Capability-gated FS: __dekaFs is the runtime-injected adapter. NEVER fall
        // through to raw Deno.* — that bypasses the deka.json security manifest.
        if want("__phpx_stat") {
            out.push_str("globalThis.__phpx_stat ??= (p) => { try { const fs = (typeof __dekaFs !== 'undefined' && __dekaFs) ? __dekaFs : null; if (fs && typeof fs.statSync === 'function') return fs.statSync(String(p)); } catch(_) {} return null; };\n");
        }
        if want("is_file") {
            out.push_str("globalThis.is_file ??= (p) => { const s = globalThis.__phpx_stat(p); if (!s) return false; return typeof s.isFile === 'function' ? s.isFile() : !!s.isFile; };\n");
        }
        if want("is_dir") {
            out.push_str("globalThis.is_dir ??= (p) => { const s = globalThis.__phpx_stat(p); if (!s) return false; return typeof s.isDirectory === 'function' ? s.isDirectory() : !!s.isDirectory; };\n");
        }
        if want("mkdir") {
            out.push_str("globalThis.mkdir ??= (p, _mode, recursive) => { try { const fs = (typeof __dekaFs !== 'undefined' && __dekaFs) ? __dekaFs : null; if (fs && typeof fs.mkdirSync === 'function') { fs.mkdirSync(String(p), { recursive: !!recursive }); return true; } } catch(_) {} return false; };\n");
        }
        if want("file") {
            out.push_str("globalThis.file ??= (p) => { try { const fs = (typeof __dekaFs !== 'undefined' && __dekaFs) ? __dekaFs : null; if (!fs || typeof fs.readFileSync !== 'function') return false; const raw = fs.readFileSync(String(p)); const text = typeof raw === 'string' ? raw : (new TextDecoder()).decode(raw); if (text === null) return false; const lines = text.split('\\n'); return lines[lines.length - 1] === '' ? lines.slice(0, -1).map((l, i) => l + '\\n') : lines.map((l, i, a) => i < a.length - 1 ? l + '\\n' : l); } catch(_) { return false; } };\n");
        }
        // PHP math and type builtins.
        // max / min are compile-time rewrites in try_rewrite_builtin. No globalThis polyfill needed.
        // is_int / is_float / is_numeric / is_string / is_object are compile-time rewrites
        // in try_rewrite_builtin (IIFE binding the arg once). No globalThis polyfill needed.
        // gettype / get_object_vars / mt_rand are compile-time rewrites in try_rewrite_builtin.
        // PHP string/time builtins.
        // ltrim / rtrim are compile-time rewrites in try_rewrite_builtin (no $chars form).
        // The $chars form falls back to globalThis via the unknown-call path, but that is
        // only reached when 2 args are passed; the 1-arg no-chars form is the common case
        // and is rewritten inline. No globalThis polyfill emitted here.
        // str_replace is a compile-time rewrite in try_rewrite_builtin. No globalThis polyfill needed.
        // preg_match / preg_replace / parse_url / microtime / strtotime / urlencode / urldecode
        // are compile-time rewrites in try_rewrite_builtin. No globalThis polyfills needed.
        // rawurlencode / dechex / hexdec / intval / floatval / boolval / strval /
        // array_slice / array_map / array_filter are compile-time rewrites in try_rewrite_builtin.
        // No globalThis polyfills needed.
        // dechex/hexdec are compile-time rewrites in JsSubsetEmitter::emit_builtin_call.
        // pack / date / gmdate — moved to emit_needed_helpers(); emitted only when needed.
        // No globalThis.pack / globalThis.date / globalThis.gmdate install.
        if want("error_log") {
            out.push_str("globalThis.error_log ??= (msg, type, dest) => { if (type === 3 && dest) { try { const fs = (typeof __dekaFs !== 'undefined' && __dekaFs) ? __dekaFs : null; if (fs && typeof fs.appendFileSync === 'function') { fs.appendFileSync(String(dest), String(msg ?? '')); return true; } } catch(_) {} } console.error(String(msg ?? '')); return true; };\n");
        }
        if want("error_get_last") {
            out.push_str("globalThis.error_get_last ??= () => null;\n");
        }
        if want("set_error_handler") {
            out.push_str("globalThis.set_error_handler ??= () => null;\n");
        }
        if want("register_shutdown_function") {
            out.push_str("globalThis.register_shutdown_function ??= () => undefined;\n");
        }
        // array_slice is a compile-time rewrite in try_rewrite_builtin. No globalThis polyfill needed.
        // htmlspecialchars is a compile-time rewrite in JsSubsetEmitter::emit_builtin_call.
        // PHP serve adapter helper — allows PHPX template files to export themselves as ESM handlers.
        // Mangled __phpx_X name (NOT a plain `servePhp` global) so it can't collide with a user-defined
        // `$servePhp` variable in PHPX source. Tree-shakable through bundler DCE since it's a
        // string-keyed assignment instead of a free identifier.
        if want("__phpx_serve_php") {
            out.push_str("globalThis.__phpx_serve_php ??= (path) => { if (globalThis.__dekaPhp && typeof globalThis.__dekaPhp.servePhp === 'function') { return globalThis.__dekaPhp.servePhp(String(path || '')); } return null; };\n");
        }
        // PHP output buffering — enables echo/header() pattern in $app request handlers.
        if want("__phpxCurrentResponse") {
            out.push_str(
                "globalThis.__phpxCurrentResponse ??= { status: 200, headers: {}, body: '' };\n",
            );
        }
        if want("header") {
            out.push_str("globalThis.header ??= (str) => { const s = String(str ?? ''); if (/^HTTP\\/[0-9]/i.test(s)) { const m = s.match(/^HTTP\\/[0-9.]+\\s+(\\d+)/i); if (m) globalThis.__phpxCurrentResponse.status = parseInt(m[1]); } else { const colon = s.indexOf(':'); if (colon > 0) { const name = s.slice(0, colon).trim().toLowerCase(); const value = s.slice(colon + 1).trim(); if (name === 'location' && globalThis.__phpxCurrentResponse.status === 200) globalThis.__phpxCurrentResponse.status = 302; globalThis.__phpxCurrentResponse.headers[name] = value; } } };\n");
        }
        if want("__phpxPrintOrig") {
            out.push_str("globalThis.__phpxPrintOrig ??= null;\n");
        }
        if want("phpxStartBuffer") {
            out.push_str("globalThis.phpxStartBuffer ??= () => { globalThis.__phpxCurrentResponse = { status: 200, headers: {}, body: '' }; if (!globalThis.__phpxPrintOrig) { globalThis.__phpxPrintOrig = globalThis.__dekaPrint; } globalThis.__dekaPrint = (v) => { globalThis.__phpxCurrentResponse.body += String(v ?? ''); }; };\n");
        }
        if want("phpxEndBuffer") {
            out.push_str("globalThis.phpxEndBuffer ??= () => { if (globalThis.__phpxPrintOrig) { globalThis.__dekaPrint = globalThis.__phpxPrintOrig; globalThis.__phpxPrintOrig = null; } return globalThis.__phpxCurrentResponse; };\n");
        }
        // Wrap a PHP-style echo/header handler so it always returns the buffered response.
        //
        // LEGACY-COMPAT ONLY: this wrapper is the linkhash-registry escape hatch for handlers
        // built in PHP-template style (echo + header()). It contradicts PHPX's "errors as values /
        // no exceptions" core principle by catching thrown errors. The catch is structured: it
        // emits a 500 JSON envelope with kind='phpxWrapHandler.error' so an outage is visible in
        // logs AND surfaces a typed error to the caller. New PHPX handlers should return
        // Result<Response, Error> directly; only linkhash uses this wrapper today. Followup issue
        // tracks migrating linkhash off this pattern.
        if want("phpxWrapHandler") {
            out.push_str("globalThis.phpxWrapHandler ??= (fn) => async (req, ctx) => { phpxStartBuffer(); try { const r = await fn(req, ctx); if (r != null) return r; } catch(_e) { const stack = _e && _e.stack ? String(_e.stack) : ''; const err = { kind: 'phpxWrapHandler.error', message: String(_e), stack }; if (typeof Deno !== 'undefined' && Deno.core && typeof Deno.core.print === 'function') { Deno.core.print('[phpxWrap] ' + JSON.stringify(err) + '\\n', true); } return { status: 500, headers: { 'content-type': 'application/json' }, body: JSON.stringify({ error: 'internal_error', kind: err.kind }) }; } return phpxEndBuffer(); };\n\n");
        }

        // JSX runtime — converts JSX calls to HTML strings for both server and browser targets.
        // `self.uses_jsx_runtime` is already set precisely (jsx.rs) whenever a
        // JsxElement/JsxFragment was actually emitted, so this is gated on
        // that flag directly rather than a body-text scan.
        if self.uses_jsx_runtime {
            out.push_str("globalThis.jsx ??= (tag, props) => {\n");
            out.push_str("  if (typeof tag === 'function') return tag(props ?? {});\n");
            out.push_str("  const attrs = Object.entries(props ?? {}).filter(([k]) => k !== 'children').map(([k, v]) => ` ${k}=\"${String(v ?? '').replace(/\\\"/g, '&quot;')}\"`).join('');\n");
            out.push_str("  const children = props?.children;\n");
            out.push_str("  let inner = '';\n");
            out.push_str("  if (children !== undefined) {\n");
            out.push_str("    if (Array.isArray(children)) { inner = children.map((c) => String(c ?? '')).join(''); }\n");
            out.push_str("    else { inner = String(children); }\n");
            out.push_str("  }\n");
            out.push_str("  if (tag === '__fragment__') return inner;\n");
            out.push_str("  return `<${tag}${attrs}>${inner}</${tag}>`;\n");
            out.push_str("};\n");
            out.push_str("globalThis.jsxs ??= globalThis.jsx;\n");
            out.push_str("const jsx = globalThis.jsx;\n");
            out.push_str("const jsxs = globalThis.jsxs;\n\n");
        }

        let mut imports = self.meta.imports.clone();
        let deka_i_locals = extract_deka_i_imports(&mut imports);

        for decl in &imports {
            out.push_str("import { ");
            for (idx, spec) in decl.specs.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                if spec.imported == spec.local {
                    out.push_str(&spec.local);
                } else {
                    out.push_str(&format!("{} as {}", spec.imported, spec.local));
                }
            }
            out.push_str(&format!(" }} from '{}';\n", decl.from));
        }

        if !imports.is_empty() {
            out.push('\n');
        }

        if !deka_i_locals.is_empty() {
            out.push_str("const __phpxTypeRegistry = {};\n\n");
            for (name, schema) in &self.struct_schemas {
                out.push_str(&format!(
                    "__phpxTypeRegistry[{}] = {};\n",
                    json_string(name),
                    schema
                ));
            }
            out.push('\n');
            out.push_str(&emit_deka_i_runtime());
            for local in &deka_i_locals {
                out.push_str(&format!("const {} = __deka_i;\n", local));
            }
            out.push('\n');
        }

        if self.uses_include_stub {
            out.push_str("function __phpx_include(path, kind) {\n");
            out.push_str("  throw new Error(`include/require not supported in JS subset emitter: ${kind} ${path}`);\n");
            out.push_str("}\n\n");
        }

        // __phpxStructMethods is read by struct-literal-with-methods codegen
        // (see expr.rs's StructLiteral handling, `globalThis.__phpxStructMethods
        // ? globalThis.__phpxStructMethods[...]`), so gate it on either this
        // file registering methods itself or the emitted body reading it.
        if !self.struct_methods.is_empty() || want("__phpxStructMethods") {
            out.push_str("globalThis.__phpxStructMethods ??= Object.create(null);\n");
        }
        if !self.struct_methods.is_empty() {
            for (name, methods) in &self.struct_methods {
                out.push_str(&format!(
                    "globalThis.__phpxStructMethods[{}] = {{ {} }};\n",
                    json_string(name),
                    methods
                        .iter()
                        .map(|(method_name, body)| format!("{}: {}", method_name, body))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            out.push('\n');
        }

        out.push_str(&self.body);

        if !self.main_body.is_empty() {
            out.push('\n');
            out.push_str("const __phpx_main = async () => {\n");
            out.push_str(&self.main_body);
            out.push_str("};\n");
            out.push_str("await __phpx_main();\n");
        }

        if !self.meta.export_specs.is_empty() {
            out.push('\n');
            out.push_str("export { ");
            for (idx, spec) in self.meta.export_specs.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                if spec.imported == spec.local {
                    out.push_str(&spec.local);
                } else {
                    out.push_str(&format!("{} as {}", spec.local, spec.imported));
                }
            }
            out.push_str(" };\n");
        }

        out
    }
}

fn unescape_php_single(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\'') => out.push('\''),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn unescape_php_double(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        match next {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'v' => out.push('\x0b'),
            'e' => out.push('\x1b'),
            'f' => out.push('\x0c'),
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            '$' => out.push('$'),
            'x' => {
                let mut hex = String::new();
                for _ in 0..2 {
                    if let Some(h) = chars.peek().copied() {
                        if h.is_ascii_hexdigit() {
                            hex.push(h);
                            chars.next();
                        }
                    }
                }
                if hex.is_empty() {
                    out.push('x');
                } else if let Ok(code) = u8::from_str_radix(&hex, 16) {
                    out.push(code as char);
                }
            }
            '0'..='7' => {
                let mut oct = String::new();
                oct.push(next);
                for _ in 0..2 {
                    if let Some(o) = chars.peek().copied() {
                        if o >= '0' && o <= '7' {
                            oct.push(o);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(code) = u32::from_str_radix(&oct, 8) {
                    if let Some(c) = std::char::from_u32(code) {
                        out.push(c);
                    }
                }
            }
            other => {
                out.push(other);
            }
        }
    }
    out
}

/// Every name `finish()` may conditionally install onto `globalThis`, keyed
/// by exactly the identifier that appears after `globalThis.` in the
/// already-emitted program body when that entry is genuinely referenced.
/// See the demand-driven-prelude comment inside `finish()` (#47).
const LEAF_GLOBALS: &[&str] = &[
    "panic",
    "class_alias",
    "defined",
    "__phpx_is_struct",
    "__phpx_func_num_args",
    "__phpx_func_get_args",
    "__phpx_func_get_arg",
    "getenv",
    "is_promise",
    "GLOBALS",
    "JSON_ERROR_NONE",
    "JSON_ERROR_DEPTH",
    "JSON_ERROR_STATE_MISMATCH",
    "JSON_ERROR_CTRL_CHAR",
    "JSON_ERROR_SYNTAX",
    "JSON_ERROR_UTF8",
    "JSON_ERROR_RECURSION",
    "JSON_ERROR_INF_OR_NAN",
    "JSON_ERROR_UNSUPPORTED_TYPE",
    "JSON_ERROR_INVALID_PROPERTY_NAME",
    "JSON_ERROR_UTF16",
    "__deka_chr",
    "__deka_ord",
    "__deka_object_set",
    "__phpx_symbol_table",
    "__deka_symbol_set",
    "__deka_symbol_get",
    "__deka_symbol_exists",
    "__phpx_array_cursor",
    "__deka_array_cursor",
    "__phpx_stat",
    "is_file",
    "is_dir",
    "mkdir",
    "file",
    "error_log",
    "error_get_last",
    "set_error_handler",
    "register_shutdown_function",
    "__phpx_serve_php",
    "__phpxCurrentResponse",
    "header",
    "__phpxPrintOrig",
    "phpxStartBuffer",
    "phpxEndBuffer",
    "phpxWrapHandler",
    "__phpxStructMethods",
];

/// Internal dependency edges between `LEAF_GLOBALS` entries: each key's
/// polyfill body itself references the listed names via their own
/// `globalThis.*` reads/writes, which can never show up in a scan of the
/// *user's* emitted body text — so once a name is wanted, its deps must be
/// forced in too. `finish()` expands this to a fixed point before emitting.
fn global_deps(name: &str) -> &'static [&'static str] {
    match name {
        "__deka_symbol_set" | "__deka_symbol_get" | "__deka_symbol_exists" => {
            &["__phpx_symbol_table"]
        }
        "__deka_array_cursor" => &["__phpx_array_cursor"],
        "is_file" | "is_dir" => &["__phpx_stat"],
        "header" => &["__phpxCurrentResponse"],
        "phpxStartBuffer" | "phpxEndBuffer" => &["__phpxCurrentResponse", "__phpxPrintOrig"],
        "phpxWrapHandler" => &[
            "phpxStartBuffer",
            "phpxEndBuffer",
            "__phpxCurrentResponse",
            "__phpxPrintOrig",
        ],
        _ => &[],
    }
}

/// True if `text` (the already-emitted program body) references
/// `globalThis.<name>` as a whole identifier — not as a substring of a
/// longer one (so `globalThis.is_file` doesn't false-positive on some
/// hypothetical `globalThis.is_filed`).
fn body_refs_global(text: &str, name: &str) -> bool {
    let pat = format!("globalThis.{}", name);
    let bytes = text.as_bytes();
    let mut start = 0;
    while let Some(pos) = text[start..].find(pat.as_str()) {
        let idx = start + pos;
        let after = idx + pat.len();
        let boundary_ok =
            after >= bytes.len() || !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
        if boundary_ok {
            return true;
        }
        start = idx + 1;
    }
    false
}

/// Emit module-scoped `function __phpx_X(...)` declarations for every Tier B
/// helper that was referenced during AST traversal (tracked in
/// `JsSubsetEmitter::needed_helpers`).
///
/// Each helper is a plain `function` declaration — NOT a `globalThis.X ??= …`
/// assignment.  Plain function declarations are in lexical scope: the bundler
/// can see that `__phpx_base64_encode` is called from line N and is defined at
/// line M, and can prune it via DCE if no caller survives tree-shaking.
///
/// Helpers are emitted in dependency order.  Inter-helper calls use plain
/// names (e.g. `__phpx_sha256_hex(…)` not `globalThis.__phpx_sha256_hex(…)`)
/// so the dependency graph is visible to the bundler.
///
/// Dependency map (each key depends on its values):
///   base64_encode  -> [base64_table]
///   base64_decode  -> [base64_table]
///   hash           -> [sha256_hex, hex_to_binary, node_crypto]
///   hash_hmac      -> [hmac_sha256_hex, hex_to_binary, node_crypto, sha256_hex]
///   date           -> [date_format]
///   gmdate         -> [date_format]
///   pack           -> []
fn emit_needed_helpers(needed: &BTreeSet<&'static str>) -> String {
    // Compute transitive closure of required internal helpers.
    // Internal helpers (not user-callable) are tracked by short name without __phpx_ prefix.
    let mut emit_base64_table = false;
    let mut emit_base64_encode = false;
    let mut emit_base64_decode = false;
    let mut emit_sha256_hex = false;
    let mut emit_hex_to_binary = false;
    let mut emit_hmac_sha256_hex = false;
    let mut emit_node_crypto = false;
    let mut emit_hash = false;
    let mut emit_hash_hmac = false;
    let mut emit_date_format = false;
    let mut emit_date = false;
    let mut emit_gmdate = false;
    let mut emit_pack = false;

    for &key in needed {
        match key {
            "base64_encode" => {
                emit_base64_table = true;
                emit_base64_encode = true;
            }
            "base64_decode" => {
                emit_base64_table = true;
                emit_base64_decode = true;
            }
            "hash" => {
                emit_sha256_hex = true;
                emit_hex_to_binary = true;
                emit_node_crypto = true;
                emit_hash = true;
            }
            "hash_hmac" => {
                emit_sha256_hex = true;
                emit_hex_to_binary = true;
                emit_hmac_sha256_hex = true;
                emit_node_crypto = true;
                emit_hash_hmac = true;
            }
            "date" => {
                emit_date_format = true;
                emit_date = true;
            }
            "gmdate" => {
                emit_date_format = true;
                emit_gmdate = true;
            }
            "pack" => {
                emit_pack = true;
            }
            _ => {}
        }
    }

    if !emit_base64_table
        && !emit_base64_encode
        && !emit_base64_decode
        && !emit_sha256_hex
        && !emit_hex_to_binary
        && !emit_hmac_sha256_hex
        && !emit_node_crypto
        && !emit_hash
        && !emit_hash_hmac
        && !emit_date_format
        && !emit_date
        && !emit_gmdate
        && !emit_pack
    {
        return String::new();
    }

    let mut out = String::new();
    out.push_str(
        "// Tier B helpers — module-scoped, emitted only when referenced (DCE-visible).\n",
    );

    if emit_base64_table {
        out.push_str("const __phpx_base64_table = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';\n");
    }
    if emit_base64_encode {
        out.push_str("function __phpx_base64_encode(input) { const str = String(input ?? ''); const tbl = __phpx_base64_table; let out = ''; for (let i = 0; i < str.length; i += 3) { const b0 = str.charCodeAt(i) & 0xff; const b1 = i + 1 < str.length ? str.charCodeAt(i + 1) & 0xff : NaN; const b2 = i + 2 < str.length ? str.charCodeAt(i + 2) & 0xff : NaN; const n = (b0 << 16) | ((Number.isNaN(b1) ? 0 : b1) << 8) | (Number.isNaN(b2) ? 0 : b2); out += tbl[(n >> 18) & 63]; out += tbl[(n >> 12) & 63]; out += Number.isNaN(b1) ? '=' : tbl[(n >> 6) & 63]; out += Number.isNaN(b2) ? '=' : tbl[n & 63]; } return out; }\n");
    }
    if emit_base64_decode {
        // Fix (issue #34): the old guard used `n2 < -1 || n3 < -1`, but
        // tbl.indexOf() returns AT MOST -1, so `< -1` is unreachable.  Invalid
        // non-padding chars in positions 2-3 were therefore silently decoded as
        // 0, producing garbage bytes instead of returning false in strict mode.
        // Fix: look up positions 2-3 unconditionally, reject on -1 only when
        // the char is NOT the padding `=` character, and reject malformed
        // padding such as `ab=Y`.
        out.push_str("function __phpx_base64_decode(input, strict = false) { const src = String(input ?? '').replace(/\\s+/g, ''); if (src.length % 4 !== 0) return strict ? false : ''; const tbl = __phpx_base64_table; let out = ''; for (let i = 0; i < src.length; i += 4) { const c0 = src[i], c1 = src[i + 1], c2 = src[i + 2], c3 = src[i + 3]; const n0 = tbl.indexOf(c0), n1 = tbl.indexOf(c1), n2raw = tbl.indexOf(c2), n3raw = tbl.indexOf(c3); if (n0 < 0 || n1 < 0) return strict ? false : ''; if (c2 === '=' && c3 !== '=') return strict ? false : ''; if (c2 !== '=' && n2raw < 0) return strict ? false : ''; if (c3 !== '=' && n3raw < 0) return strict ? false : ''; const n2 = c2 === '=' ? 0 : n2raw; const n3 = c3 === '=' ? 0 : n3raw; const n = (n0 << 18) | (n1 << 12) | (n2 << 6) | n3; out += String.fromCharCode((n >> 16) & 0xff); if (c2 !== '=') out += String.fromCharCode((n >> 8) & 0xff); if (c3 !== '=') out += String.fromCharCode(n & 0xff); } return out; }\n");
    }
    if emit_sha256_hex {
        out.push_str("function __phpx_utf8_bytes(input) { const src = String(input ?? ''); if (typeof TextEncoder !== 'undefined') return Array.from(new TextEncoder().encode(src)); const out = []; for (let i = 0; i < src.length; i += 1) { let c = src.charCodeAt(i); if (c >= 0xd800 && c <= 0xdbff && i + 1 < src.length) { const d = src.charCodeAt(i + 1); if (d >= 0xdc00 && d <= 0xdfff) { const cp = 0x10000 + (((c - 0xd800) << 10) | (d - 0xdc00)); out.push(0xf0 | (cp >> 18), 0x80 | ((cp >> 12) & 0x3f), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f)); i += 1; continue; } } if (c >= 0xd800 && c <= 0xdfff) c = 0xfffd; if (c < 0x80) out.push(c); else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 0x3f)); else out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f)); } return out; }\n");
        out.push_str("function __phpx_sha256_hex(input, rawBytes = false) { const K = [1116352408,1899447441,3049323471,3921009573,961987163,1508970993,2453635748,2870763221,3624381080,310598401,607225278,1426881987,1925078388,2162078206,2614888103,3248222580,3835390401,4022224774,264347078,604807628,770255983,1249150122,1555081692,1996064986,2554220882,2821834349,2952996808,3210313671,3336571891,3584528711,113926993,338241895,666307205,773529912,1294757372,1396182291,1695183700,1986661051,2177026350,2456956037,2730485921,2820302411,3259730800,3345764771,3516065817,3600352804,4094571909,275423344,430227734,506948616,659060556,883997877,958139571,1322822218,1537002063,1747873779,1955562222,2024104815,2227730452,2361852424,2428436474,2756734187,3204031479,3329325298]; const src = String(input ?? ''); const bytes = rawBytes ? [] : __phpx_utf8_bytes(src); if (rawBytes) { for (let i = 0; i < src.length; i += 1) bytes.push(src.charCodeAt(i) & 0xff); } const bitLen = bytes.length * 8; bytes.push(0x80); while ((bytes.length % 64) !== 56) bytes.push(0); const hi = Math.floor(bitLen / 0x100000000); const lo = (bitLen >>> 0) & 0xffffffff; for (let i = 3; i >= 0; i -= 1) bytes.push((hi >>> (i * 8)) & 0xff); for (let i = 3; i >= 0; i -= 1) bytes.push((lo >>> (i * 8)) & 0xff); let h0 = 0x6a09e667, h1 = 0xbb67ae85, h2 = 0x3c6ef372, h3 = 0xa54ff53a, h4 = 0x510e527f, h5 = 0x9b05688c, h6 = 0x1f83d9ab, h7 = 0x5be0cd19; const rotr = (x, n) => ((x >>> n) | (x << (32 - n))) >>> 0; for (let i = 0; i < bytes.length; i += 64) { const w = new Array(64); for (let j = 0; j < 16; j += 1) { const k = i + (j * 4); w[j] = (((bytes[k] << 24) | (bytes[k + 1] << 16) | (bytes[k + 2] << 8) | bytes[k + 3]) >>> 0); } for (let j = 16; j < 64; j += 1) { const s0 = (rotr(w[j - 15], 7) ^ rotr(w[j - 15], 18) ^ (w[j - 15] >>> 3)) >>> 0; const s1 = (rotr(w[j - 2], 17) ^ rotr(w[j - 2], 19) ^ (w[j - 2] >>> 10)) >>> 0; w[j] = (((w[j - 16] + s0) >>> 0) + ((w[j - 7] + s1) >>> 0)) >>> 0; } let a = h0, b = h1, c = h2, d = h3, e = h4, f = h5, g = h6, h = h7; for (let j = 0; j < 64; j += 1) { const S1 = (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) >>> 0; const ch = ((e & f) ^ ((~e) & g)) >>> 0; const t1 = (((((h + S1) >>> 0) + ch) >>> 0) + ((K[j] + w[j]) >>> 0)) >>> 0; const S0 = (rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) >>> 0; const maj = ((a & b) ^ (a & c) ^ (b & c)) >>> 0; const t2 = (S0 + maj) >>> 0; h = g; g = f; f = e; e = (d + t1) >>> 0; d = c; c = b; b = a; a = (t1 + t2) >>> 0; } h0 = (h0 + a) >>> 0; h1 = (h1 + b) >>> 0; h2 = (h2 + c) >>> 0; h3 = (h3 + d) >>> 0; h4 = (h4 + e) >>> 0; h5 = (h5 + f) >>> 0; h6 = (h6 + g) >>> 0; h7 = (h7 + h) >>> 0; } const words = [h0, h1, h2, h3, h4, h5, h6, h7]; let out = ''; for (const w of words) { out += (w >>> 0).toString(16).padStart(8, '0'); } return out; }\n");
    }
    if emit_hex_to_binary {
        out.push_str("function __phpx_hex_to_binary(hex) { const src = String(hex ?? ''); let out = ''; for (let i = 0; i < src.length; i += 2) out += String.fromCharCode(parseInt(src.slice(i, i + 2), 16) & 0xff); return out; }\n");
    }
    if emit_hmac_sha256_hex {
        out.push_str("function __phpx_hmac_sha256_hex(data, key) { const rawBytes = (s) => { const out = []; const src = String(s ?? ''); for (let i = 0; i < src.length; i += 1) out.push(src.charCodeAt(i) & 0xff); return out; }; const fromBytes = (arr) => arr.map((v) => String.fromCharCode(v & 0xff)).join(''); const dataBytes = __phpx_utf8_bytes(data); let k = __phpx_utf8_bytes(key); if (k.length > 64) { const kh = __phpx_sha256_hex(fromBytes(k), true); k = rawBytes(__phpx_hex_to_binary(kh)); } while (k.length < 64) k.push(0); const o = [], i = []; for (let n = 0; n < 64; n += 1) { o.push(k[n] ^ 0x5c); i.push(k[n] ^ 0x36); } const innerHex = __phpx_sha256_hex(fromBytes(i) + fromBytes(dataBytes), true); const outerHex = __phpx_sha256_hex(fromBytes(o) + __phpx_hex_to_binary(innerHex), true); return outerHex; }\n");
    }
    if emit_node_crypto {
        out.push_str("const __phpx_node_crypto = (() => { try { if (typeof require === 'function') { return require('node:crypto'); } } catch (_err) {} try { if (typeof require === 'function') { return require('crypto'); } } catch (_err) {} return null; })();\n");
    }
    if emit_hash {
        out.push_str("function __phpx_hash(algo, data, raw = false) { const name = String(algo || '').toLowerCase(); if (name === 'sha256') { const hex = __phpx_sha256_hex(String(data ?? '')); return raw ? __phpx_hex_to_binary(hex) : hex; } const mod = __phpx_node_crypto; if (!mod || typeof mod.createHash !== 'function') throw new Error('hash() requires crypto support'); const digest = mod.createHash(name).update(String(data ?? ''), 'utf8').digest(raw ? 'latin1' : 'hex'); return digest; }\n");
    }
    if emit_hash_hmac {
        out.push_str("function __phpx_hash_hmac(algo, data, key, raw = false) { const name = String(algo || '').toLowerCase(); if (name === 'sha256') { const hex = __phpx_hmac_sha256_hex(String(data ?? ''), String(key ?? '')); return raw ? __phpx_hex_to_binary(hex) : hex; } const mod = __phpx_node_crypto; if (!mod || typeof mod.createHmac !== 'function') throw new Error('hash_hmac() requires crypto support'); const digest = mod.createHmac(name, String(key ?? '')).update(String(data ?? ''), 'utf8').digest(raw ? 'latin1' : 'hex'); return digest; }\n");
    }
    if emit_date_format {
        out.push_str("function __phpx_date_format(fmt, ts) { const d = ts !== undefined && ts !== null ? new Date(Number(ts) * 1000) : new Date(); const p = (n, w) => String(n).padStart(w || 2, '0'); const days = ['Sunday','Monday','Tuesday','Wednesday','Thursday','Friday','Saturday']; const months = ['January','February','March','April','May','June','July','August','September','October','November','December']; let out = ''; for (let i = 0; i < fmt.length; i++) { const c = fmt[i]; switch(c) { case 'Y': out += d.getFullYear(); break; case 'y': out += String(d.getFullYear()).slice(-2); break; case 'm': out += p(d.getMonth()+1); break; case 'd': out += p(d.getDate()); break; case 'H': out += p(d.getHours()); break; case 'i': out += p(d.getMinutes()); break; case 's': out += p(d.getSeconds()); break; case 'n': out += d.getMonth()+1; break; case 'j': out += d.getDate(); break; case 'G': out += d.getHours(); break; case 'N': out += d.getDay()||7; break; case 'w': out += d.getDay(); break; case 'l': out += days[d.getDay()]; break; case 'D': out += days[d.getDay()].slice(0,3); break; case 'F': out += months[d.getMonth()]; break; case 'M': out += months[d.getMonth()].slice(0,3); break; case 't': out += new Date(d.getFullYear(),d.getMonth()+1,0).getDate(); break; case 'U': out += Math.floor(d.getTime()/1000); break; case 'e': case 'T': out += 'UTC'; break; case 'Z': out += -d.getTimezoneOffset()*60; break; case 'c': out += d.toISOString().replace(/\\.\\d{3}Z$/, '+00:00'); break; case 'r': out += d.toUTCString(); break; case 'L': { const y = d.getFullYear(); out += ((y%4===0&&y%100!==0)||(y%400===0)) ? '1' : '0'; break; } default: out += c; } } return out; }\n");
    }
    if emit_date {
        out.push_str(
            "function __phpx_date(fmt, ts) { return __phpx_date_format(String(fmt ?? ''), ts); }\n",
        );
    }
    if emit_gmdate {
        out.push_str("function __phpx_gmdate(fmt, ts) { const d = ts !== undefined && ts !== null ? new Date(Number(ts) * 1000) : new Date(); return __phpx_date_format(String(fmt ?? ''), Math.floor(d.getTime()/1000)); }\n");
    }
    if emit_pack {
        out.push_str("function __phpx_pack(format, ...values) { const fmt = String(format ?? ''); let out = ''; let vi = 0; for (let i = 0; i < fmt.length; i++) { const c = fmt[i]; if (c === 'H') { const hex = String(values[vi++] ?? ''); for (let j = 0; j < hex.length; j += 2) out += String.fromCharCode(parseInt(hex.slice(j, j+2), 16)); } else if (c === 'N') { const n = Number(values[vi++] ?? 0) >>> 0; out += String.fromCharCode((n>>24)&0xff,(n>>16)&0xff,(n>>8)&0xff,n&0xff); } else if (c === 'n') { const n = Number(values[vi++] ?? 0) & 0xffff; out += String.fromCharCode((n>>8)&0xff,n&0xff); } else if (c === 'C') { out += String.fromCharCode(Number(values[vi++] ?? 0) & 0xff); } } return out; }\n");
    }
    out.push('\n');
    out
}

fn add_or_merge_import(imports: &mut Vec<ImportDecl>, from: &str, specs: Vec<ImportSpec>) {
    if let Some(existing) = imports.iter_mut().find(|decl| decl.from == from) {
        for spec in specs {
            if !existing
                .specs
                .iter()
                .any(|item| item.imported == spec.imported && item.local == spec.local)
            {
                existing.specs.push(spec);
            }
        }
        return;
    }
    imports.push(ImportDecl {
        from: from.to_string(),
        specs,
    });
}

fn extract_deka_i_imports(imports: &mut Vec<ImportDecl>) -> Vec<String> {
    let mut locals = Vec::new();
    let mut kept = Vec::new();
    for decl in imports.drain(..) {
        if decl.from == "deka/i" {
            for spec in decl.specs {
                if !locals.contains(&spec.local) {
                    locals.push(spec.local);
                }
            }
        } else {
            kept.push(decl);
        }
    }
    *imports = kept;
    locals
}

fn emit_deka_i_runtime() -> String {
    include_str!("../deka_i_runtime.js").to_string()
}

fn json_string(input: &str) -> String {
    serde_json::to_string(input).unwrap_or_else(|_| "\"\"".to_string())
}

/// Decode a PHPX string-literal key as it appears in object literal syntax.
///
/// The lexer hands us the full source span for string-literal keys, which
/// includes the surrounding `'` or `"` delimiters. To produce the correct
/// JS object key we need to strip those delimiters and interpret the common
/// escape sequences. Mirrors `parse_string_key` / `unescape_string_key`
/// in `crates/php-rs/src/phpx/typeck/check.rs`.
// Keep in sync with `decode_string_key` in
// runtime/crates/phpx_lsp/src/lib.rs and `parse_string_key` in
// runtime/crates/php-rs/src/phpx/typeck/check.rs. All three strip the matching
// quote pair and decode the same escape set on ObjectKey::String tokens.
fn decode_string_key(raw: &str) -> String {
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            let inner = &raw[1..raw.len() - 1];
            return unescape_string_key(inner, first == b'"');
        }
    }
    raw.to_string()
}

fn unescape_string_key(value: &str, double_quoted: bool) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        match next {
            '\'' if !double_quoted => out.push('\''),
            '"' if double_quoted => out.push('"'),
            '\\' => out.push('\\'),
            'n' if double_quoted => out.push('\n'),
            'r' if double_quoted => out.push('\r'),
            't' if double_quoted => out.push('\t'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

fn is_js_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "implements"
            | "import"
            | "in"
            | "instanceof"
            | "interface"
            | "let"
            | "new"
            | "null"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "static"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
    )
}
