//! Production React builtins for `@js/react*` (rfd#64 amendment 2).
//!
//! The resolver serves these specifiers in run/serve/bundle with no install
//! and no `deka.json` entry. Development React stays feature-gated under
//! `crates/http/vendor/react/`.

use deno_core::ModuleSpecifier;
use std::collections::BTreeSet;
use std::sync::OnceLock;

pub const REACT_VERSION: &str = trim_version(include_str!("../vendor/react-prod/VERSION"));

const fn trim_version(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    let mut end = bytes.len();
    while end > 0 {
        match bytes[end - 1] {
            b' ' | b'\n' | b'\r' | b'\t' => end -= 1,
            _ => break,
        }
    }
    let mut start = 0;
    while start < end {
        match bytes[start] {
            b' ' | b'\n' | b'\r' | b'\t' => start += 1,
            _ => break,
        }
    }
    let trimmed = raw.as_bytes().split_at(end).0.split_at(start).1;
    match core::str::from_utf8(trimmed) {
        Ok(text) => text,
        Err(_) => raw,
    }
}

pub const URL_PREFIX: &str = "deka:///js/";

/// User-facing specifiers the runtime provides. Subpaths are intentional:
/// summoned `@js/<name>` does not support them, which is why these cannot
/// go through `js_modules/`. Single list: `file_for_user_spec` / `is_builtin`
/// and the dsc-externals rewrite all read this.
pub const USER_SPECS: &[&str] = &["@js/react", "@js/react/jsx-runtime", "@js/react-dom/server"];

/// Distinctive development-build strings. Production output must not contain
/// these (the deka#937 feature-gate lesson).
pub const DEV_BYTE_PROBES: &[&str] = &[
    "react.development.js",
    "react-jsx-dev-runtime.development.js",
    "react-dom-client.development.js",
    "injectIntoGlobalHook",
    "jsxDEV(",
    "You are calling ReactDOMClient.createRoot()",
];

#[cfg(test)]
const HASHES: &str = include_str!("../vendor/react-prod/HASHES");
const REACT_CJS: &str = include_str!("../vendor/react-prod/cjs/react.production.js");
const JSX_RUNTIME_CJS: &str =
    include_str!("../vendor/react-prod/cjs/react-jsx-runtime.production.js");
const REACT_DOM_CJS: &str = include_str!("../vendor/react-prod/cjs/react-dom.production.js");
const SERVER_LEGACY_CJS: &str =
    include_str!("../vendor/react-prod/cjs/react-dom-server-legacy.browser.production.js");
const SERVER_BROWSER_CJS: &str =
    include_str!("../vendor/react-prod/cjs/react-dom-server.edge.production.js");

const REACT_SHA: &str = "f1e2323f141be9d9379c612eeabd7f282f052e60116547395780b032a6f0770d";
const JSX_RUNTIME_SHA: &str = "1e46f15002696985e80c61d47aaa30dabb03c954270a690f5f7dfbbacfa9002b";
const REACT_DOM_SHA: &str = "f518694f8588dacc9acf35452bbeb18f98174bc2e3eb1bf1181b7fbb4a0f0295";
const SERVER_LEGACY_SHA: &str = "cf3928705cd3051cb8fb88da14811aee4ac4320e4eb4caa5193996019d7cf008";
const SERVER_BROWSER_SHA: &str = "8b316fa071e1fb0e4eb30ae1783c3a16a5d67ca0736192bedda6b5c03310cb7d";

const REACT_EXPORTS: &[&str] = &[
    "Children",
    "Component",
    "Fragment",
    "Profiler",
    "PureComponent",
    "StrictMode",
    "Suspense",
    "__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE",
    "__COMPILER_RUNTIME",
    "cache",
    "cloneElement",
    "createContext",
    "createElement",
    "createRef",
    "forwardRef",
    "isValidElement",
    "lazy",
    "memo",
    "startTransition",
    "unstable_useCacheRefresh",
    "use",
    "useActionState",
    "useCallback",
    "useContext",
    "useDebugValue",
    "useDeferredValue",
    "useEffect",
    "useId",
    "useImperativeHandle",
    "useInsertionEffect",
    "useLayoutEffect",
    "useMemo",
    "useOptimistic",
    "useReducer",
    "useRef",
    "useState",
    "useSyncExternalStore",
    "useTransition",
    "version",
];
const JSX_RUNTIME_EXPORTS: &[&str] = &["Fragment", "jsx", "jsxs"];
const REACT_DOM_EXPORTS: &[&str] = &[
    "__DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE",
    "createPortal",
    "flushSync",
    "preconnect",
    "prefetchDNS",
    "preinit",
    "preinitModule",
    "preload",
    "preloadModule",
    "requestFormReset",
    "unstable_batchedUpdates",
    "useFormState",
    "useFormStatus",
    "version",
];
const SERVER_LEGACY_EXPORTS: &[&str] = &["renderToStaticMarkup", "renderToString", "version"];
const SERVER_BROWSER_EXPORTS: &[&str] = &["prerender", "renderToReadableStream", "version"];

pub fn is_builtin(spec: &str) -> bool {
    file_for_user_spec(spec.trim()).is_some()
}

pub fn filter_imports(imports: &[String]) -> Vec<String> {
    imports
        .iter()
        .filter(|spec| !is_builtin(spec.trim()))
        .cloned()
        .collect()
}

pub fn file_for_user_spec(spec: &str) -> Option<&'static str> {
    match spec.trim() {
        "@js/react" => Some("react.js"),
        "@js/react/jsx-runtime" => Some("jsx-runtime.js"),
        "@js/react-dom/server" => Some("react-dom-server.js"),
        _ => None,
    }
}

pub fn url_for_file(file: &str) -> Option<ModuleSpecifier> {
    ModuleSpecifier::parse(&format!("{URL_PREFIX}{file}")).ok()
}

pub fn resolve_specifier(spec: &str) -> Option<ModuleSpecifier> {
    if let Some(file) = file_for_user_spec(spec) {
        return url_for_file(file);
    }
    if is_builtin_url_str(spec) {
        return ModuleSpecifier::parse(spec).ok();
    }
    None
}

pub fn is_builtin_url(specifier: &ModuleSpecifier) -> bool {
    is_builtin_url_str(specifier.as_str())
}

fn is_builtin_url_str(spec: &str) -> bool {
    spec.strip_prefix(URL_PREFIX)
        .is_some_and(|file| esm_for_file(file).is_some())
}

pub fn load_esm(specifier: &ModuleSpecifier) -> Option<&'static str> {
    let file = specifier.as_str().strip_prefix(URL_PREFIX)?;
    esm_for_file(file)
}

pub fn esm_for_file(file: &str) -> Option<&'static str> {
    match file {
        "react.js" => Some(react_esm()),
        "jsx-runtime.js" => Some(jsx_runtime_esm()),
        "react-dom.js" => Some(react_dom_esm()),
        "react-dom-server-legacy.js" => Some(server_legacy_esm()),
        "react-dom-server-browser.js" => Some(server_browser_esm()),
        "react-dom-server.js" => Some(server_esm()),
        _ => None,
    }
}

/// Files to materialize under `dist/server/.ui/` for a set of user specifiers,
/// including internal CJS dependencies.
pub fn reachable_files<'a>(specs: impl IntoIterator<Item = &'a str>) -> Vec<&'static str> {
    let mut needed: BTreeSet<&str> = BTreeSet::new();
    for spec in specs {
        if let Some(file) = file_for_user_spec(spec) {
            add_reachable(&mut needed, file);
        }
    }
    needed.into_iter().collect()
}

fn add_reachable<'a>(needed: &mut BTreeSet<&'a str>, file: &'a str) {
    if !needed.insert(file) {
        return;
    }
    match file {
        "react-dom.js" => add_reachable(needed, "react.js"),
        "react-dom-server-legacy.js" | "react-dom-server-browser.js" => {
            add_reachable(needed, "react.js");
            add_reachable(needed, "react-dom.js");
        }
        "react-dom-server.js" => {
            add_reachable(needed, "react-dom-server-legacy.js");
            add_reachable(needed, "react-dom-server-browser.js");
        }
        _ => {}
    }
}

pub fn source_imports_builtins(js: &str) -> bool {
    deka_modules::ds_imports::paths(js)
        .iter()
        .any(|spec| is_builtin(spec))
}

/// dsc 0.52 has no `--external` flag. Explicit `@js/react*` imports are
/// treated as undeclared summoned packages and fail `dsc transpile` before
/// emit. Compiler-known hooks/JSX re-inject `@js/react` and
/// `@js/react/jsx-runtime`; `@js/react-dom/server` is rewritten to a sibling
/// stub and restored after emit so host inlining still sees the specifier.
pub struct DscTranspileRewrite {
    pub source: String,
    pub stubs: Vec<DscExternalStub>,
}

pub struct DscExternalStub {
    pub spec: String,
    pub filename: String,
    pub body: String,
}

impl DscTranspileRewrite {
    pub fn restore_js(&self, js: &str) -> String {
        let mut out = js.to_string();
        for stub in &self.stubs {
            let js_name = stub_js_filename(&stub.filename);
            let from = format!("./{js_name}");
            out = out.replace(&format!("\"{from}\""), &format!("\"{}\"", stub.spec));
            out = out.replace(&format!("'{from}'"), &format!("'{}'", stub.spec));
        }
        out
    }
}

pub fn rewrite_for_dsc_transpile(source: &str) -> DscTranspileRewrite {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    let mut stubs: Vec<DscExternalStub> = Vec::new();
    while let Some((prefix, decl, spec, names, suffix)) = next_builtin_import(rest) {
        out.push_str(prefix);
        if dsc_reinjects(&spec) {
            rest = suffix;
            continue;
        }
        let filename = stub_ds_filename(&spec);
        let relative = format!("./{filename}");
        out.push_str(&retarget_import_decl(decl, &spec, &relative));
        if !stubs.iter().any(|stub| stub.spec == spec) {
            stubs.push(DscExternalStub {
                body: stub_module_body(&names),
                spec,
                filename,
            });
        }
        rest = suffix;
    }
    out.push_str(rest);
    DscTranspileRewrite { source: out, stubs }
}

fn dsc_reinjects(spec: &str) -> bool {
    spec == "@js/react" || spec == "@js/react/jsx-runtime"
}

fn stub_ds_filename(spec: &str) -> String {
    let name = spec
        .trim_start_matches("@js/")
        .replace('/', "_")
        .replace('-', "_");
    format!("__deka_js_builtin_{name}.ds")
}

fn stub_js_filename(ds_filename: &str) -> String {
    match ds_filename.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.js"),
        None => ds_filename.to_string(),
    }
}

fn retarget_import_decl(decl: &str, spec: &str, relative: &str) -> String {
    decl.replace(&format!("\"{spec}\""), &format!("\"{relative}\""))
        .replace(&format!("'{spec}'"), &format!("'{relative}'"))
}

fn stub_module_body(names: &[ImportedName]) -> String {
    let mut body =
        String::from("// Generated for dsc transpile; runtime builtins are inlined after emit.\n");
    let mut emitted = BTreeSet::new();
    for name in names {
        if name.imported == "*" || name.imported == "default" {
            for export in ["renderToString", "renderToStaticMarkup", "version"] {
                push_stub_export(&mut body, export, &mut emitted);
            }
            continue;
        }
        push_stub_export(&mut body, &name.imported, &mut emitted);
    }
    if emitted.is_empty() {
        push_stub_export(&mut body, "renderToString", &mut emitted);
    }
    body
}

fn push_stub_export(body: &mut String, name: &str, emitted: &mut BTreeSet<String>) {
    if !emitted.insert(name.to_string()) {
        return;
    }
    if name == "version" {
        body.push_str("export const version: string = \"\"\n");
        return;
    }
    body.push_str("export fn ");
    body.push_str(name);
    body.push_str("(node: ReactNode) string {\n  return \"\"\n}\n");
}

/// Inline `@js/react*` imports into a single module by wrapping the CJS
/// factories. dsc cannot resolve these specifiers (`@js/` subpaths are not
/// summoned packages), so the host owns this step after transpile.
pub fn inline_into(js: &str) -> Result<String, String> {
    let (body, specs) = strip_builtin_imports(js)?;
    if specs.is_empty() {
        return Ok(js.to_string());
    }
    let mut files: BTreeSet<&str> = BTreeSet::new();
    for spec in &specs {
        if let Some(file) = file_for_user_spec(spec) {
            add_reachable(&mut files, file);
        }
    }
    let mut out = String::from(
        "// deka: inlined production React builtins (rfd#64 amendment 2)\n\
         const __deka_js_builtins = Object.create(null);\n\
         function __deka_require_builtin(id) {\n\
           const m = __deka_js_builtins[id];\n\
           if (!m) throw new Error(\"deka react builtin: unexpected require(\" + JSON.stringify(id) + \")\");\n\
           return m;\n\
         }\n",
    );
    for file in [
        "react.js",
        "jsx-runtime.js",
        "react-dom.js",
        "react-dom-server-legacy.js",
        "react-dom-server-browser.js",
        "react-dom-server.js",
    ] {
        if files.contains(file) {
            out.push_str(&factory_assignment(file));
        }
    }
    out.push_str(&body);
    if !contains_dev_bytes(&out) {
        return Ok(out);
    }
    Err("inlined React builtins contained development-build bytes".to_string())
}

fn contains_dev_bytes(js: &str) -> bool {
    DEV_BYTE_PROBES.iter().any(|probe| js.contains(probe))
}

fn factory_assignment(file: &str) -> String {
    let key = registry_key(file);
    if file == "react-dom-server.js" {
        return format!(
            "__deka_js_builtins[{key:?}] = {{\n\
               version: __deka_js_builtins[\"react-dom-server-legacy\"].version,\n\
               renderToString: __deka_js_builtins[\"react-dom-server-legacy\"].renderToString,\n\
               renderToStaticMarkup: __deka_js_builtins[\"react-dom-server-legacy\"].renderToStaticMarkup,\n\
               renderToReadableStream: __deka_js_builtins[\"react-dom-server-browser\"].renderToReadableStream,\n\
               prerender: __deka_js_builtins[\"react-dom-server-browser\"].prerender,\n\
             }};\n"
        );
    }
    let (cjs, _sha) = cjs_for_file(file).expect("reachable file has CJS");
    format!(
        "__deka_js_builtins[{key:?}] = (function(require) {{\n\
           const process = {{ env: {{ NODE_ENV: \"production\" }} }};\n\
           const exports = {{}};\n\
           const module = {{ exports }};\n\
           (function (exports, module, require, process) {{\n\
           {cjs}\n\
           }})(exports, module, require, process);\n\
           return module.exports;\n\
         }})(__deka_require_builtin);\n"
    )
}

fn registry_key(file: &str) -> &'static str {
    match file {
        "react.js" => "react",
        "jsx-runtime.js" => "react/jsx-runtime",
        "react-dom.js" => "react-dom",
        "react-dom-server-legacy.js" => "react-dom-server-legacy",
        "react-dom-server-browser.js" => "react-dom-server-browser",
        "react-dom-server.js" => "react-dom/server",
        _ => "unknown",
    }
}

fn cjs_for_file(file: &str) -> Option<(&'static str, &'static str)> {
    match file {
        "react.js" => Some((REACT_CJS, REACT_SHA)),
        "jsx-runtime.js" => Some((JSX_RUNTIME_CJS, JSX_RUNTIME_SHA)),
        "react-dom.js" => Some((REACT_DOM_CJS, REACT_DOM_SHA)),
        "react-dom-server-legacy.js" => Some((SERVER_LEGACY_CJS, SERVER_LEGACY_SHA)),
        "react-dom-server-browser.js" => Some((SERVER_BROWSER_CJS, SERVER_BROWSER_SHA)),
        _ => None,
    }
}

fn strip_builtin_imports(js: &str) -> Result<(String, BTreeSet<String>), String> {
    let mut specs = BTreeSet::new();
    let mut out = String::with_capacity(js.len());
    let mut rest = js;
    while let Some((prefix, decl, spec, names, suffix)) = next_builtin_import(rest) {
        specs.insert(spec.clone());
        out.push_str(prefix);
        let key = spec.trim_start_matches("@js/");
        out.push_str(&bind_imported_names(key, &names));
        rest = suffix;
        let _ = decl;
    }
    out.push_str(rest);
    Ok((out, specs))
}

struct ImportedName {
    imported: String,
    local: String,
}

fn bind_imported_names(key: &str, names: &[ImportedName]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let mut bindings = Vec::new();
    for name in names {
        if name.imported == "*" {
            bindings.push(format!(
                "const {} = __deka_js_builtins[{key:?}];\n",
                name.local
            ));
        } else if name.imported == "default" {
            bindings.push(format!(
                "const {} = __deka_js_builtins[{key:?}];\n",
                name.local
            ));
        } else {
            bindings.push(format!(
                "const {} = __deka_js_builtins[{key:?}].{};\n",
                name.local, name.imported
            ));
        }
    }
    bindings.concat()
}

fn next_builtin_import(source: &str) -> Option<(&str, &str, String, Vec<ImportedName>, &str)> {
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if is_word_at(bytes, i, b"import") || is_word_at(bytes, i, b"export") {
            let start = i;
            let is_export = bytes[i] == b'e';
            i += if is_export { 6 } else { 6 };
            i = skip_ws_and_comments(bytes, i);
            let names_start = i;
            if let Some((names, after_clause)) = take_import_clause(bytes, i) {
                i = skip_ws_and_comments(bytes, after_clause);
                if is_word_at(bytes, i, b"from") {
                    i = skip_ws_and_comments(bytes, i + 4);
                    if let Some((spec, after_spec)) = take_quoted(bytes, i) {
                        if is_builtin(&spec) {
                            let mut end = skip_ws_and_comments(bytes, after_spec);
                            if bytes.get(end) == Some(&b';') {
                                end += 1;
                            }
                            if bytes.get(end) == Some(&b'\n') {
                                end += 1;
                            }
                            let _ = names_start;
                            return Some((
                                &source[..start],
                                &source[start..end],
                                spec,
                                names,
                                &source[end..],
                            ));
                        }
                    }
                }
            }
            i = start + 1;
            continue;
        }
        i += 1;
    }
    None
}

fn take_import_clause(bytes: &[u8], mut i: usize) -> Option<(Vec<ImportedName>, usize)> {
    i = skip_ws_and_comments(bytes, i);
    if bytes.get(i) == Some(&b'"') || bytes.get(i) == Some(&b'\'') {
        return Some((Vec::new(), i));
    }
    if bytes.get(i) == Some(&b'{') {
        let close = bytes[i + 1..].iter().position(|&b| b == b'}')? + i + 1;
        let inner = std::str::from_utf8(&bytes[i + 1..close]).ok()?;
        let mut names = Vec::new();
        for part in inner.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            names.push(parse_as_name(part));
        }
        return Some((names, close + 1));
    }
    if is_word_at(bytes, i, b"from") {
        return Some((Vec::new(), i));
    }
    if bytes.get(i) == Some(&b'*') {
        i = skip_ws_and_comments(bytes, i + 1);
        if is_word_at(bytes, i, b"as") {
            i = skip_ws_and_comments(bytes, i + 2);
            let (local, next) = take_ident(bytes, i)?;
            return Some((
                vec![ImportedName {
                    imported: "*".to_string(),
                    local,
                }],
                next,
            ));
        }
        return None;
    }
    let (local, next) = take_ident(bytes, i)?;
    Some((
        vec![ImportedName {
            imported: "default".to_string(),
            local,
        }],
        next,
    ))
}

fn parse_as_name(part: &str) -> ImportedName {
    if let Some((imported, local)) = part.split_once(" as ") {
        ImportedName {
            imported: imported.trim().to_string(),
            local: local.trim().to_string(),
        }
    } else {
        ImportedName {
            imported: part.to_string(),
            local: part.to_string(),
        }
    }
}

fn take_quoted(bytes: &[u8], i: usize) -> Option<(String, usize)> {
    let quote = *bytes.get(i)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut j = i + 1;
    while j < bytes.len() && bytes[j] != quote {
        if bytes[j] == b'\\' {
            j += 2;
            continue;
        }
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let spec = std::str::from_utf8(&bytes[i + 1..j]).ok()?.to_string();
    Some((spec, j + 1))
}

fn take_ident(bytes: &[u8], i: usize) -> Option<(String, usize)> {
    let mut j = i;
    while j < bytes.len()
        && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'$')
    {
        j += 1;
    }
    if j == i {
        return None;
    }
    Some((std::str::from_utf8(&bytes[i..j]).ok()?.to_string(), j))
}

fn is_word_at(bytes: &[u8], i: usize, word: &[u8]) -> bool {
    let end = i + word.len();
    if end > bytes.len() || &bytes[i..end] != word {
        return false;
    }
    let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
    let after_ok = end == bytes.len() || !is_ident_byte(bytes[end]);
    before_ok && after_ok
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn skip_ws_and_comments(bytes: &[u8], mut i: usize) -> usize {
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = i.saturating_add(2);
            continue;
        }
        break;
    }
    i
}

fn react_esm() -> &'static str {
    static CELL: OnceLock<String> = OnceLock::new();
    CELL.get_or_init(|| {
        wrap_cjs(
            &format!("https://unpkg.com/react@{REACT_VERSION}/cjs/react.production.js"),
            REACT_SHA,
            REACT_CJS,
            "",
            "",
            REACT_EXPORTS,
        )
    })
    .as_str()
}

fn jsx_runtime_esm() -> &'static str {
    static CELL: OnceLock<String> = OnceLock::new();
    CELL.get_or_init(|| {
        wrap_cjs(
            &format!("https://unpkg.com/react@{REACT_VERSION}/cjs/react-jsx-runtime.production.js"),
            JSX_RUNTIME_SHA,
            JSX_RUNTIME_CJS,
            "",
            "",
            JSX_RUNTIME_EXPORTS,
        )
    })
    .as_str()
}

fn react_dom_esm() -> &'static str {
    static CELL: OnceLock<String> = OnceLock::new();
    CELL.get_or_init(|| {
        wrap_cjs(
            &format!("https://unpkg.com/react-dom@{REACT_VERSION}/cjs/react-dom.production.js"),
            REACT_DOM_SHA,
            REACT_DOM_CJS,
            "import * as __req_react from \"./react.js\";\n",
            "  if (id === \"react\") return __req_react.default;\n",
            REACT_DOM_EXPORTS,
        )
    })
    .as_str()
}

fn server_legacy_esm() -> &'static str {
    static CELL: OnceLock<String> = OnceLock::new();
    CELL.get_or_init(|| {
        wrap_cjs(
            &format!(
                "https://unpkg.com/react-dom@{REACT_VERSION}/cjs/react-dom-server-legacy.browser.production.js"
            ),
            SERVER_LEGACY_SHA,
            SERVER_LEGACY_CJS,
            "import * as __req_react from \"./react.js\";\nimport * as __req_react_dom from \"./react-dom.js\";\n",
            "  if (id === \"react\") return __req_react.default;\n  if (id === \"react-dom\") return __req_react_dom.default;\n",
            SERVER_LEGACY_EXPORTS,
        )
    })
    .as_str()
}

fn server_browser_esm() -> &'static str {
    static CELL: OnceLock<String> = OnceLock::new();
    CELL.get_or_init(|| {
        wrap_cjs(
            &format!(
                "https://unpkg.com/react-dom@{REACT_VERSION}/cjs/react-dom-server.edge.production.js"
            ),
            SERVER_BROWSER_SHA,
            SERVER_BROWSER_CJS,
            "import * as __req_react from \"./react.js\";\nimport * as __req_react_dom from \"./react-dom.js\";\n",
            "  if (id === \"react\") return __req_react.default;\n  if (id === \"react-dom\") return __req_react_dom.default;\n",
            SERVER_BROWSER_EXPORTS,
        )
    })
    .as_str()
}

fn server_esm() -> &'static str {
    static CELL: OnceLock<String> = OnceLock::new();
    CELL.get_or_init(|| {
        format!(
            "/**\n * Deka runtime builtin `@js/react-dom/server` (React {REACT_VERSION}, edge production).\n */\n\
             import * as legacy from \"./react-dom-server-legacy.js\";\n\
             import * as stream from \"./react-dom-server-browser.js\";\n\
             export const version = legacy.version;\n\
             export const renderToString = legacy.renderToString;\n\
             export const renderToStaticMarkup = legacy.renderToStaticMarkup;\n\
             export const renderToReadableStream = stream.renderToReadableStream;\n\
             export const prerender = stream.prerender;\n\
             export default {{\n\
               version,\n\
               renderToString,\n\
               renderToStaticMarkup,\n\
               renderToReadableStream,\n\
               prerender,\n\
             }};\n"
        )
    })
    .as_str()
}

fn wrap_cjs(
    upstream: &str,
    sha: &str,
    cjs: &str,
    imports: &str,
    require_arms: &str,
    export_names: &[&str],
) -> String {
    let mut out = String::with_capacity(cjs.len() + 2048);
    out.push_str("/**\n * Deka-vendored ESM wrapper around the upstream CJS production build.\n");
    out.push_str(" * Upstream: ");
    out.push_str(upstream);
    out.push_str("\n * sha256: ");
    out.push_str(sha);
    out.push_str("\n * Pinned React ");
    out.push_str(REACT_VERSION);
    out.push_str(" ships as a runtime builtin (rfd#64 amendment 2).\n */\n");
    out.push_str(imports);
    out.push_str("const process = { env: { NODE_ENV: \"production\" } };\n");
    out.push_str("const exports = {};\nconst module = { exports };\n");
    out.push_str("function require(id) {\n");
    out.push_str(require_arms);
    out.push_str(
        "  throw new Error(\"deka react builtin: unexpected require(\" + JSON.stringify(id) + \")\");\n}\n",
    );
    // Production CJS is not an IIFE (unlike the development builds). Wrap it
    // so `function Component` / `var Fragment` do not collide with ESM exports.
    out.push_str("(function (exports, module, require, process) {\n");
    out.push_str(cjs);
    if !cjs.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("})(exports, module, require, process);\n");
    out.push_str("\nconst __m = module.exports;\nexport default __m;\n");
    for name in export_names {
        out.push_str("export const ");
        out.push_str(name);
        out.push_str(" = __m.");
        out.push_str(name);
        out.push_str(";\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_lock_upstream_cjs() {
        assert!(HASHES.contains(REACT_SHA));
        assert!(HASHES.contains(JSX_RUNTIME_SHA));
        assert!(HASHES.contains(REACT_DOM_SHA));
        assert!(HASHES.contains(SERVER_LEGACY_SHA));
        assert!(HASHES.contains(SERVER_BROWSER_SHA));
        assert!(
            HASHES.contains(&format!("react@{REACT_VERSION}"))
                || HASHES.contains("react.production.js")
        );
    }

    #[test]
    fn public_specs_are_runtime_provided() {
        for spec in USER_SPECS {
            assert!(is_builtin(spec), "{spec}");
            assert!(file_for_user_spec(spec).is_some(), "{spec}");
        }
        assert!(!is_builtin("@js/lodash"));
        assert!(!is_builtin("@js/react/jsx-dev-runtime"));
        assert_eq!(
            filter_imports(&["@js/react".into(), "@js/chosen".into(), "./local.js".into()]),
            vec!["@js/chosen".to_string(), "./local.js".to_string()]
        );
    }

    #[test]
    fn production_wrappers_do_not_contain_dev_bytes() {
        for file in reachable_files(USER_SPECS.iter().copied()) {
            let esm = esm_for_file(file).expect(file);
            assert!(
                !contains_dev_bytes(esm),
                "{file} leaked a development-build probe"
            );
            assert!(
                !esm.contains("NODE_ENV: \"development\""),
                "{file} is not a production wrap"
            );
        }
        let react = esm_for_file("react.js").unwrap();
        assert!(react.contains(REACT_VERSION));
        assert!(react.contains("useState"));
        let jsx = esm_for_file("jsx-runtime.js").unwrap();
        assert!(jsx.contains("exports.jsx"));
        let server = esm_for_file("react-dom-server.js").unwrap();
        assert!(server.contains("renderToString"));
        assert!(server.contains("renderToReadableStream"));
    }

    #[test]
    fn inline_rewrites_named_imports_and_keeps_prod_bytes() {
        let js = r#"
import { useState, createElement as h } from "@js/react";
import { jsx } from "@js/react/jsx-runtime";
import { renderToString } from "@js/react-dom/server";
export function probe() { return renderToString(jsx("div", { children: h("span") })); }
"#;
        let inlined = inline_into(js).expect("inline");
        assert!(!inlined.contains("from \"@js/react"));
        assert!(inlined.contains("__deka_js_builtins[\"react\"]"));
        assert!(inlined.contains("__deka_js_builtins[\"react/jsx-runtime\"]"));
        assert!(inlined.contains("__deka_js_builtins[\"react-dom/server\"]"));
        assert!(inlined.contains("const useState ="));
        assert!(inlined.contains("const h ="));
        assert!(inlined.contains("export function probe()"));
        assert!(!contains_dev_bytes(&inlined));
    }

    #[test]
    fn rewrite_strips_auto_injected_react_import() {
        let src = "import { useState } from \"@js/react\"\n\nexport fn probe() string {\n  const [label, setLabel] = useState(\"ssr\")\n  return label\n}\n";
        let rewrite = rewrite_for_dsc_transpile(src);
        assert!(
            !rewrite.source.contains("@js/react"),
            "explicit @js/react import must be stripped: {}",
            rewrite.source
        );
        assert!(rewrite.source.contains("useState"));
        assert!(rewrite.stubs.is_empty());
    }

    #[test]
    fn rewrite_stubs_react_dom_server_and_restores_the_specifier() {
        let src = "import { renderToString } from \"@js/react-dom/server\"\nexport fn html() string {\n  return renderToString(\"x\")\n}\n";
        let rewrite = rewrite_for_dsc_transpile(src);
        assert!(
            rewrite
                .source
                .contains("./__deka_js_builtin_react_dom_server.ds"),
            "{}",
            rewrite.source
        );
        assert!(!rewrite.source.contains("@js/react-dom/server"));
        assert_eq!(rewrite.stubs.len(), 1);
        assert!(rewrite.stubs[0].body.contains("export fn renderToString"));
        let restored = rewrite.restore_js(
            "import { renderToString } from \"./__deka_js_builtin_react_dom_server.js\";\n",
        );
        assert!(restored.contains("from \"@js/react-dom/server\""));
        assert!(!restored.contains("__deka_js_builtin_"));
    }
}
