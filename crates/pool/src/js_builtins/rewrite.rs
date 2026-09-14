//! dsc stub rewrite and host-owned CJS inlining for `@js/react*` imports.
//!
//! Split out of `js_builtins.rs` (deka#391 file-size gate): dsc cannot
//! resolve `@js/` subpaths, so the host rewrites those imports before
//! transpile and inlines the CJS factories after emit.

use super::specs::{add_reachable, contains_dev_bytes, file_for_user_spec, is_builtin};
use super::vendor::{cjs_for_file, registry_key};
use std::collections::BTreeSet;

/// dsc 0.52 has no `--external` flag. Explicit `@js/react*` imports are
/// treated as undeclared summoned packages and fail `dsc transpile` before
/// emit. Compiler-known hooks re-inject `@js/react`. Other imports use
/// temporary compiler declarations, restored after emit so host inlining
/// sees the real runtime specifier. Explicit JSX helpers must retain their
/// bindings even when the source contains no JSX literals.
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
                body: if spec == "@js/react/jsx-runtime" {
                    jsx_runtime_declarations()
                } else {
                    stub_module_body(&names)
                },
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
    spec == "@js/react"
}

fn jsx_runtime_declarations() -> String {
    // Like the other compiler adapters below, this module supplies types
    // only. restore_js replaces its import before execution; React's real
    // factories supply the implementation. Generic props preserve the
    // caller's object shape without pretending props are ReactNode values.
    let mut body = String::from("export const Fragment: ReactNode = \"\"\n");
    for name in ["jsx", "jsxs"] {
        body.push_str(&format!(
            "export fn {name}<T, P>(tag: T, props: P, key: ReactNode = \"\") ReactNode {{\n  return \"\"\n}}\n"
        ));
    }
    body
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
    if name == "hydrateRoot" || name == "createRoot" {
        body.push_str("export fn ");
        body.push_str(name);
        body.push_str("(container: ReactNode, node: ReactNode) ReactNode {\n  return node\n}\n");
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
        "scheduler.js",
        "react-dom.js",
        "react-dom-client.js",
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
