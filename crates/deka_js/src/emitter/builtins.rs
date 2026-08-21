use super::*;

impl<'a> JsSubsetEmitter<'a> {
    /// Compile-time rewrite for class (a) PHP-named builtins.
    /// Returns Some(js_string) if the callee matches a rewrite, None otherwise.
    pub(super) fn try_rewrite_builtin(
        &mut self,
        name: &str,
        args: &[php_rs::parser::ast::Arg<'_>],
    ) -> Result<Option<String>, String> {
        // Helper: emit positional args into a Vec<String>.
        let emit_args = |emitter: &mut Self,
                         args: &[php_rs::parser::ast::Arg<'_>]|
         -> Result<Vec<String>, String> {
            let mut out = Vec::with_capacity(args.len());
            for arg in args {
                out.push(emitter.emit_expr(arg.value)?);
            }
            Ok(out)
        };

        match name {
            // count($x) -> direct .length/Object.keys when known, single-eval inline fallback otherwise.
            "count" if args.len() == 1 => {
                let kind = self.infer_expr_kind(args[0].value);
                let a = emit_args(self, args)?;
                Ok(Some(match kind {
                    Some(JsValueKind::Array | JsValueKind::String) => format!("{}.length", a[0]),
                    Some(JsValueKind::Object) => format!("Object.keys({}).length", a[0]),
                    None => format!(
                        "(() => {{ const __v = {}; return (Array.isArray(__v) || typeof __v === \"string\") ? __v.length : (__v && typeof __v === \"object\" ? Object.keys(__v).length : 0); }})()",
                        a[0]
                    ),
                }))
            }
            // strlen($s) -> $s.length
            "strlen" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("{}.length", a[0])))
            }
            // substr($s, $start) -> $s.slice($start)
            // substr($s, $start, $len) -> $s.slice($start, $start + $len)
            "substr" if args.len() >= 2 && args.len() <= 3 => {
                let a = emit_args(self, args)?;
                if args.len() == 2 {
                    Ok(Some(format!("{}.slice({})", a[0], a[1])))
                } else {
                    Ok(Some(format!(
                        "(() => {{ const __s = {}; const __start = {}; const __len = {}; return __s.slice(__start, __start + __len); }})()",
                        a[0], a[1], a[2]
                    )))
                }
            }
            // trim($s) -> String($s).trim()  (no $chars arg only)
            "trim" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("String({}).trim()", a[0])))
            }
            // ltrim($s) -> String($s).trimStart()
            // ltrim($s, $chars) -> inline regex strip from left
            "ltrim" if args.len() >= 1 && args.len() <= 2 => {
                let a = emit_args(self, args)?;
                if args.len() == 1 {
                    Ok(Some(format!("String({}).trimStart()", a[0])))
                } else {
                    Ok(Some(format!(
                        "(() => {{ const __str = String({0}); const __esc = String({1}).replace(/[-[\\]{{}}()*+?.,\\\\^$|#\\s]/g, \"\\\\$&\"); return __str.replace(new RegExp(\"^[\" + __esc + \"]+\"), \"\"); }})()",
                        a[0], a[1]
                    )))
                }
            }
            // rtrim($s) -> String($s).trimEnd()
            // rtrim($s, $chars) -> inline regex strip from right
            "rtrim" if args.len() >= 1 && args.len() <= 2 => {
                let a = emit_args(self, args)?;
                if args.len() == 1 {
                    Ok(Some(format!("String({}).trimEnd()", a[0])))
                } else {
                    Ok(Some(format!(
                        "(() => {{ const __str = String({0}); const __esc = String({1}).replace(/[-[\\]{{}}()*+?.,\\\\^$|#\\s]/g, \"\\\\$&\"); return __str.replace(new RegExp(\"[\" + __esc + \"]+$\"), \"\"); }})()",
                        a[0], a[1]
                    )))
                }
            }
            // strpos($h, $n) -> IIFE returning index or false
            // strpos($h, $n, $offset) -> IIFE with offset
            "strpos" if args.len() >= 2 && args.len() <= 3 => {
                let a = emit_args(self, args)?;
                let offset = if args.len() == 3 {
                    a[2].clone()
                } else {
                    "0".to_string()
                };
                Ok(Some(format!(
                    "(() => {{ const __i = String({}).indexOf(String({}), {}); return __i >= 0 ? __i : false; }})()",
                    a[0], a[1], offset
                )))
            }
            // strrpos($h, $n) -> IIFE returning last index or false
            "strrpos" if args.len() >= 2 && args.len() <= 3 => {
                let a = emit_args(self, args)?;
                if args.len() == 3 {
                    Ok(Some(format!(
                        "(() => {{ const __i = String({}).lastIndexOf(String({}), {}); return __i >= 0 ? __i : false; }})()",
                        a[0], a[1], a[2]
                    )))
                } else {
                    Ok(Some(format!(
                        "(() => {{ const __i = String({}).lastIndexOf(String({})); return __i >= 0 ? __i : false; }})()",
                        a[0], a[1]
                    )))
                }
            }
            // str_starts_with($h, $n) -> String($h).startsWith(String($n))
            "str_starts_with" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "String({}).startsWith(String({}))",
                    a[0], a[1]
                )))
            }
            // str_ends_with($h, $n) -> String($h).endsWith(String($n))
            "str_ends_with" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("String({}).endsWith(String({}))", a[0], a[1])))
            }
            // str_contains($h, $n) -> String($h).includes(String($n))
            "str_contains" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("String({}).includes(String({}))", a[0], a[1])))
            }
            // strtolower($s) -> String($s).toLowerCase()
            "strtolower" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("String({}).toLowerCase()", a[0])))
            }
            // strtoupper($s) -> String($s).toUpperCase()
            "strtoupper" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("String({}).toUpperCase()", a[0])))
            }
            // array_key_exists($k, $a) -> full inline check
            "array_key_exists" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "({1} != null && typeof {1} === \"object\" && Object.prototype.hasOwnProperty.call({1}, {0}))",
                    a[0], a[1]
                )))
            }
            // in_array($needle, $haystack) -> $haystack.includes($needle)
            "in_array" if args.len() >= 2 && args.len() <= 3 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("{}.includes({})", a[1], a[0])))
            }
            // explode($sep, $s) -> String($s).split(String($sep))
            // explode($sep, $s, $limit) -> keep polyfill behavior for limit
            "explode" if args.len() >= 2 && args.len() <= 3 => {
                let a = emit_args(self, args)?;
                if args.len() == 2 {
                    Ok(Some(format!("String({}).split(String({}))", a[1], a[0])))
                } else {
                    // With limit: emit IIFE matching PHP explode limit behavior
                    Ok(Some(format!(
                        "(() => {{ const __parts = String({1}).split(String({0})); const __n = {2}; if (__n <= 0 || !Number.isInteger(__n)) return __parts; if (__parts.length <= __n) return __parts; const __h = __parts.slice(0, __n - 1); __h.push(__parts.slice(__n - 1).join(String({0}))); return __h; }})()",
                        a[0], a[1], a[2]
                    )))
                }
            }
            // implode($glue, $pieces) -> $pieces.join(String($glue))
            // implode($pieces) -> $pieces.join('')
            "implode" if args.len() >= 1 && args.len() <= 2 => {
                let a = emit_args(self, args)?;
                if args.len() == 1 {
                    Ok(Some(format!(
                        "(Array.isArray({0}) ? {0} : []).join(\"\")",
                        a[0]
                    )))
                } else {
                    Ok(Some(format!(
                        "(Array.isArray({1}) ? {1} : []).join(String({0}))",
                        a[0], a[1]
                    )))
                }
            }
            // chr($n) -> String.fromCharCode(($n) & 0xff)
            "chr" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("String.fromCharCode(({}) & 0xff)", a[0])))
            }
            // ord($s) -> (String($s).length ? String($s).charCodeAt(0) : 0)
            "ord" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __s = String({}); return __s.length ? __s.charCodeAt(0) : 0; }})()",
                    a[0]
                )))
            }
            // time() -> Math.floor(Date.now() / 1000)
            "time" if args.is_empty() => Ok(Some("Math.floor(Date.now() / 1000)".to_string())),
            // array_keys($a) -> Object.keys($a)
            "array_keys" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("Object.keys({})", a[0])))
            }
            // array_values($a) -> Object.values($a)
            "array_values" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("Object.values({})", a[0])))
            }
            // array_map($fn, $a) -> $a.map($fn)
            "array_map" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("{}.map({})", a[1], a[0])))
            }
            // array_filter($a) -> $a.filter(Boolean)
            // array_filter($a, $fn) -> $a.filter($fn)
            "array_filter" if args.len() >= 1 && args.len() <= 2 => {
                let a = emit_args(self, args)?;
                let callback = if args.len() == 2 {
                    a[1].clone()
                } else {
                    "Boolean".to_string()
                };
                Ok(Some(format!("{}.filter({})", a[0], callback)))
            }
            // is_array($x) -> Array.isArray($x). Structs are emitted as plain objects,
            // so they are excluded by the native JS array check.
            "is_array" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("Array.isArray({})", a[0])))
            }
            // is_int($x) -> IIFE to bind arg once: typeof __v === "number" && Number.isInteger(__v)
            "is_int" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __v = {}; return typeof __v === \"number\" && Number.isInteger(__v); }})()",
                    a[0]
                )))
            }
            // is_float($x) -> IIFE: typeof __v === "number" && !Number.isInteger(__v)
            "is_float" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __v = {}; return typeof __v === \"number\" && !Number.isInteger(__v); }})()",
                    a[0]
                )))
            }
            // is_numeric($x) -> IIFE: __v !== '' && !isNaN(Number(__v))
            "is_numeric" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __v = {}; return __v !== \"\" && !isNaN(Number(__v)); }})()",
                    a[0]
                )))
            }
            // is_string($x) -> IIFE: typeof __v === "string"
            "is_string" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __v = {}; return typeof __v === \"string\"; }})()",
                    a[0]
                )))
            }
            // is_object($x) -> IIFE: __v != null && typeof __v === "object" && !Array.isArray(__v)
            "is_object" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __v = {}; return __v != null && typeof __v === \"object\" && !Array.isArray(__v); }})()",
                    a[0]
                )))
            }
            // htmlspecialchars($s) -> inline .replace() chain
            // Optional flags/encoding/double-encode args are accepted but ignored (same as PHP default).
            "htmlspecialchars" if args.len() >= 1 && args.len() <= 4 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "String({}).replace(/&/g, \"&amp;\").replace(/</g, \"&lt;\").replace(/>/g, \"&gt;\").replace(/\"/g, \"&quot;\").replace(/'/g, \"&#039;\")",
                    a[0]
                )))
            }
            // max($a, $b, ...) -> Math.max($a, $b, ...)
            // max($arr)        -> IIFE: Math.max(...$arr) if array, else Math.max($arr)
            "max" if !args.is_empty() => {
                let a = emit_args(self, args)?;
                if args.len() == 1 {
                    Ok(Some(format!(
                        "(() => {{ const __v = {}; return Array.isArray(__v) ? Math.max(...__v) : Math.max(__v); }})()",
                        a[0]
                    )))
                } else {
                    Ok(Some(format!("Math.max({})", a.join(", "))))
                }
            }
            // min($a, $b, ...) -> Math.min($a, $b, ...)
            // min($arr)        -> IIFE: Math.min(...$arr) if array, else Math.min($arr)
            "min" if !args.is_empty() => {
                let a = emit_args(self, args)?;
                if args.len() == 1 {
                    Ok(Some(format!(
                        "(() => {{ const __v = {}; return Array.isArray(__v) ? Math.min(...__v) : Math.min(__v); }})()",
                        a[0]
                    )))
                } else {
                    Ok(Some(format!("Math.min({})", a.join(", "))))
                }
            }
            // str_replace($search, $replace, $subject)
            // Scalar search: String($subject).split(String($search)).join(String($replace))
            // Array search: IIFE loop (handles array form without a runtime helper).
            "str_replace" if args.len() == 3 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __srch = {0}; const __repl = {1}; let __s = String({2}); if (Array.isArray(__srch)) {{ for (let __i = 0; __i < __srch.length; __i++) {{ const __r = Array.isArray(__repl) ? String(__repl[__i] ?? \"\") : String(__repl); __s = __s.split(String(__srch[__i])).join(__r); }} return __s; }} return __s.split(String(__srch)).join(String(__repl)); }})()",
                    a[0], a[1], a[2]
                )))
            }
            // rawurlencode($s) -> encodeURIComponent(String($s)) + RFC 3986 replacements for !'/*()'
            "rawurlencode" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "encodeURIComponent(String({})).replace(/!/g, \"%21\").replace(/'/g, \"%27\").replace(/\\(/g, \"%28\").replace(/\\)/g, \"%29\").replace(/\\*/g, \"%2A\")",
                    a[0]
                )))
            }
            // dechex($n) -> (Number($n)>>>0).toString(16)
            "dechex" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("(Number({})>>>0).toString(16)", a[0])))
            }
            // hexdec($s) -> (parseInt(String($s), 16) || 0)
            "hexdec" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("(parseInt(String({}), 16) || 0)", a[0])))
            }
            // urlencode($s) -> PHP form-encoding: space → '+', rest via encodeURIComponent
            // PHP urlencode replaces space with '+' (not %20) and encodes all non-alphanumeric
            // except '-', '_', '.', '~'.  encodeURIComponent leaves '-', '_', '.', '!' — we
            // then replace '!' with '%21', restore '+' convention for space, and encode '*','(',')','~'.
            "urlencode" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "encodeURIComponent(String({})).replace(/%20/g, '+').replace(/[!'()*~]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase())",
                    a[0]
                )))
            }
            // urldecode($s) -> reverse of urlencode: '+' → space, then decodeURIComponent.
            // Wrapped in IIFE with try/catch so malformed sequences (%G0, %2, %E0) return
            // the raw post-substitution string instead of throwing URIError — matching PHP
            // pass-through semantics for bad percent-sequences (DoS safety for user input).
            "urldecode" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __s = String({}).replace(/\\+/g, '%20'); try {{ return decodeURIComponent(__s); }} catch(_) {{ return __s; }} }})()",
                    a[0]
                )))
            }
            // gettype($v) -> inline type-string mapping matching PHP return values
            "gettype" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __v = {}; if (typeof __v === 'boolean') return 'boolean'; if (typeof __v === 'number') return Number.isInteger(__v) ? 'integer' : 'double'; if (typeof __v === 'string') return 'string'; if (Array.isArray(__v)) return 'array'; if (__v !== null && typeof __v === 'object') return 'object'; return 'unknown type'; }})()",
                    a[0]
                )))
            }
            // get_object_vars($v) -> own enumerable keys excluding __struct tag
            "get_object_vars" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __v = {}; if (!__v || typeof __v !== 'object') return {{}}; const __o = {{}}; for (const __k of Object.keys(__v)) {{ if (__k !== '__struct') __o[__k] = __v[__k]; }} return __o; }})()",
                    a[0]
                )))
            }
            // mt_rand($min, $max) -> Math.floor(Math.random() * (max - min + 1)) + min
            "mt_rand" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __lo = Number({}); const __hi = Number({}); return Math.floor(Math.random() * (__hi - __lo + 1)) + __lo; }})()",
                    a[0], a[1]
                )))
            }
            // mt_rand() with no args -> 0..PHP_INT_MAX (2147483647)
            "mt_rand" if args.is_empty() => {
                Ok(Some("Math.floor(Math.random() * 2147483648)".to_string()))
            }
            // microtime(true) -> float seconds; microtime() -> string "msec sec"
            "microtime" if args.len() <= 1 => {
                let a = emit_args(self, args)?;
                if args.is_empty() {
                    Ok(Some("(() => { const __t = Date.now(); const __sec = Math.floor(__t / 1000); const __msec = (__t % 1000) / 1000; return __msec.toFixed(6) + ' ' + __sec; })()".to_string()))
                } else {
                    Ok(Some(format!(
                        "(() => {{ const __t = Date.now(); return {} ? __t / 1000 : (() => {{ const __sec = Math.floor(__t / 1000); const __msec = (__t % 1000) / 1000; return __msec.toFixed(6) + ' ' + __sec; }})(); }})()",
                        a[0]
                    )))
                }
            }
            // strtotime($s) -> parse date string to Unix timestamp integer, or false
            "strtotime" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __s = {}; if (!__s) return false; const __d = new Date(String(__s)); return isNaN(__d.getTime()) ? false : Math.floor(__d.getTime() / 1000); }})()",
                    a[0]
                )))
            }
            // preg_match($pattern, $subject) -> 1 if matched, 0 if not
            // preg_match($pattern, $subject, &$matches) -> also populates matches (omitted for now — 3-arg form falls through)
            "preg_match" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __src = String({}); const __lastSlash = __src.lastIndexOf('/'); const __pat = __lastSlash > 0 ? __src.slice(1, __lastSlash) : __src.slice(1); const __flags = (__lastSlash > 0 ? __src.slice(__lastSlash + 1) : '').replace(/[^gimsuy]/g, ''); try {{ return new RegExp(__pat, __flags).test(String({})) ? 1 : 0; }} catch(__e) {{ return 0; }} }})()",
                    a[0], a[1]
                )))
            }
            // preg_replace($pattern, $replacement, $subject) -> String with global replace
            "preg_replace" if args.len() == 3 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __src = String({}); const __lastSlash = __src.lastIndexOf('/'); const __pat = __lastSlash > 0 ? __src.slice(1, __lastSlash) : __src.slice(1); const __flags = (__lastSlash > 0 ? __src.slice(__lastSlash + 1) : '') + 'g'; try {{ return String({}).replace(new RegExp(__pat, __flags.replace(/g+/g, 'g')), String({})); }} catch(__e) {{ return String({}); }} }})()",
                    a[0], a[2], a[1], a[2]
                )))
            }
            // parse_url($url) -> map of components; parse_url($url, $component) -> single component
            // Uses URL constructor with a synthetic base so relative paths work.
            // The synthetic base is stripped from results: scheme/host are omitted when the
            // input had none (path-only URLs).
            "parse_url" if args.len() >= 1 && args.len() <= 2 => {
                let a = emit_args(self, args)?;
                if args.len() == 1 {
                    Ok(Some(format!(
                        "(() => {{ const __raw = String({}); try {{ const __hasScheme = /^[a-zA-Z][a-zA-Z0-9+\\-.]*:\\/\\//.test(__raw); const __u = new URL(__raw, 'http://x'); const __map = {{}}; if (__hasScheme) {{ __map['scheme'] = __u.protocol.replace(':',''); __map['host'] = __u.hostname || undefined; if (__u.port) __map['port'] = parseInt(__u.port); if (__u.username) __map['user'] = __u.username; if (__u.password) __map['pass'] = __u.password; }} __map['path'] = __u.pathname; if (__u.search) __map['query'] = __u.search.slice(1); if (__u.hash) __map['fragment'] = __u.hash.slice(1); return __map; }} catch(__e) {{ return false; }} }})()",
                        a[0]
                    )))
                } else {
                    Ok(Some(format!(
                        "(() => {{ const __raw = String({}); const __comp = {}; try {{ const __hasScheme = /^[a-zA-Z][a-zA-Z0-9+\\-.]*:\\/\\//.test(__raw); const __u = new URL(__raw, 'http://x'); const __map = {{}}; if (__hasScheme) {{ __map['scheme'] = __u.protocol.replace(':',''); __map['host'] = __u.hostname || undefined; if (__u.port) __map['port'] = parseInt(__u.port); if (__u.username) __map['user'] = __u.username; if (__u.password) __map['pass'] = __u.password; }} __map['path'] = __u.pathname; if (__u.search) __map['query'] = __u.search.slice(1); if (__u.hash) __map['fragment'] = __u.hash.slice(1); const __names = ['scheme','host','path','port','user','pass','query','fragment']; return __map[__names[__comp]] ?? false; }} catch(__e) {{ return false; }} }})()",
                        a[0], a[1]
                    )))
                }
            }
            // intval($v) / intval($v, $base) -> integer coercion
            "intval" if args.len() >= 1 && args.len() <= 2 => {
                let a = emit_args(self, args)?;
                if args.len() == 1 {
                    Ok(Some(format!("(parseInt(String({}), 10) || 0)", a[0])))
                } else {
                    Ok(Some(format!(
                        "(parseInt(String({}), Number({}) || 10) || 0)",
                        a[0], a[1]
                    )))
                }
            }
            // floatval($v) -> float coercion
            "floatval" | "doubleval" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("(parseFloat(String({})) || 0)", a[0])))
            }
            // boolval($v) -> boolean coercion
            "boolval" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("Boolean({})", a[0])))
            }
            // strval($v) -> string coercion
            "strval" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("String({})", a[0])))
            }
            // array_slice($arr, $offset) / array_slice($arr, $offset, $length) /
            // array_slice($arr, $offset, $length, $preserve_keys)
            // Arrays: .slice(). Objects: iterate keys. preserve_keys only matters for objects.
            "array_slice" if args.len() >= 2 && args.len() <= 4 => {
                let a = emit_args(self, args)?;
                // PHP arg order: array_slice($arr, $offset, $length, $preserve_keys)
                let arr = a[0].clone();
                let off = a[1].clone();
                let len_expr = if args.len() >= 3 {
                    Some(a[2].clone())
                } else {
                    None
                };
                let preserve = if args.len() == 4 {
                    Some(a[3].clone())
                } else {
                    None
                };
                match (len_expr, preserve.clone()) {
                    (None, _) => Ok(Some(format!(
                        "(() => {{ const __a = {}; const __off = Number({}); if (Array.isArray(__a)) return __a.slice(__off); const __ks = Object.keys(__a); const __sl = __ks.slice(__off); const __o = {{}}; for (const __k of __sl) __o[__k] = __a[__k]; return __o; }})()",
                        arr, off
                    ))),
                    (Some(len), None) | (Some(len), Some(_)) => {
                        let pk = preserve.unwrap_or_else(|| "false".to_string());
                        Ok(Some(format!(
                            "(() => {{ const __a = {}; const __off = Number({}); const __len = {}; const __pk = !!{}; if (Array.isArray(__a)) return __a.slice(__off, __len != null ? __off + Number(__len) : undefined); const __ks = Object.keys(__a); const __sl = __len != null ? __ks.slice(__off, __off + Number(__len)) : __ks.slice(__off); if (__pk) {{ const __o = {{}}; for (const __k of __sl) __o[__k] = __a[__k]; return __o; }} return __sl.map(__k => __a[__k]); }})()",
                            arr, off, len, pk
                        )))
                    }
                }
            }
            // base64_encode($s) -> __phpx_base64_encode($s)  (Tier B helper, module-scoped)
            "base64_encode" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                self.needed_helpers.insert("base64_encode");
                Ok(Some(format!("__phpx_base64_encode({})", a[0])))
            }
            // base64_decode($s) / base64_decode($s, $strict) -> __phpx_base64_decode(...)
            "base64_decode" if args.len() >= 1 && args.len() <= 2 => {
                let a = emit_args(self, args)?;
                self.needed_helpers.insert("base64_decode");
                if args.len() == 1 {
                    Ok(Some(format!("__phpx_base64_decode({})", a[0])))
                } else {
                    Ok(Some(format!("__phpx_base64_decode({}, {})", a[0], a[1])))
                }
            }
            // hash($algo, $data) / hash($algo, $data, $raw) -> __phpx_hash(...)
            "hash" if args.len() >= 2 && args.len() <= 3 => {
                let a = emit_args(self, args)?;
                self.needed_helpers.insert("hash");
                if args.len() == 2 {
                    Ok(Some(format!("__phpx_hash({}, {})", a[0], a[1])))
                } else {
                    Ok(Some(format!("__phpx_hash({}, {}, {})", a[0], a[1], a[2])))
                }
            }
            // hash_hmac($algo, $data, $key) / hash_hmac($algo, $data, $key, $raw) -> __phpx_hash_hmac(...)
            "hash_hmac" if args.len() >= 3 && args.len() <= 4 => {
                let a = emit_args(self, args)?;
                self.needed_helpers.insert("hash_hmac");
                if args.len() == 3 {
                    Ok(Some(format!(
                        "__phpx_hash_hmac({}, {}, {})",
                        a[0], a[1], a[2]
                    )))
                } else {
                    Ok(Some(format!(
                        "__phpx_hash_hmac({}, {}, {}, {})",
                        a[0], a[1], a[2], a[3]
                    )))
                }
            }
            // hash_equals($a, $b) -> inline constant-time compare (security-critical, no short-circuit)
            // XOR loop over max(len_a, len_b) chars. Returns false if lengths differ but still
            // consumes the loop so timing does not leak which side is shorter.
            "hash_equals" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __a = String({}); const __b = String({}); if (__a.length !== __b.length) return false; let __r = 0; const __n = __a.length; for (let __i = 0; __i < __n; __i++) __r |= __a.charCodeAt(__i) ^ __b.charCodeAt(__i); return __r === 0; }})()",
                    a[0], a[1]
                )))
            }
            // date($fmt) / date($fmt, $ts) -> __phpx_date(...)
            "date" if args.len() >= 1 && args.len() <= 2 => {
                let a = emit_args(self, args)?;
                self.needed_helpers.insert("date");
                if args.len() == 1 {
                    Ok(Some(format!("__phpx_date({})", a[0])))
                } else {
                    Ok(Some(format!("__phpx_date({}, {})", a[0], a[1])))
                }
            }
            // gmdate($fmt) / gmdate($fmt, $ts) -> __phpx_gmdate(...)
            "gmdate" if args.len() >= 1 && args.len() <= 2 => {
                let a = emit_args(self, args)?;
                self.needed_helpers.insert("gmdate");
                if args.len() == 1 {
                    Ok(Some(format!("__phpx_gmdate({})", a[0])))
                } else {
                    Ok(Some(format!("__phpx_gmdate({}, {})", a[0], a[1])))
                }
            }
            // pack($fmt, ...$values) -> __phpx_pack(...)
            "pack" if !args.is_empty() => {
                let a = emit_args(self, args)?;
                self.needed_helpers.insert("pack");
                Ok(Some(format!("__phpx_pack({})", a.join(", "))))
            }
            // function_exists($name) -> typeof globalThis[String($name)] === 'function'
            // Inline rewrite: no polyfill on globalThis. The dynamic globalThis lookup is
            // the correct semantic — function_exists checks whether a named global function
            // is defined at runtime.
            "function_exists" if args.len() == 1 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(typeof globalThis[String({})] === \"function\")",
                    a[0]
                )))
            }
            // class_exists($name) / class_exists($name, $autoload) -> false
            // PHPX has no classes. Any call to class_exists always returns false.
            "class_exists" if args.len() >= 1 && args.len() <= 2 => {
                // Emit the args to trigger any side-effects the expression might have,
                // then return false.
                let _ = emit_args(self, args)?;
                Ok(Some("false".to_string()))
            }
            // usort($arr, $fn) -> ($arr.sort($fn), true)
            // JS Array.prototype.sort is in-place and mutates the array.
            // The PHPX caller's variable still references the same array object.
            "usort" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!("({}.sort({}), true)", a[0], a[1])))
            }
            // uasort($arr, $fn) -> sort by values while preserving object keys.
            // Arrays use native in-place sort; object-shaped associative arrays are
            // rebuilt in comparator order without changing each entry's key.
            "uasort" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __arr = {0}; const __fn = {1}; if (Array.isArray(__arr)) {{ __arr.sort(__fn); return true; }} const __entries = Object.keys(__arr).map(k => [k, __arr[k]]); __entries.sort((a, b) => __fn(a[1], b[1])); Object.keys(__arr).forEach(k => delete __arr[k]); __entries.forEach(([k, v]) => {{ __arr[k] = v; }}); return true; }})()",
                    a[0], a[1]
                )))
            }
            // uksort($arr, $fn) -> not directly mappable; emit sort by key as best-effort
            "uksort" if args.len() == 2 => {
                let a = emit_args(self, args)?;
                Ok(Some(format!(
                    "(() => {{ const __arr = {0}; const __fn = {1}; const __keys = Object.keys(__arr); __keys.sort(__fn); const __tmp = __keys.reduce((o, k) => {{ o[k] = __arr[k]; return o; }}, {{}}); Object.keys(__arr).forEach(k => delete __arr[k]); Object.assign(__arr, __tmp); return true; }})()",
                    a[0], a[1]
                )))
            }
            _ => Ok(None),
        }
    }
}
