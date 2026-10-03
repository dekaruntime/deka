#![cfg(all(feature = "compiler", feature = "ui"))]
use deka_native_ui::Node;
use deka_vm::*;

fn ids(node: &Node) -> Vec<String> {
    let mut ids = vec![node.id.clone()];
    for child in &node.children {
        ids.extend(self::ids(child));
    }
    ids
}
fn component() -> component::Component {
    let source = r#"export fn App() {
        let count = 0;
        let visible = true;
        let paragraph = false;
        const view = fn() { return (<view>
            <button onClick={fn() { count += 1; }}>{count}</button>
            {visible ? (paragraph ? <p>Paragraph</p> : <span>Span</span>) : None}
        </view>); };
        return {view:view, hide:fn() {visible=false;}, show:fn() {visible=true;},
            swap:fn() {paragraph = paragraph == false;}};
    }"#;
    component::Component::new(
        compiler::compile_entry(source, &Hosts::default(), "App").unwrap(),
        Hosts::default(),
    )
    .unwrap()
}

#[test]
fn component_keeps_identities_but_never_reuses_a_removed_or_replaced_node() {
    let mut app = component();
    let first = app.render().unwrap().root;
    for count in 1..=50 {
        app.event(first.children[0].on_click.unwrap(), vec![])
            .unwrap();
        let frame = app.render().unwrap().root;
        assert_eq!(ids(&frame), ids(&first));
        assert_eq!(frame.children[0].children[0].text, Some(count.to_string()));
    }
    app.call("hide", vec![]).unwrap();
    let hidden = app.render().unwrap().root;
    assert_eq!(hidden.id, first.id);
    assert_eq!(ids(&hidden.children[0]), ids(&first.children[0]));
    assert_eq!(hidden.children.len(), 1);
    app.call("show", vec![]).unwrap();
    let shown = app.render().unwrap().root;
    assert_eq!(ids(&shown.children[0]), ids(&first.children[0]));
    for id in ids(&shown.children[1]) {
        assert!(!ids(&first).contains(&id), "removed identity reused: {id}");
    }
    app.call("swap", vec![]).unwrap();
    let replaced = app.render().unwrap().root;
    assert_eq!(ids(&replaced.children[0]), ids(&first.children[0]));
    assert_ne!(replaced.children[1].id, shown.children[1].id);
    assert_ne!(
        replaced.children[1].children[0].id,
        shown.children[1].children[0].id
    );
    assert_eq!(
        replaced.children[1].children[0].text.as_deref(),
        Some("Paragraph")
    );
    let unique: std::collections::BTreeSet<_> = ids(&replaced).into_iter().collect();
    assert_eq!(unique.len(), ids(&replaced).len());
}

#[test]
fn session_presentation_uses_new_identities_for_new_nodes_and_keeps_other_nodes() {
    let source = r#"export fn App() {
        let paragraph = false;
        let visible = true;
        return (<view>
            <button onClick={fn() {paragraph = paragraph == false;}}>Swap</button>
            <button onClick={fn() {visible = visible == false;}}>Toggle</button>
            {visible ? (paragraph ? <p>Paragraph</p> : <span>Span</span>) : None}
        </view>);
    }"#;
    let program = compiler::compile_entry(source, &Hosts::default(), "App").unwrap();
    let mut first = ui::UiSession::new(program.clone()).unwrap();
    let second = ui::UiSession::new(program).unwrap();
    let initial = first.tree().clone();
    first.click(0).unwrap();
    assert_eq!(ids(&first.tree().children[0]), ids(&initial.children[0]));
    assert_ne!(first.tree().children[2].id, initial.children[2].id);
    let replaced = first.tree().children[2].clone();
    first.click(1).unwrap();
    assert_eq!(first.tree().children.len(), 2);
    first.click(1).unwrap();
    assert_ne!(first.tree().children[2].id, replaced.id);
    assert_eq!(second.tree(), &initial);
}
