//! JavaScript output formatter backed by SWC.

use swc_common::{FileName, SourceMap, sync::Lrc};
use swc_ecma_codegen::{text_writer::JsWriter, Config as CodegenConfig, Emitter};
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax};

/// Format a JavaScript source string using SWC's parser and code generator.
pub fn format_js(source: &str) -> Result<String, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(FileName::Anon.into(), source.to_string());

    let lexer = Lexer::new(
        Syntax::Es(Default::default()),
        Default::default(),
        StringInput::from(&*fm),
        None,
    );
    let mut parser = Parser::new_from(lexer);

    let module = parser
        .parse_module()
        .map_err(|err| format!("failed to parse JavaScript: {:?}", err))?;

    let mut buf = Vec::new();
    let mut emitter = Emitter {
        cfg: CodegenConfig::default().with_minify(false),
        comments: None,
        cm: cm.clone(),
        wr: JsWriter::new(cm, "\n", &mut buf, None),
    };
    emitter
        .emit_module(&module)
        .map_err(|err| format!("failed to emit formatted JavaScript: {err}"))?;

    String::from_utf8(buf).map_err(|err| format!("formatted JavaScript was not UTF-8: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_basic_function() {
        let input = "function add(left,right){return left+right;}\n";
        let output = format_js(input).unwrap();
        assert!(output.contains("function add(left, right)"));
        assert!(output.contains("return left + right"));
    }

    #[test]
    fn formats_struct_emit() {
        let input = r#"const origin=(()=>{const __obj={"__struct":"Point","x":3,"y":4};const __m=globalThis.__phpxStructMethods?globalThis.__phpxStructMethods["Point"]:null;if(__m)Object.assign(__obj,__m);return __obj;})();
console.log((origin.x+origin.y));
"#;
        let output = format_js(input).unwrap();
        assert!(output.contains("const origin"));
        assert!(output.contains("console.log"));
    }
}
