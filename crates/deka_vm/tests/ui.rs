#![cfg(all(feature = "compiler", feature = "ui"))]
use deka_native_ui::{Application, Node};
use deka_vm::*;
fn program() -> Program {
    compiler::compile_entry(
        include_str!("../examples/counter.dsx"),
        &Hosts::default(),
        "Counter",
    )
    .unwrap()
}
fn texts(node: &Node) -> Vec<String> {
    let mut output = vec![];
    if let Some(s) = &node.text {
        output.push(s.clone());
    }
    for child in &node.children {
        output.extend(texts(child));
    }
    output
}
#[test]
fn dsx_handlers_patch_retained_nodes_and_gc_preserves_state() {
    let mut session = ui::UiSession::new(program()).unwrap();
    assert!(texts(session.tree()).contains(&"0".into()));
    let address = session.tree() as *const Node;
    let child = session.tree().children.as_ptr();
    for _ in 0..1000 {
        session.click(0).unwrap();
    }
    assert!(texts(session.tree()).contains(&"1000".into()));
    assert_eq!(session.tree() as *const Node, address);
    assert_eq!(session.tree().children.as_ptr(), child);
    assert!(session.stats().slots < 200, "{:?}", session.stats());
    session.click(1).unwrap();
    assert!(texts(session.tree()).contains(&"0".into()));
    assert!(session.click(999).is_err());
}
#[test]
fn source_edits_drive_actual_vm_handlers_and_idle_render_does_no_vm_work() {
    let source = include_str!("../examples/counter.dsx").replace("count += 1", "count += 7");
    let p = compiler::compile_entry(&source, &Hosts::default(), "Counter").unwrap();
    let app = ui::VmApp::new(p).unwrap();
    assert!(deka_native_ui::exercise(app, 3).contains("21"));
    let app = ui::VmApp::new(program()).unwrap();
    let initial = app.render(&[]);
    let instructions = app.instructions();
    for _ in 0..60 {
        assert_eq!(texts(&app.render(&[])), texts(&initial));
    }
    assert_eq!(app.instructions(), instructions);
}
#[test]
fn independent_windows_have_independent_state() {
    let mut first = ui::UiSession::new(program()).unwrap();
    let second = ui::UiSession::new(program()).unwrap();
    first.click(0).unwrap();
    assert!(texts(first.tree()).contains(&"1".into()));
    assert!(texts(second.tree()).contains(&"0".into()));
}
#[test]
fn unknown_ui_attributes_and_async_handlers_fail_compilation() {
    for (from, to) in [
        ("className=", "unknown="),
        ("onClick={fn()", "onClick={async fn()"),
    ] {
        let source = include_str!("../examples/counter.dsx").replace(from, to);
        assert!(compiler::compile_entry(&source, &Hosts::default(), "Counter").is_err());
    }
}
#[cfg(feature = "v8-control")]
#[test]
fn v8_and_vm_produce_the_same_native_scene() {
    for clicks in [0, 1, 17] {
        let mut vm: serde_json::Value = serde_json::from_str(
            &ui::snapshot(ui::VmApp::new(program()).unwrap(), clicks).unwrap(),
        )
        .unwrap();
        let mut v8: serde_json::Value =
            serde_json::from_str(&ui::snapshot(v8_control::V8App::new().unwrap(), clicks).unwrap())
                .unwrap();
        // Glyph cache iteration order is not a visual property.
        for scene in [&mut vm, &mut v8] {
            if let Some(images) = scene.get_mut("images").and_then(|v| v.as_array_mut()) {
                images.sort_by_key(|v| v["id"].as_str().unwrap().to_owned());
            }
        }
        assert_eq!(vm, v8, "scene mismatch after {clicks} clicks");
    }
}

#[test]
fn conditionals_select_one_branch_and_dynamic_lists_keep_live_handlers() {
    let source = r#"export fn App() {
        let selected = "None";
        let changed = 0;
        const branch = fn() { changed += 1; return "chosen"; }
        const unused = fn() { changed += 100; return "wrong"; }
        const initial = true ? branch() : unused();
        const rows = ["One", "Two"];
        return (<view>
            <p>{initial}</p><p>{changed}</p><p>{selected}</p>
            {rows.map(fn(name: string) { return (<button
                className={selected == name ? "bg-[#00ff00]" : "bg-[#ffffff]"}
                onClick={fn() { selected = name; }}>{name}</button>); })}
            {selected == "Two" ? <p>Second selected</p> : None}
        </view>);
    }"#;
    let program = compiler::compile_entry(source, &Hosts::default(), "App").unwrap();
    let mut session = ui::UiSession::new(program).unwrap();
    assert!(texts(session.tree()).contains(&"chosen".into()));
    assert!(texts(session.tree()).contains(&"1".into()));
    assert!(!texts(session.tree()).contains(&"Second selected".into()));
    for _ in 0..100 {
        session.click(1).unwrap();
        assert!(texts(session.tree()).contains(&"Second selected".into()));
        assert_eq!(session.tree().children[4].style.background, Some(0x00ff00));
        session.click(0).unwrap();
        assert!(!texts(session.tree()).contains(&"Second selected".into()));
    }
    assert!(session.stats().slots < 1000, "{:?}", session.stats());
}
