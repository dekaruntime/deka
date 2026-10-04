use std::{fs, path::Path};
fn roundtrip(source: &str) -> String {
    let arena = bumpalo::Bump::new();
    let input = deka_syntax::parse(source, &arena);
    assert!(
        input.errors.is_empty() && input.program.is_some(),
        "invalid fixture: {:?}",
        input.errors
    );
    let once = deka_fmt::format_ds(source).unwrap();
    let output_arena = bumpalo::Bump::new();
    let output = deka_syntax::parse(&once, &output_arena);
    assert!(
        output.errors.is_empty() && output.program.is_some(),
        "{once}\n{:?}",
        output.errors
    );
    assert_eq!(
        once,
        deka_fmt::format_ds(&once).unwrap(),
        "second pass changed source"
    );
    once
}
#[test]
fn current_tour_sources_parse_and_reach_a_fixed_point() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tour");
    let mut count = 0;
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("dsx") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        roundtrip(&source);
        count += 1;
    }
    assert_eq!(count, 27, "tour snapshot lookup lost fixtures");
}
#[test]
fn native_component_children_slot_and_async_handler_roundtrip() {
    let source = "fn Card(title: string, children: ReactNode) {return <div><p>{title}</p><slot /></div>}\nexport fn App(){return <view><Card title=\"Project\"><button onClick={async fn(){await sleep(0)}}>Open</button></Card></view>}\n";
    let once = roundtrip(source);
    for preserved in [
        "<Card title={\"Project\"}>",
        "<slot />",
        "await sleep(0)",
        "</Card>",
    ] {
        assert!(once.contains(preserved), "{once}");
    }
}
#[test]
fn incomplete_source_stays_byte_identical() {
    for source in [
        "fn App(",
        "const =",
        "<view><",
        "const value = \"unfinished",
    ] {
        assert_eq!(deka_fmt::format_ds(source).unwrap(), source);
    }
}
#[test]
fn exported_function_span_includes_body_and_keeps_adjacent_exports_stable() {
    let source = include_str!("fixtures/imported_exception.ds");
    let once = roundtrip(source);
    assert!(
        !once.contains("}\n\nexport { Fault }"),
        "invented an empty line:\n{once}"
    );
    // The body is on lines 3-5. Both export spellings must cover it before
    // the formatter measures the source gap to a following statement.
    for modifier in ["", "async "] {
        let source = format!("export {modifier}fn source() number {{\n  const value = 7\n  return value\n}}\nexport {{ source }}\n");
        let arena = bumpalo::Bump::new();
        let parsed = deka_syntax::parse(&source, &arena);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let deka_syntax::ast::Stmt::Export { span, .. } = &parsed.program.unwrap().statements[0]
        else {
            panic!("expected export")
        };
        assert!(
            span.end.line >= 4,
            "{modifier:?}: truncated body span: {span:?}"
        );
        roundtrip(&source);
    }
}
