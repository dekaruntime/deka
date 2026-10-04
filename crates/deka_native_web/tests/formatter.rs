use deka_native_web::format_ds;
#[test]
fn browser_export_formats_source_and_preserves_incomplete_input() {
    assert_eq!(format_ds("const count=0\n").unwrap(), "const count = 0\n");
    assert_eq!(format_ds("fn App(\n").unwrap(), "fn App(\n");
}
#[test]
fn browser_export_handles_nested_components_comments_and_slots() {
    let source = "// reusable card\nfn Card(){return <div><slot /></div>}\nexport fn App(){return <view><Card><p>Hi</p></Card></view>}\n";
    let once = format_ds(source).unwrap();
    assert_ne!(once, source);
    assert!(
        once.contains("// reusable card")
            && once.contains("<slot />")
            && once.contains("<Card><p>Hi</p></Card>"),
        "{once}"
    );
    assert_eq!(format_ds(&once).unwrap(), once);
}
