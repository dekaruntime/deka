#![cfg(all(feature = "compiler", feature = "ui"))]
use deka_vm::*;
fn app(source: &str) -> component::Component {
    component::Component::new(
        compiler::compile_entry(source, &Hosts::default(), "App").unwrap(),
        Hosts::default(),
    )
    .unwrap()
}
fn text(node: &deka_native_ui::Node) -> String {
    node.text.clone().unwrap_or_default() + &node.children.iter().map(text).collect::<String>()
}
#[test]
fn only_observed_bindings_run_and_branch_changes_refresh_dependencies() {
    let mut app = app(r#"export fn App() {
        let left=1; let right=2; let unused=0; let choose=true;
        return { view:fn(){ return (<view><p>{choose ? left : right}</p></view>); },
            left:fn(){left+=1;}, right:fn(){right+=1;}, unused:fn(){unused+=1;},
            choose:fn(){choose=choose==false;} };
    }"#);
    assert_eq!(text(&app.render().unwrap().root), "1");
    let initial = app.evaluations();
    for _ in 0..10 {
        app.render().unwrap();
    }
    assert_eq!(app.evaluations(), initial);
    app.call("unused", vec![]).unwrap();
    app.render().unwrap();
    assert_eq!(app.evaluations(), initial);
    app.call("right", vec![]).unwrap();
    app.render().unwrap();
    assert_eq!(app.evaluations(), initial);
    app.call("choose", vec![]).unwrap();
    assert_eq!(text(&app.render().unwrap().root), "3");
    let after = app.evaluations();
    app.call("left", vec![]).unwrap();
    app.render().unwrap();
    assert_eq!(app.evaluations(), after);
    app.call("right", vec![]).unwrap();
    assert_eq!(text(&app.render().unwrap().root), "4");
    assert!(app.evaluations() > after);
}
#[test]
fn record_fields_list_items_and_length_are_independent_dependencies() {
    let mut app = app(r#"export fn App() {
        let record={first:1,second:2}; let list=[3,4];
        return {view:fn(){return (<view><p>{record.first}</p><p>{list.has(0) ? list[0] : 0}</p><p>{list.length}</p></view>);},
            second:fn(){record.second=record.second+1;}, other:fn(){if(list.has(1)){list[1]=list[1]+1;}},
            first:fn(){record.first=record.first+1;}, item:fn(){if(list.has(0)){list[0]=list[0]+1;}}, push:fn(){list.push(5);} };
    }"#);
    assert_eq!(text(&app.render().unwrap().root), "132");
    let baseline = app.evaluations();
    app.call("second", vec![]).unwrap();
    app.call("other", vec![]).unwrap();
    app.render().unwrap();
    assert_eq!(app.evaluations(), baseline);
    app.call("first", vec![]).unwrap();
    assert_eq!(text(&app.render().unwrap().root), "232");
    assert_eq!(app.evaluations(), baseline + 1);
    app.call("item", vec![]).unwrap();
    assert_eq!(text(&app.render().unwrap().root), "242");
    assert_eq!(app.evaluations(), baseline + 2);
    app.call("push", vec![]).unwrap();
    assert_eq!(text(&app.render().unwrap().root), "243");
    assert_eq!(app.evaluations(), baseline + 4);
}
#[test]
fn conditional_slots_do_not_steal_a_static_siblings_identity() {
    let mut app = app(r#"export fn App() {
        let visible=true; let count=0;
        return (<view>{visible ? <button>Optional</button> : None}
            <button onClick={fn(){count+=1;}}>{count}</button>
            <button onClick={fn(){visible=visible==false;}}>Toggle</button>
        </view>);
    }"#);
    let first = app.render().unwrap().root;
    let counter = first.children[1].id.clone();
    app.event(first.children[2].on_click.unwrap(), vec![])
        .unwrap();
    let hidden = app.render().unwrap().root;
    assert_eq!(hidden.children[0].id, counter);
    app.event(hidden.children[0].on_click.unwrap(), vec![])
        .unwrap();
    let incremented = app.render().unwrap().root;
    assert_eq!(text(&incremented.children[0]), "1");
    app.event(incremented.children[1].on_click.unwrap(), vec![])
        .unwrap();
    let shown = app.render().unwrap().root;
    assert_eq!(shown.children[1].id, counter);
    assert_ne!(shown.children[0].id, first.children[0].id);
}
#[test]
fn removed_binding_caches_release_values_and_survive_collection() {
    let mut app = app(r#"export fn App() { let visible=true; let count=0;
        return (<view>{visible ? <p>{count}</p> : None}
            <button onClick={fn(){visible=visible==false;count+=1;}}>Toggle</button></view>);
    }"#);
    app.render().unwrap();
    let baseline = app.stats().live;
    for _ in 0..100 {
        let frame = app.render().unwrap().root;
        let button = frame.children.last().unwrap();
        app.event(button.on_click.unwrap(), vec![]).unwrap();
        app.render().unwrap();
    }
    assert!(app.stats().live <= baseline + 10, "{:?}", app.stats());
    assert_eq!(text(&app.render().unwrap().root), "100Toggle");
}
#[test]
fn printing_nested_mutable_data_observes_the_nested_entity() {
    let mut app = app(
        r#"struct Inner { value: number } struct Outer { inner: Inner }
        export fn App(){ let inner=Inner{value:1}; let outer=Outer{inner:inner};
        return {view:fn(){return (<view><p>{string(outer)}</p></view>);},
            change:fn(){inner.value=2;} };
    }"#,
    );
    let initial = app.render().unwrap().root;
    assert!(text(&initial).contains("value: 1"));
    let count = app.evaluations();
    app.call("change", vec![]).unwrap();
    assert!(text(&app.render().unwrap().root).contains("value: 2"));
    assert_eq!(app.evaluations(), count + 1);
}
