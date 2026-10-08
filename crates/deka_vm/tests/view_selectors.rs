#![cfg(all(feature = "compiler", feature = "ui"))]
use deka_vm::{HostValue, Hosts, compiler, component::Component};

const SOURCE: &str = r#"export fn App() {
    let visible = true; let id = "first"; let classes = "p-4 text-sm";
    return {view: fn() { return (<view id="root">
        <div class="p-4"><p id={id} class={classes}>First<span> child</span></p></div>
        <div class="p-4"><div><div class="p-4">
            {visible ? <p id="second" class="text-sm">Second</p> : None}
        </div></div></div>Text</view>); },
        first: fn(selector: string) { return view.querySelector(selector); },
        all: fn(selector: string) { return view.querySelectorAll(selector); },
        byId: fn(id: string) { return view.getElementById(id); },
        rename: fn() { id = "renamed"; classes = "p-8"; }, hide: fn() {visible=false;},
        snapshot: fn() {return match(view.querySelectorAll("p")) {Ok(nodes)=>nodes,Err(_)=>[]};},
    };
}"#;
fn app() -> Component {
    let program = compiler::compile_entry(SOURCE, &Hosts::default(), "App").unwrap();
    // Real serialized bytecode must bind the runtime session, not a compiler tree.
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    Component::new(program, Hosts::default()).unwrap()
}
fn call(app: &mut Component, method: &str, selector: &str) -> HostValue {
    app.call(method, vec![HostValue::String(selector.into())])
        .unwrap()
}
fn some(value: HostValue) -> HostValue {
    HostValue::Enum {
        name: "Result".into(),
        case: "Ok".into(),
        payload: Some(Box::new(value)),
    }
}
fn list(app: &mut Component, selector: &str) -> Vec<HostValue> {
    let HostValue::Enum {
        name,
        case,
        payload: Some(nodes),
    } = call(app, "all", selector)
    else {
        panic!("query failed");
    };
    assert_eq!((name.as_str(), case.as_str()), ("Result", "Ok"));
    let HostValue::List(nodes) = *nodes else {
        panic!("not a list");
    };
    nodes
}
fn by_id(app: &mut Component, id: &str) -> HostValue {
    let HostValue::Option(Some(node)) = call(app, "byId", id) else {
        panic!("missing node");
    };
    *node
}
#[test]
fn source_selectors_match_live_compounds_combinators_order_and_identity() {
    let mut app = app();
    assert_eq!(call(&mut app, "first", "p"), some(HostValue::Option(None)));
    assert_eq!(list(&mut app, "p"), []);
    app.render().unwrap();
    let first = by_id(&mut app, "first");
    let second = by_id(&mut app, "second");
    assert_eq!(list(&mut app, "p"), [first.clone(), second.clone()]);
    for selector in [
        "p#first.p-4.text-sm",
        "#root > .p-4 > p",
        "view > .p-4 .text-sm",
    ] {
        assert_eq!(
            call(&mut app, "first", selector),
            some(HostValue::Option(Some(Box::new(first.clone())))),
            "{selector}"
        );
    }
    // The nearer .p-4 is not a child of view. Backtrack to the outer ancestor.
    assert_eq!(
        list(&mut app, "view > .p-4 .text-sm"),
        [first.clone(), second.clone()]
    );
    assert_eq!(
        list(&mut app, "view > .p-4 > .text-sm"),
        std::slice::from_ref(&first)
    );
    assert_eq!(list(&mut app, "#root"), [by_id(&mut app, "root")]);
    assert!(list(&mut app, ".p").is_empty());
    assert!(list(&mut app, "#absent").is_empty());
    let HostValue::List(snapshot) = app.call("snapshot", vec![]).unwrap() else {
        panic!("missing snapshot");
    };
    app.call("rename", vec![]).unwrap();
    app.render().unwrap();
    assert!(list(&mut app, "#first").is_empty());
    assert_eq!(
        list(&mut app, "p#renamed.p-8"),
        std::slice::from_ref(&first)
    );
    assert_eq!(list(&mut app, ".text-sm"), std::slice::from_ref(&second));
    app.call("hide", vec![]).unwrap();
    app.render().unwrap();
    assert_eq!(list(&mut app, "p"), std::slice::from_ref(&first));
    assert_eq!(snapshot, [first, second]); // detached retained handle survives in snapshot
}
#[test]
fn source_selectors_report_invalid_syntax_as_err_even_without_a_tree() {
    let mut app = app();
    for rendered in [false, true] {
        if rendered {
            app.render().unwrap();
        }
        for selector in [
            "",
            " ",
            "*",
            "p,div",
            "[id]",
            "p:hover",
            "p + div",
            "p ~ div",
            "p >",
            ".1x",
            "p/",
            "#",
            "p >> span",
            ".bg-[red]",
            "\\p",
        ] {
            for method in ["first", "all"] {
                let HostValue::Enum {
                    name,
                    case,
                    payload: Some(error),
                } = call(&mut app, method, selector)
                else {
                    panic!("accepted {selector:?}");
                };
                assert_eq!((name.as_str(), case.as_str()), ("Result", "Err"));
                let HostValue::String(error) = *error else {
                    panic!("missing error");
                };
                assert!(error.starts_with("invalid selector at byte "), "{error}");
            }
        }
    }
}
#[test]
fn selector_catalog_checks_arguments_and_option_result_payloads() {
    for expression in [
        "view.querySelector(7)",
        "view.querySelectorAll()",
        "view.querySelectorAll(false)",
    ] {
        assert!(
            compiler::compile_entry(
                &format!("export fn App(){{{expression};return (<view/>);}}"),
                &Hosts::default(),
                "App"
            )
            .is_err(),
            "{expression}"
        );
    }
    for expression in ["view.querySelector(\"p\")", "view.querySelectorAll(\"p\")"] {
        let source =
            format!("export fn App(){{let forged:ViewElement={expression};return (<view/>);}}");
        let error = compiler::compile_entry(&source, &Hosts::default(), "App").unwrap_err();
        assert!(
            error.contains("ViewElement") && error.contains("Result"),
            "{error}"
        );
    }
}
#[test]
fn documented_selector_button_runs_from_bytecode() {
    let guide = include_str!("../../../docs/dekascript/native/view-selectors.mdx");
    let source = guide
        .split("```deka\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let program = compiler::compile_entry(source, &Hosts::default(), "App").unwrap();
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    let mut session = deka_vm::ui::UiSession::new(program).unwrap();
    session.click(0).unwrap();
    assert_eq!(
        session.tree().children[2].children[0].text.as_deref(),
        Some("Hello, Deka!")
    );
}
