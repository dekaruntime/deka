#![cfg(all(feature = "compiler", feature = "ui"))]
use deka_vm::*;
#[test]
fn dynamic_component_lists_capture_row_identity_and_release_old_frames() {
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "names",
            vec![],
            HostType::Strings,
            false,
            None,
            |_| {
                HostReply::Ready(Ok(HostValue::Strings(vec![
                    "First".into(),
                    "Second".into(),
                ])))
            },
        ))
        .unwrap();
    let source = r#"import { names } from "vm:host";
    export fn App() {
      let selected = "";
      let rows = names();
      const selected_value = fn() { return selected; }
      const clear = fn() { rows = []; }
      const view = fn() { return (<view>{rows.map(fn(name: string) { return (<button onClick={fn() { selected = name; }}>{name}</button>); })}</view>); }
      return { selected: selected_value, clear: clear, view: view };
    }"#;
    let program = compiler::compile_entry(source, &hosts, "App").unwrap();
    let mut app = component::Component::new(program, hosts).unwrap();
    for _ in 0..300 {
        let frame = app.render().unwrap();
        assert_eq!(frame.root.children.len(), 2);
        app.event(frame.root.children[1].on_click.unwrap(), vec![])
            .unwrap();
        assert_eq!(
            app.call("selected", vec![]).unwrap(),
            HostValue::String("Second".into())
        );
    }
    assert!(app.stats().slots < 500, "{:?}", app.stats());
    app.call("clear", vec![]).unwrap();
    assert!(app.render().unwrap().root.children.is_empty());
}
