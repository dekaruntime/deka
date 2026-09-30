use deka_native::{DevApp, compile_file};
use deka_native_compile::compile;
use deka_native_ir::Node;
use deka_native_ui::{Application, Host, Reload};
const SOURCE: &str = include_str!("../../../examples/native/counter.dsx");
fn text(node: &Node) -> String {
    let mut chunks = vec![];
    if let Some(t) = &node.text {
        chunks.push(t.clone());
    }
    chunks.extend(node.children.iter().map(text));
    chunks.join(" ")
}
#[test]
fn real_deka_handler_changes_rendered_text() {
    let mut host = Host::new(DevApp::new(compile(SOURCE).unwrap()).unwrap());
    assert!(text(&host.render()).ends_with('0'));
    let handler = host.render().children[2].on_click.unwrap();
    host.click(handler);
    host.click(handler);
    assert!(text(&host.render()).ends_with('2'));
}
#[test]
fn replacing_code_preserves_state_and_uses_new_handler() {
    let mut host = Host::new(DevApp::new(compile(SOURCE).unwrap()).unwrap());
    for _ in 0..7 {
        host.click(0);
    }
    let edit = SOURCE
        .replace("count + 1", "count + 2")
        .replace("Deka, native.", "Changed live");
    assert!(matches!(
        host.app.replace(compile(&edit).unwrap()).unwrap(),
        Reload::Preserve
    ));
    assert_eq!(host.state, vec![7.]);
    host.click(0);
    let frame = text(&host.render());
    assert!(frame.contains("Changed live"));
    assert!(frame.ends_with('9'));
}
#[test]
fn invalid_reload_keeps_working_state_and_code() {
    let mut host = Host::new(DevApp::new(compile(SOURCE).unwrap()).unwrap());
    host.click(0);
    assert!(compile(&SOURCE.replace("count + 1", "unknown + 1")).is_err());
    let mut invalid = compile(SOURCE).unwrap();
    invalid.handlers[0].state = 999;
    assert!(host.app.replace(invalid).is_err());
    host.click(0);
    assert!(text(&host.render()).ends_with('2'));
}
#[test]
fn incompatible_hook_layout_requests_reset() {
    let mut app = DevApp::new(compile(SOURCE).unwrap()).unwrap();
    let edit = SOURCE
        .replace("count", "total")
        .replace("useState(0)", "useState(42)");
    assert!(matches!(
        app.replace(compile(&edit).unwrap()).unwrap(),
        Reload::Reset
    ));
    assert_eq!(app.initial_state(), vec![42.]);
}
#[test]
fn missing_compiler_is_a_failure() {
    assert!(
        compile_file(
            std::path::Path::new("absent.dsx"),
            std::path::Path::new("/no/such/compiler")
        )
        .is_err()
    );
}
