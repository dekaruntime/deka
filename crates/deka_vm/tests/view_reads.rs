#![cfg(all(feature = "compiler", feature = "ui"))]
use deka_vm::{HostValue, Hosts, compiler, component::Component};

const SOURCE: &str = r#"fn enlarge(nodes: Array<ViewElement>, node: ViewElement) number {
    let copy = nodes; copy.push(node); return node.children.length;
}
export fn App() {
    let visible = true;
    let title = "First";
    let held: Option<ViewElement> = None;
    return {view: fn() { return (<view id="root"><div id="section" className="p-4">
        {visible ? <p id="message" className="text-sm">{title}<span> child</span></p> : None}
    </div></view>); },
        read: fn() { return match(view.getElementById("message")) {
            Some(node) => node.textContent, None => "Missing"
        }; },
        inspect: fn(node: ViewElement) { return node.textContent; },
        handle: fn() { return view.getElementById("message"); },
        retain: fn() { held = view.getElementById("message"); },
        held: fn() { return match(held) {Some(node) => node.textContent, None => "Empty"}; },
        heldHandle: fn() { return held; },
        heldParent: fn() { return match(held) {Some(node)=>node.parentNode,None=>None}; },
        children: fn() { return match(view.getElementById("message")) {
            Some(node) => node.children.length, None => 0
        }; },
        childSnapshot: fn() { return match(view.getElementById("message")) {
            Some(node) => enlarge(node.children, node),
            None => 0
        }; },
        parent: fn() { return match(view.getElementById("message")) {
            Some(node) => match(node.parentNode) {Some(parent) => parent.textContent, None => "No parent"},
            None => "Missing"
        }; },
        classes: fn() { return match(view.getElementById("message")) {
            Some(node) => node.getAttribute("className"), None => None
        }; },
        id: fn() { return match(view.getElementById("message")) {
            Some(node) => node.getAttribute("id"), None => None
        }; },
        absent: fn() { return match(view.getElementById("message")) {
            Some(node) => node.getAttribute("not-an-attribute"), None => None
        }; },
        classIdentity: fn() { return match(view.getElementById("message")) {
            Some(node) => Some(node.classList), None => None
        }; },
        rename: fn() { title = "Changed"; }, hide: fn() { visible = false; },
        show: fn() { visible = true; }, release: fn() { held = None; }};
}"#;
fn app(source: &str) -> Component {
    Component::new(
        compiler::compile_entry(source, &Hosts::default(), "App").unwrap(),
        Hosts::default(),
    )
    .unwrap()
}
#[test]
fn source_view_reads_live_nodes_and_retains_detached_identity() {
    let mut app = app(SOURCE);
    assert_eq!(
        app.call("read", vec![]).unwrap(),
        HostValue::String("Missing".into())
    );
    app.render().unwrap();
    assert_eq!(
        app.call("read", vec![]).unwrap(),
        HostValue::String("First child".into())
    );
    assert_eq!(app.call("children", vec![]).unwrap(), HostValue::Number(1.));
    assert_eq!(
        app.call("childSnapshot", vec![]).unwrap(),
        HostValue::Number(1.)
    );
    assert_eq!(
        app.call("parent", vec![]).unwrap(),
        HostValue::String("First child".into())
    );
    for (method, value) in [("classes", "text-sm"), ("id", "message")] {
        assert_eq!(
            app.call(method, vec![]).unwrap(),
            HostValue::Option(Some(Box::new(HostValue::String(value.into()))))
        );
    }
    assert_eq!(app.call("absent", vec![]).unwrap(), HostValue::Option(None));
    assert_eq!(
        app.call("classIdentity", vec![]).unwrap(),
        app.call("classIdentity", vec![]).unwrap()
    );
    app.call("retain", vec![]).unwrap();
    app.call("rename", vec![]).unwrap();
    app.render().unwrap();
    assert_eq!(
        app.call("held", vec![]).unwrap(),
        HostValue::String("Changed child".into())
    );
    assert_eq!(
        app.call("heldHandle", vec![]).unwrap(),
        app.call("handle", vec![]).unwrap()
    );
    app.call("hide", vec![]).unwrap();
    app.render().unwrap();
    assert_eq!(
        app.call("heldParent", vec![]).unwrap(),
        HostValue::Option(None)
    );
    assert_eq!(
        app.call("read", vec![]).unwrap(),
        HostValue::String("Missing".into())
    );
    assert_eq!(
        app.call("held", vec![]).unwrap(),
        HostValue::String("Changed child".into())
    );
    app.call("show", vec![]).unwrap();
    app.render().unwrap();
    assert_ne!(
        app.call("heldHandle", vec![]).unwrap(),
        app.call("handle", vec![]).unwrap()
    );
    app.call("release", vec![]).unwrap();
    app.render().unwrap();
    assert_eq!(
        app.call("held", vec![]).unwrap(),
        HostValue::String("Empty".into())
    );
}
#[test]
fn source_view_is_session_scoped_even_when_hosts_are_shared() {
    let hosts = Hosts::default();
    let program = compiler::compile_entry(SOURCE, &hosts, "App").unwrap();
    let mut a = Component::new(program.clone(), hosts.clone()).unwrap();
    let mut b = Component::new(program, hosts).unwrap();
    a.render().unwrap();
    b.render().unwrap();
    let HostValue::Option(Some(foreign)) = a.call("handle", vec![]).unwrap() else {
        panic!("missing node");
    };
    assert!(
        b.call("inspect", vec![*foreign])
            .unwrap_err()
            .contains("another session")
    );
    a.call("rename", vec![]).unwrap();
    a.render().unwrap();
    assert_eq!(
        a.call("read", vec![]).unwrap(),
        HostValue::String("Changed child".into())
    );
    assert_eq!(
        b.call("read", vec![]).unwrap(),
        HostValue::String("First child".into())
    );
}
#[test]
fn view_read_contracts_reject_wrong_arguments_forged_handles_and_writes() {
    for expression in ["view.getElementById(7)", "view.getElementById()"] {
        let source = format!("export fn App() {{ {expression}; return (<view/>); }}");
        assert!(
            compiler::compile_entry(&source, &Hosts::default(), "App").is_err(),
            "{expression}"
        );
    }
    for expression in [
        "node.getAttribute(7)",
        "node.children = []",
        "node.parentNode = None",
        "node.classList = {tokens: []}",
    ] {
        let source = format!(
            "fn inspect(node: ViewElement) {{{expression};}} export fn App() {{return (<view/>);}}"
        );
        let error = compiler::compile_entry(&source, &Hosts::default(), "App").unwrap_err();
        if let Some((field, _)) = expression
            .strip_prefix("node.")
            .and_then(|s| s.split_once(" = "))
        {
            assert!(
                error.contains(field) && error.contains("read-only host property"),
                "{expression}: {error}"
            );
        } else {
            assert!(
                error.contains("number") && error.contains("string"),
                "{expression}: {error}"
            );
        }
    }
    for brand in ["ViewNode", "ViewElement", "ViewClassList"] {
        let source = format!("export fn App() {{ let node: {brand} = {{}}; return (<view/>); }}");
        let error = compiler::compile_entry(&source, &Hosts::default(), "App").unwrap_err();
        assert!(error.contains(brand), "{error}");
    }
}

#[test]
fn authored_attribute_reads_follow_bindings_and_distinguish_absence() {
    let source = r#"export fn App(){let id="old";
        return {view:fn(){return (<view id="root"><p id={id}>First</p><p id={id}>Second</p></view>);},
            lookup:fn(id:string){return match(view.getElementById(id)){Some(node)=>node.textContent,None=>"Missing"};},
            classes:fn(){return match(view.getElementById("root")){Some(node)=>node.getAttribute("className"),None=>None};},
            parent:fn(){return match(view.getElementById("root")){Some(node)=>node.parentNode,None=>None};},
            rename:fn(){id="new";}};}"#;
    let mut app = app(source);
    app.render().unwrap();
    assert_eq!(
        app.call("lookup", vec![HostValue::String("old".into())])
            .unwrap(),
        HostValue::String("First".into())
    );
    assert_eq!(
        app.call("classes", vec![]).unwrap(),
        HostValue::Option(None)
    );
    assert_eq!(app.call("parent", vec![]).unwrap(), HostValue::Option(None));
    app.call("rename", vec![]).unwrap();
    app.render().unwrap();
    assert_eq!(
        app.call("lookup", vec![HostValue::String("old".into())])
            .unwrap(),
        HostValue::String("Missing".into())
    );
    assert_eq!(
        app.call("lookup", vec![HostValue::String("new".into())])
            .unwrap(),
        HostValue::String("First".into())
    );
}

#[test]
fn documented_button_reads_the_live_tree_from_serialized_bytecode() {
    let guide = include_str!("../../../docs/dekascript/native/view-reads.mdx");
    let source = guide
        .split("```deka\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let program = compiler::compile_entry(source, &Hosts::default(), "App").unwrap();
    // The host binds its runtime session after loading bytecode; no source or
    // captured compiler-side tree is needed to resolve the getter operations.
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    let mut session = deka_vm::ui::UiSession::new(program).unwrap();
    assert_eq!(
        session.tree().children[2].children[0].text.as_deref(),
        Some("Click to read the greeting")
    );
    session.click(0).unwrap();
    assert_eq!(
        session.tree().children[2].children[0].text.as_deref(),
        Some("Hello, Deka!")
    );
}
