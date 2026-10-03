#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    http_headers::register(&mut hosts).unwrap();
    hosts
}
async fn run(source: &str) -> HostValue {
    let catalog = hosts();
    let program = compiler::compile(source, &catalog).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, catalog).unwrap();
    let value = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    value
}

#[tokio::test]
async fn aliases_mutate_one_header_list_and_defaults_make_an_empty_list() {
    assert_eq!(
        run(r#"
fn lookup(h: Headers, key: string) string {
    const value = unwrap(h.get(key)) or { return "invalid"; };
    return match value { Some(text) => text, None => "missing" };
}
fn main() string {
    const h = unwrap(Headers([["X", " one "], ["x", "two"]])) or { return "constructor"; };
    const shared = h;
    const added = unwrap(shared.append("X", "three")) or { return "append"; };
    const before = lookup(h, "x");
    const changed = unwrap(h.set("X", "replacement")) or { return "set"; };
    const after = lookup(shared, "x");
    const removed = unwrap(shared.delete("x")) or { return "delete"; };
    const empty = unwrap(Headers()) or { return "default"; };
    const present = unwrap(empty.has("X")) or { return "has"; };
    return before + ";" + after + ";" + lookup(h, "x") + ";" + string(present);
}"#)
        .await,
        HostValue::String("one, two, three;replacement;missing;false".into())
    );
}

#[tokio::test]
async fn invalid_operations_are_results_and_do_not_partially_mutate() {
    assert_eq!(run(r#"
fn lookup(h: Headers, key: string) string {
    return match h.get(key) { Ok(value) => match value {Some(text) => text, None => "missing"}, Err(message) => "invalid" };
}
fn main() string {
    const h = unwrap(Headers([["x", "before"], ["empty", ""]])) or { return "constructor"; };
    const badName = match h.append("bad name", "bad") {Ok(value) => "accepted", Err(message) => "name"};
    const badValue = match h.set("x", "a\nb") {Ok(value) => "accepted", Err(message) => "value"};
    const badGet = lookup(h, "bad name");
    const badCtor = match Headers([["x", "ok"], ["bad name", "bad"]]) {Ok(value) => "accepted", Err(message) => "ctor"};
    return badName + ";" + badValue + ";" + badGet + ";" + badCtor + ";" + lookup(h, "x") + ";" + lookup(h, "empty") + ";" + lookup(h, "missing");
}"#).await, HostValue::String("name;value;invalid;ctor;before;;missing".into()));
}

#[tokio::test]
async fn sorted_tuple_and_cookie_collections_are_independent_snapshots() {
    assert_eq!(run(r#"
fn main() string {
    const h = unwrap(Headers([["Z", "last"], ["X", "one"], ["x", "two"], ["Set-Cookie", "a=1"], ["set-cookie", "b=2"]])) or { return "constructor"; };
    const entries = h.entries();
    const keys = h.keys();
    const values = h.values();
    const cookies = h.getSetCookie();
    const changed = unwrap(h.set("x", "new")) or { return "set"; };
    const deleted = unwrap(h.delete("set-cookie")) or { return "delete"; };
    return JSON.stringify(entries) + ";" + JSON.stringify(keys) + ";" + JSON.stringify(values) + ";" + JSON.stringify(cookies) + ";" + JSON.stringify(h.getSetCookie());
}"#).await, HostValue::String(r#"[["set-cookie","a=1"],["set-cookie","b=2"],["x","one, two"],["z","last"]];["set-cookie","set-cookie","x","z"];["a=1","b=2","one, two","last"];["a=1","b=2"];[]"#.into()));
}

#[tokio::test]
async fn header_byte_strings_preserve_latin1_and_http_whitespace_rules() {
    assert_eq!(
        run(r#"
fn main() string {
    const h = unwrap(Headers([["X", "\t café\r\n"], ["x", " keep "]])) or { return "constructor"; };
    const value = unwrap(h.get("X")) or { return "get"; };
    const invalid = match h.set("x", "🙂") {Ok(v) => "accepted", Err(e) => "rejected"};
    return match value {Some(text) => text + ";" + invalid, None => "missing"};
}"#)
        .await,
        HostValue::String("café,  keep ;rejected".into())
    );
}

#[test]
fn wrong_arguments_and_forged_receivers_fail_before_execution() {
    for source in [
        "fn main() {Headers(7);}",
        "fn main() {Headers([[\"x\"]]);}",
        "fn main() {Headers([[\"x\",7]]);}",
        "fn main() {const h:Headers={};h.get(\"x\");}",
        "fn main() {const h=unwrap(Headers()) or {return;};h.get(7);}",
        "fn main() {const h=unwrap(Headers()) or {return;};h.append(\"x\",7);}",
        "fn main() {const h=unwrap(Headers()) or {return;};h.missing();}",
        "fn main() {const h=unwrap(Headers()) or {return;};h.get<number>(\"x\");}",
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
}

#[tokio::test]
async fn imported_header_factories_preserve_native_signatures_through_a_barrel() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("factory.ds"),
        "export fn make() {return Headers([[\"x\",\"hi\"]]);}",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("barrel.ds"),
        "export {make} from \"./factory.ds\";",
    )
    .unwrap();
    let entry = dir.path().join("main.ds");
    std::fs::write(&entry, r#"import {make} from "./barrel.ds";
fn read(h:Headers) string {const value=unwrap(h.get("x")) or{return "get";};return match value{Some(text)=>text,None=>"missing"};}
fn main() string {const h=unwrap(make()) or{return "constructor";};return read(h);}"#).unwrap();
    let catalog = hosts();
    let program = compiler::compile_file(&entry, &catalog, Some("main")).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, catalog).unwrap();
    assert_eq!(vm.run().await.unwrap(), HostValue::String("hi".into()));
    assert_eq!(vm.stats().live, 0);
}

#[cfg(feature = "ui")]
#[test]
fn a_desktop_handler_uses_the_same_catalog_without_opening_a_window() {
    let catalog = hosts();
    let program = compiler::compile_entry(
        r#"
fn App() {
    const headers = unwrap(Headers()) or { return <view>Failed</view>; };
    let text = "Before";
    return (<view><button onClick={fn() {
        const set = unwrap(headers.set("X", "After")) or { return; };
        const value = unwrap(headers.get("x")) or { return; };
        text = match value {Some(value) => value, None => "missing"};
    }}>Set</button><p>{text}</p></view>);
}"#,
        &catalog,
        "App",
    )
    .unwrap();
    let mut session = ui::UiSession::with_hosts(program, catalog).unwrap();
    let before = texts(session.tree());
    assert!(before.contains(&"Before".into()), "{before:?}");
    session.click(0).unwrap();
    let after = texts(session.tree());
    assert!(after.contains(&"After".into()), "{after:?}");
    assert!(!after.contains(&"Before".into()), "{after:?}");
}

#[cfg(feature = "ui")]
fn texts(node: &deka_native_ui::Node) -> Vec<String> {
    let mut output = node.text.iter().cloned().collect::<Vec<_>>();
    for child in &node.children {
        output.extend(texts(child));
    }
    output
}
