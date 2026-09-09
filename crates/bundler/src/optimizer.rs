//! Standalone-module optimization: the restricted SWC minify configuration
//! shared by bundle minification (`crate::bundler`) and the module-preserving
//! dist client-asset path (`optimize_emitted_module`, deka#750).

use std::path::{Path, PathBuf};

use swc_common::sync::Lrc;
use swc_common::{FileName, GLOBALS, Globals, Mark, SourceMap};
use swc_ecma_ast::{EsVersion, Module, Pass, Program};
use swc_ecma_codegen::{Emitter, text_writer::JsWriter};
use swc_ecma_minifier::optimize;
use swc_ecma_minifier::option::{CompressOptions, MangleOptions, MinifyOptions};
use swc_ecma_parser::{EsSyntax, Parser, StringInput, Syntax, lexer::Lexer};
use swc_ecma_transforms_base::resolver;

/// Optimize already-emitted ESM without resolving or bundling imports.
///
/// Contract: source and output are ESM; relative specifiers are preserved.
/// This is the same safe SWC configuration used for `BundleOptions::minify`
/// and exists for the CLI's module-preserving `--treeshake` mode.
pub fn optimize_emitted_module(source: &str, path: &Path) -> Result<String, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(
        FileName::Real(path.to_path_buf()).into(),
        source.to_string(),
    );
    let syntax = Syntax::Es(EsSyntax {
        jsx: false,
        export_default_from: true,
        import_attributes: true,
        ..Default::default()
    });
    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let module = parser
        .parse_module()
        .map_err(|err| format!("failed to parse emitted JavaScript: {err:?}"))?;
    let globals = Globals::new();
    let module = GLOBALS.set(&globals, || {
        // Mangling requires resolver hygiene marks: without them shadowed
        // bindings collide under renaming (e.g. a param and a same-named
        // `let` in its body). Resolve with the marks the minifier sees.
        let top_level_mark = Mark::new();
        let unresolved_mark = Mark::new();
        let mut program = Program::Module(module);
        resolver(unresolved_mark, top_level_mark, false).process(&mut program);
        let module = match program {
            Program::Module(module) => module,
            Program::Script(_) => unreachable!("emitted module parsed as a script"),
        };
        minify_module(module, cm.clone(), true, unresolved_mark, top_level_mark)
    });
    emit_module_compact(&module, cm)
}

pub(crate) fn minify_module(
    module: Module,
    cm: Lrc<SourceMap>,
    mangle_locals: bool,
    unresolved_mark: Mark,
    top_level_mark: Mark,
) -> Module {
    // These restrictions guard known SWC output bugs: conditionals/bools can
    // emit invalid assignment expressions, sequences can corrupt for-of heads,
    // inline can merge module-local bindings, and if_return can lose ternary
    // parentheses. Keep this shared configuration in sync for bundling and
    // module-preserving transpile optimization.
    let mut compress = CompressOptions::default();
    compress.conditionals = false;
    compress.bools = false;
    compress.sequences = 0;
    compress.inline = 0;
    compress.if_return = false;
    // Local mangling (top-level names untouched, so exports and their string
    // identities survive) is enabled only for the standalone-module path used
    // by dist client assets; bundle output keeps the historical no-mangle.
    let mangle = mangle_locals.then(|| MangleOptions {
        top_level: Some(false),
        ..Default::default()
    });
    let minify_options = MinifyOptions {
        compress: Some(compress),
        mangle,
        ..Default::default()
    };
    match optimize(
        Program::Module(module),
        cm,
        None,
        None,
        &minify_options,
        &swc_ecma_minifier::option::ExtraOptions {
            unresolved_mark,
            top_level_mark,
            mangle_name_cache: Default::default(),
        },
    ) {
        Program::Module(module) => module,
        Program::Script(_) => unreachable!("module optimization returned a script"),
    }
}

/// Minify a standalone JS module source with the same restricted SWC
/// configuration as bundle minification (see `minify_module`).
pub(crate) fn minify_source_text(name: &str, source: &str) -> Result<String, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(FileName::Real(PathBuf::from(name)).into(), source.to_string());
    let syntax = Syntax::Es(EsSyntax {
        jsx: false,
        export_default_from: true,
        import_attributes: true,
        ..Default::default()
    });
    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let module = parser
        .parse_module()
        .map_err(|err| format!("failed to parse prelude for minification: {err:?}"))?;
    let globals = Globals::new();
    let module = GLOBALS.set(&globals, || {
        minify_module(module, cm.clone(), false, Mark::new(), Mark::new())
    });
    emit_module(&module, cm)
}

fn emit_module(module: &Module, cm: Lrc<SourceMap>) -> Result<String, String> {
    let mut buf = Vec::new();
    let mut emitter = Emitter {
        cfg: swc_ecma_codegen::Config::default(),
        comments: None,
        cm: cm.clone(),
        wr: JsWriter::new(cm, "\n", &mut buf, None),
    };
    emitter
        .emit_module(module)
        .map_err(|err| format!("failed to emit optimized JavaScript: {err}"))?;
    String::from_utf8(buf).map_err(|err| format!("optimized JavaScript was not UTF-8: {err}"))
}

/// Compact emission (no pretty whitespace) for minified standalone modules;
/// the bundle emitter keeps its historical formatting.
fn emit_module_compact(module: &Module, cm: Lrc<SourceMap>) -> Result<String, String> {
    let mut cfg = swc_ecma_codegen::Config::default();
    cfg.minify = true;
    let mut buf = Vec::new();
    let mut emitter = Emitter {
        cfg,
        comments: None,
        cm: cm.clone(),
        wr: JsWriter::new(cm, "", &mut buf, None),
    };
    emitter
        .emit_module(module)
        .map_err(|err| format!("failed to emit optimized JavaScript: {err}"))?;
    String::from_utf8(buf).map_err(|err| format!("optimized JavaScript was not UTF-8: {err}"))
}
