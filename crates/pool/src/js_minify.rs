//! Restricted SWC minify for vendored production React CJS.
//!
//! React 19's npm `*.production.js` files are dev-stripped but not minified.
//! Islands emit (`inline_into`) and the ESM wrappers both run those bytes
//! through this pass so `deka serve` and `deka build` ship the same payload.
//!
//! Compress is off. The deleted bundler's four SWC 42.x opt-outs
//! (`crates/bundler/src/optimizer.rs`, deka#750 / phpx lineage) plus
//! `collapse_vars = false` still emit invalid JS on React 19.1.1
//! production CJS (`typeof x = expr`, `a || b = c`, `cond ? a, b : c`).
//! Those paths are not all flag-gated. Do not re-enable compress blindly.
//! Mangle + compact codegen is the SWC minify that stays valid.

use std::path::PathBuf;
use std::sync::OnceLock;

use swc_common::sync::Lrc;
use swc_common::{FileName, GLOBALS, Globals, Mark, SourceMap};
use swc_ecma_ast::{EsVersion, Program};
use swc_ecma_codegen::{Emitter, text_writer::JsWriter};
use swc_ecma_minifier::optimize;
use swc_ecma_minifier::option::{ExtraOptions, MangleOptions, MinifyOptions};
use swc_ecma_parser::{EsSyntax, Parser, StringInput, Syntax, lexer::Lexer};

pub fn cached(name: &'static str, source: &'static str) -> &'static str {
    fn init(name: &'static str, source: &'static str) -> String {
        // Isolate worker stacks are too small for SWC on react-dom-client
        // (~500KB AST in debug). Minify on a dedicated thread.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name(format!("deka-minify-{name}"))
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let _ = tx.send(minify_cjs(name, source));
            })
            .unwrap_or_else(|err| panic!("failed to spawn minify thread for {name}: {err}"))
            .join()
            .unwrap_or_else(|_| panic!("minify thread for {name} panicked"));
        rx.recv()
            .unwrap_or_else(|_| panic!("minify thread for {name} dropped the result"))
            .unwrap_or_else(|err| panic!("failed to minify vendored React {name}: {err}"))
    }
    match name {
        "react.production.js" => {
            static CELL: OnceLock<String> = OnceLock::new();
            CELL.get_or_init(|| init(name, source)).as_str()
        }
        "react-jsx-runtime.production.js" => {
            static CELL: OnceLock<String> = OnceLock::new();
            CELL.get_or_init(|| init(name, source)).as_str()
        }
        "react-dom.production.js" => {
            static CELL: OnceLock<String> = OnceLock::new();
            CELL.get_or_init(|| init(name, source)).as_str()
        }
        "react-dom-client.production.js" => {
            static CELL: OnceLock<String> = OnceLock::new();
            CELL.get_or_init(|| init(name, source)).as_str()
        }
        "scheduler.production.js" => {
            static CELL: OnceLock<String> = OnceLock::new();
            CELL.get_or_init(|| init(name, source)).as_str()
        }
        "react-dom-server-legacy.browser.production.js" => {
            static CELL: OnceLock<String> = OnceLock::new();
            CELL.get_or_init(|| init(name, source)).as_str()
        }
        "react-dom-server.edge.production.js" => {
            static CELL: OnceLock<String> = OnceLock::new();
            CELL.get_or_init(|| init(name, source)).as_str()
        }
        other => panic!("unknown vendored React file {other}"),
    }
}

/// Minify a CJS production file. Leading license comments are preserved;
/// the body is mangled and compact-emitted (no compress — see module docs).
pub fn minify_cjs(name: &str, source: &str) -> Result<String, String> {
    let header = leading_comments(source);
    let minified = minify_script(name, source)?;
    if header.is_empty() {
        Ok(minified)
    } else if minified.is_empty() {
        Ok(header)
    } else {
        Ok(format!("{header}\n{minified}"))
    }
}

fn leading_comments(source: &str) -> String {
    match source.find("\"use strict\"") {
        Some(idx) => source[..idx].trim_end().to_string(),
        None => String::new(),
    }
}

fn minify_script(name: &str, source: &str) -> Result<String, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(
        FileName::Real(PathBuf::from(name)).into(),
        source.to_string(),
    );
    let syntax = Syntax::Es(EsSyntax {
        jsx: false,
        export_default_from: false,
        import_attributes: false,
        ..Default::default()
    });
    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let script = parser
        .parse_script()
        .map_err(|err| format!("failed to parse {name} for minify: {err:?}"))?;
    let globals = Globals::new();
    let script = GLOBALS.set(&globals, || {
        let top_level_mark = Mark::new();
        let unresolved_mark = Mark::new();
        let mut program = Program::Script(script);
        program.mutate(swc_ecma_transforms_base::resolver(
            unresolved_mark,
            top_level_mark,
            false,
        ));
        minify_program(program, cm.clone(), unresolved_mark, top_level_mark)
    });
    match script {
        Program::Script(script) => emit_script_compact(&script, cm),
        Program::Module(_) => Err(format!("minifier returned a module for {name}")),
    }
}

fn minify_program(
    program: Program,
    cm: Lrc<SourceMap>,
    unresolved_mark: Mark,
    top_level_mark: Mark,
) -> Program {
    // Compress is off on purpose. The four bundler opt-outs
    // (conditionals/bools, sequences=0, inline=0, if_return=false) plus
    // collapse_vars=false still leave un-gated SWC 42.x miscompiles on
    // React 19.1.1 production CJS:
    //
    //  - compress/optimize/collapse_vars.rs: `typeof x = expr`
    //  - compress/optimize/conditionals.rs `compress_if_stmt_as_cond`:
    //    `if (foo); else bar = baz` => `foo || bar = baz` (not gated)
    //  - compress/optimize/if_return.rs: `cond ? stmt, expr : alt`
    //    still fires with if_return=false
    //
    // Re-enable compress only after those emit valid JS on every file in
    // vendor/react-prod/cjs (see `minified_vendor_parses_in_node`).
    //
    // CJS exports are property assignments (`exports.useState = …`), so
    // top-level mangling is safe: string identities on `exports` survive.
    let minify_options = MinifyOptions {
        compress: None,
        mangle: Some(MangleOptions {
            top_level: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    optimize(
        program,
        cm,
        None,
        None,
        &minify_options,
        &ExtraOptions {
            unresolved_mark,
            top_level_mark,
            mangle_name_cache: None,
        },
    )
}

fn emit_script_compact(
    script: &swc_ecma_ast::Script,
    cm: Lrc<SourceMap>,
) -> Result<String, String> {
    let mut cfg = swc_ecma_codegen::Config::default();
    cfg.minify = true;
    let mut buf = Vec::new();
    {
        let mut emitter = Emitter {
            cfg,
            comments: None,
            cm: cm.clone(),
            wr: JsWriter::new(cm, "", &mut buf, None),
        };
        emitter
            .emit_script(script)
            .map_err(|err| format!("failed to emit minified JavaScript: {err}"))?;
    }
    String::from_utf8(buf).map_err(|err| format!("minified JavaScript was not UTF-8: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minify_preserves_if_assignment() {
        let src = "var x = 0;\nfunction f(c, y) {\n  if (c) x = y;\n  return x;\n}\n";
        let out = minify_cjs("if-assign.js", src).expect("minify");
        assert!(
            !out.contains("&&x=") && !out.contains("&& x ="),
            "conditionals/bools must not emit `c && x = y`:\n{out}"
        );
        assert!(
            out.contains("x=y")
                || out.contains("x = y")
                || out.contains("if(")
                || out.contains("if ("),
            "assignment must survive:\n{out}"
        );
    }

    #[test]
    fn minify_preserves_for_of_head() {
        let src = "var count = 0;\nvar arr = [1, 2];\nfor (var x of arr) count++;\n";
        let out = minify_cjs("for-of.js", src).expect("minify");
        assert!(
            !out.contains("of count") && !out.contains("ofcount"),
            "sequences must not fold into for-of heads:\n{out}"
        );
        assert!(out.contains("of "), "for-of must survive:\n{out}");
    }

    #[test]
    fn minify_preserves_assign_then_typeof() {
        let src = r#"
var MAYBE_ITERATOR_SYMBOL = Symbol.iterator;
function getIteratorFn(maybeIterable) {
  if (null === maybeIterable || "object" !== typeof maybeIterable) return null;
  maybeIterable =
    (MAYBE_ITERATOR_SYMBOL && maybeIterable[MAYBE_ITERATOR_SYMBOL]) ||
    maybeIterable["@@iterator"];
  return "function" === typeof maybeIterable ? maybeIterable : null;
}
exports.getIteratorFn = getIteratorFn;
"#;
        let out = minify_cjs("iterator.js", src).expect("minify");
        assert!(
            !out.contains("typeof maybeIterable=") && !out.contains("typeof e="),
            "collapse_vars must not emit `typeof x = expr`:\n{out}"
        );
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("iterator.js");
        std::fs::write(&path, &out).expect("write");
        let check = std::process::Command::new("node")
            .args(["--check", path.to_str().expect("utf-8")])
            .output()
            .expect("node --check");
        assert!(
            check.status.success(),
            "assign-then-typeof minify failed node --check:\n{}\n{out}",
            String::from_utf8_lossy(&check.stderr)
        );
    }

    #[test]
    fn minify_parens_assignment_in_logical_sequence() {
        let src = r#"
function settle(e, t) {
  if ("pending" === e.status) e.status = "fulfilled", e.value = t;
}
exports.settle = settle;
"#;
        let out = minify_cjs("logical-seq.js", src).expect("minify");
        assert!(
            !out.contains("&&e.status=") && !out.contains("&& e.status ="),
            "assignment after && in a sequence must be parenthesized:\n{out}"
        );
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("logical-seq.js");
        std::fs::write(&path, &out).expect("write");
        let check = std::process::Command::new("node")
            .args(["--check", path.to_str().expect("utf-8")])
            .output()
            .expect("node --check");
        assert!(
            check.status.success(),
            "logical-seq minify failed node --check:\n{}\n{out}",
            String::from_utf8_lossy(&check.stderr)
        );
    }

    #[test]
    fn minify_parens_assignment_in_logical() {
        let src = r#"
function copy(t, n) {
  var s;
  if (null != t)
    for (s in t)
      if (!Object.prototype.hasOwnProperty.call(t, s) || "key" === s);
      else n[s] = t[s];
}
exports.copy = copy;
"#;
        let out = minify_cjs("logical-assign.js", src).expect("minify");
        assert!(
            !out.contains("||n[s]=") && !out.contains("|| n[s] ="),
            "assignment in || must be parenthesized:\n{out}"
        );
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("logical-assign.js");
        std::fs::write(&path, &out).expect("write");
        let check = std::process::Command::new("node")
            .args(["--check", path.to_str().expect("utf-8")])
            .output()
            .expect("node --check");
        assert!(
            check.status.success(),
            "logical-assign minify failed node --check:\n{}\n{out}",
            String::from_utf8_lossy(&check.stderr)
        );
    }

    #[test]
    fn minify_keeps_cjs_export_names() {
        let src = "\"use strict\";\nfunction useState(init) { return [init, function () {}]; }\nexports.useState = useState;\nexports.version = \"19.1.1\";\n";
        let out = minify_cjs("react-stub.js", src).expect("minify");
        assert!(out.contains("exports.useState"), "{out}");
        assert!(out.contains("exports.version"), "{out}");
        assert!(
            out.len() < src.len(),
            "minified {} >= source {}",
            out.len(),
            src.len()
        );
    }
}
