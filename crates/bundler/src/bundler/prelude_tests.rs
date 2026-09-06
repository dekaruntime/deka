// Prelude-synthesis bundle tests (deka#595). Split from tests.rs to keep
// both files under the 1,000-line file-size gate (deka#391).
use super::tests::{make_tmp_dir, SimpleVirtualSource};
use super::*;


/// deka#595 bundle-size regression fixture: the issue's own two-module
/// reproduction (two modules declaring one `super struct` each, a main
/// module constructing one of each and reading a field). Asserts the EFFECT
/// on the emitted bundle, not bookkeeping: exactly ONE `__deka_struct` for
/// the whole program, and no `impl`/`implMut`/embeds machinery anywhere —
/// neither module has an impl block or an embed. If the per-module prelude
/// synthesis were reverted, the bundle would carry two full copies of the
/// helper (with `implMut` in each) and every count assertion below fails.
#[test]
fn bundle_synthesizes_struct_prelude_once_per_program() {
    let tmp = make_tmp_dir("prelude_once_per_program");
    std::fs::write(
        tmp.join("a.ds"),
        "super struct Point { x: number; y: number }\nexport { Point }\n",
    )
    .expect("write a.ds");
    std::fs::write(
        tmp.join("b.ds"),
        "super struct Size { w: number }\nexport { Size }\n",
    )
    .expect("write b.ds");
    let entry = tmp.join("main.ds");
    std::fs::write(
        &entry,
        "import { Point } from \"./a.ds\";\nimport { Size } from \"./b.ds\";\nconst p = Point { x: 1, y: 2 };\nconst s = Size { w: 3 };\nconst total = p.x + s.w;\n",
    )
    .expect("write main.ds");

    let loader = deka_compile::module_graph::FsModuleLoader::new(tmp.clone());
    let graph = deka_compile::module_graph::compile_module_graph(&entry, &loader)
        .expect("graph compiles");

    struct GraphProvider {
        modules: std::collections::HashMap<std::path::PathBuf, String>,
    }
    impl VirtualSource for GraphProvider {
        fn load_virtual(&self, path: &Path) -> Result<Option<String>, String> {
            let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            Ok(self
                .modules
                .get(&canon)
                .or_else(|| self.modules.get(path))
                .cloned())
        }
    }
    let provider = Arc::new(GraphProvider {
        modules: graph.modules.clone(),
    });
    let bundle = bundle_virtual_entry(
        &entry,
        BundleOptions {
            project_root: tmp.clone(),
            minify: false,
            iife: false,
            client: false,
            prelude: Some(graph.prelude.clone()),
        },
        provider,
    )
    .expect("bundle succeeds");

    eprintln!(
        "deka#595 two-module bundle: bytes={} lines={} __deka_struct_defs={} implMut={}",
        bundle.len(),
        bundle.lines().count(),
        bundle.matches("function __deka_struct").count(),
        bundle.matches("implMut").count(),
    );

    // The whole program shares exactly one factory helper, no matter how
    // many modules declare structs.
    assert_eq!(
        bundle.matches("function __deka_struct").count(),
        1,
        "expected exactly one __deka_struct in the bundle:\n{}",
        bundle
    );
    // Neither module has an impl block or an embed: none of the machinery
    // may appear anywhere in the output.
    assert!(
        !bundle.contains("implMut"),
        "implMut emitted although no module has a mutable impl block:\n{}",
        bundle
    );
    assert!(
        !bundle.contains("f.impl") && !bundle.contains(".impl("),
        "impl emitted although no module has an impl block:\n{}",
        bundle
    );
    assert!(
        !bundle.contains("MutationError"),
        "MutationError class emitted although nothing can throw it:\n{}",
        bundle
    );
    assert!(
        !bundle.contains("Object.entries(embeds)"),
        "embeds loop emitted although no module declares an embed:\n{}",
        bundle
    );
    // The program still works: both factories are constructed through the
    // single shared helper and the field read survives.
    assert!(
        bundle.contains("const Point = __deka_struct(\"Point\")"),
        "Point factory lost:\n{}",
        bundle
    );
    assert!(
        bundle.contains("const Size = __deka_struct(\"Size\")"),
        "Size factory lost:\n{}",
        bundle
    );
    assert!(
        bundle.contains("p.x + s.w"),
        "program content lost:\n{}",
        bundle
    );

    // The combined output must parse as valid JS — the prelude is prepended
    // text, so a malformed splice would only surface here.
    use swc_common::SourceMap;
    use swc_ecma_parser::{EsSyntax, Parser, StringInput, Syntax, lexer::Lexer};
    let cm = swc_common::sync::Lrc::new(SourceMap::default());
    let fm = cm.new_source_file(
        swc_common::FileName::Custom("prelude_once_per_program.js".into()).into(),
        bundle.clone(),
    );
    let lexer = Lexer::new(
        Syntax::Es(EsSyntax {
            jsx: false,
            ..Default::default()
        }),
        Default::default(),
        StringInput::from(&*fm),
        None,
    );
    let mut parser = Parser::new_from(lexer);
    assert!(
        parser.parse_module().is_ok(),
        "bundle with program prelude is not valid JS:\n{}",
        bundle
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// deka#595 member granularity in the other direction: a program whose
/// struct HAS an impl block keeps exactly the members that are used — one
/// `impl`, no `implMut`, no embeds loop, no MutationError.
#[test]
fn bundle_struct_helper_keeps_only_demanded_members() {
    let tmp = make_tmp_dir("prelude_member_granularity");
    std::fs::write(
        tmp.join("a.ds"),
        "struct Point { x: number; y: number }\nfn (p Point) sum() number { return p.x + p.y }\nexport { Point }\n",
    )
    .expect("write a.ds");
    let entry = tmp.join("main.ds");
    std::fs::write(
        &entry,
        "import { Point } from \"./a.ds\";\nconst p = Point { x: 1, y: 2 };\nconst t = p.sum();\n",
    )
    .expect("write main.ds");

    let loader = deka_compile::module_graph::FsModuleLoader::new(tmp.clone());
    let graph = deka_compile::module_graph::compile_module_graph(&entry, &loader)
        .expect("graph compiles");

    let prelude = graph.prelude.clone();
    assert_eq!(
        prelude.matches("function __deka_struct").count(),
        1,
        "program prelude must contain exactly one factory helper:\n{}",
        prelude
    );
    assert!(prelude.contains("f.impl="), "impl member missing:\n{}", prelude);
    assert!(
        !prelude.contains("implMut"),
        "implMut emitted although no mutable method exists:\n{}",
        prelude
    );
    assert!(
        !prelude.contains("MutationError"),
        "MutationError emitted although nothing can throw it:\n{}",
        prelude
    );
    assert!(
        !prelude.contains("Object.entries(embeds)"),
        "embeds loop emitted although no embed exists:\n{}",
        prelude
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn bundle_iife_injects_prelude_inside_wrapper() {
    let tmp = make_tmp_dir("iife_prelude_injection");
    let entry = tmp.join("entry.js");
    let source = r#"
const __deka_main = async () => {
    globalThis.app = () => ({ status: 200, body: "ok" });
};
await __deka_main();
"#;
    std::fs::write(&entry, source).expect("write entry");
    let provider = Arc::new(SimpleVirtualSource {
        entry: entry.clone(),
        code: source.to_string(),
    });
    for minify in [false, true] {
        let result = bundle_virtual_entry(
            &entry,
            BundleOptions {
                project_root: tmp.clone(),
                minify,
                iife: true,
                client: false,
                prelude: Some("function __deka_struct(id) { return id; }\n".to_string()),
            },
            provider.clone(),
        )
        .expect("bundle should succeed");
        let trimmed = result.trim_start();
        assert!(
            trimmed.starts_with("(async function"),
            "minify={minify}: IIFE bundle must still start with the wrapper: {:?}",
            &trimmed[..60.min(trimmed.len())]
        );
        assert!(
            result.contains("__deka_struct"),
            "minify={minify}: prelude missing from bundle:\n{}",
            result
        );
        // The prelude is inside the wrapper: it must not precede it.
        let wrapper_at = result.find("(async function").unwrap_or(usize::MAX);
        let prelude_at = result.find("__deka_struct").unwrap_or(usize::MAX);
        assert!(
            prelude_at > wrapper_at,
            "minify={minify}: prelude must be injected inside the IIFE:\n{}",
            result
        );
        // And the result must remain valid JS.
        use swc_common::SourceMap;
        use swc_ecma_parser::{EsSyntax, Parser, StringInput, Syntax, lexer::Lexer};
        let cm = swc_common::sync::Lrc::new(SourceMap::default());
        let fm = cm.new_source_file(
            swc_common::FileName::Custom(format!("iife_prelude_{minify}.js").into()).into(),
            result.clone(),
        );
        let lexer = Lexer::new(
            Syntax::Es(EsSyntax {
                jsx: false,
                ..Default::default()
            }),
            Default::default(),
            StringInput::from(&*fm),
            None,
        );
        let mut parser = Parser::new_from(lexer);
        assert!(
            parser.parse_script().is_ok(),
            "minify={minify}: injected bundle is not valid JS:\n{}",
            result
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
