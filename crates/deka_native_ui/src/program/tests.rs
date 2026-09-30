use super::*;
use deka_native_ir::{State, Style, Update};
fn leaf(id: &str, text: Text) -> Template {
    Template {
        id: id.into(),
        style: Style::default(),
        style_when: None,
        visible_when: None,
        text: Some(text),
        on_click: None,
        children: vec![],
    }
}
fn program() -> Program {
    let mut root = leaf("root", Text::Literal("Counter".into()));
    root.children = vec![
        leaf("count", Text::Number(Number::State(0))),
        leaf("other", Text::Number(Number::State(1))),
        leaf("static", Text::Literal("untouched".into())),
    ];
    Program {
        format: FORMAT_VERSION,
        component: "Counter".into(),
        states: vec![
            State {
                name: "count".into(),
                initial: 0.,
            },
            State {
                name: "other".into(),
                initial: 10.,
            },
        ],
        handlers: vec![Update {
            state: 0,
            value: Number::Add(Box::new(Number::State(0)), Box::new(Number::Literal(1.))),
        }],
        root,
    }
}
#[test]
fn updates_only_dependent_bindings_and_keeps_node_allocations() {
    let mut host = crate::Host::new(ProgramApp::new(program()).unwrap());
    host.render();
    let stats = host.app.binding_stats();
    let addresses = {
        let c = host.app.retained.borrow();
        let t = c.as_ref().unwrap();
        t.root
            .children
            .iter()
            .map(|n| n as *const Node)
            .collect::<Vec<_>>()
    };
    host.click(0);
    let updated = host.render();
    assert_eq!(updated.children[0].text.as_deref(), Some("1"));
    assert_eq!(updated.children[1].text.as_deref(), Some("10"));
    let after = host.app.binding_stats();
    assert_eq!(after.tree_builds, 1);
    assert_eq!(after.nodes_created, stats.nodes_created);
    assert_eq!(after.binding_evaluations, stats.binding_evaluations + 1);
    assert_eq!(after.changed_nodes, vec!["count"]);
    {
        let c = host.app.retained.borrow();
        let t = c.as_ref().unwrap();
        assert_eq!(
            addresses,
            t.root
                .children
                .iter()
                .map(|n| n as *const Node)
                .collect::<Vec<_>>()
        );
    }
    for _ in 0..60 {
        host.render();
    }
    assert_eq!(
        host.app.binding_stats().binding_evaluations,
        after.binding_evaluations
    );
    host.state[0] = 0.;
    host.render();
    assert_eq!(host.render().children[0].text.as_deref(), Some("0"));
}
#[test]
fn instances_and_compatible_reload_have_separate_lifetimes() {
    let mut a = crate::Host::new(ProgramApp::new(program()).unwrap());
    let b = crate::Host::new(ProgramApp::new(program()).unwrap());
    a.render();
    b.render();
    a.click(0);
    assert_eq!(b.render().children[0].text.as_deref(), Some("0"));
    let mut changed = program();
    changed.root.children[0].text = Some(Text::Number(Number::Mul(
        Box::new(Number::State(0)),
        Box::new(Number::Literal(2.)),
    )));
    assert!(matches!(a.app.replace(changed), Ok(Reload::Preserve)));
    assert_eq!(a.render().children[0].text.as_deref(), Some("2"));
    assert_eq!(a.app.binding_stats().tree_builds, 1);
    let mut invalid = program();
    invalid.handlers[0].state = 99;
    assert!(a.app.replace(invalid).is_err());
    assert_eq!(a.render().children[0].text.as_deref(), Some("2"));
}
