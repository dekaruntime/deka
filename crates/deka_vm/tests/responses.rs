#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    http_headers::register(&mut hosts).unwrap();
    text_codec::register(&mut hosts).unwrap();
    http_response::register(&mut hosts).unwrap();
    hosts
}
async fn execute(program: Program, hosts: Hosts) -> HostValue {
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let value = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    value
}
async fn run(source: &str) -> HostValue {
    let hosts = hosts();
    execute(compiler::compile(source, &hosts).unwrap(), hosts).await
}

#[tokio::test]
async fn metadata_aliases_and_async_reads_share_one_body_state() {
    assert_eq!(run(r#"
async fn main() Promise<string> {
    const response = unwrap(Response("Hello, 雪")) or { return "constructor"; };
    const shared = response;
    const headers = shared.headers;
    const changed = unwrap(headers.set("X", "Deka")) or { return "set"; };
    const value = unwrap(response.headers.get("x")) or { return "get"; };
    const content = unwrap(response.headers.get("content-type")) or { return "type"; };
    const before = response.bodyUsed;
    const reading = response.text();
    const immediate = shared.bodyUsed;
    const text = unwrap(await reading) or { return "read"; };
    const twice = match await shared.bytes() { Ok(b) => "accepted", Err(e) => e };
    return string(response.status) + ";" + string(response.ok) + ";" + string(before)
        + ";" + string(immediate) + ";" + text + ";" + string(value) + ";" + string(content) + ";" + twice;
}"#).await, HostValue::String("200;true;false;true;Hello, 雪;Some(\"Deka\");Some(\"text/plain;charset=UTF-8\");response body has already been consumed".into()));
}

#[tokio::test]
async fn json_is_a_typed_async_result_and_consumes_at_the_call_site() {
    assert_eq!(run(r#"
struct Person { name: string; }
interface Greeting { fn greet() string; }
fn (p Person) greet() string { return "Hello, " + p.name; }
fn greeting(p: Greeting) string { return p.greet(); }
async fn main() Promise<string> {
    const response = unwrap(Response(JSON.stringify(Person { name: "Deka" }))) or { return "constructor"; };
    let calls = 0;
    const select = fn() Response { calls += 1; return response; };
    const parsed: Promise<Result<Person,string>> = select().json<Person>();
    const consumed = response.bodyUsed;
    const rival = match await response.text() {Ok(t)=>"bad",Err(e)=>"used"};
    const person = unwrap(await parsed) or { return "parse"; };
    return greeting(person) + ";" + person.getType().toString() + ";" + string(consumed) + ";" + rival + ";" + string(calls);
}"#).await, HostValue::String("Hello, Deka;Person;true;used;1".into()));
}

#[tokio::test]
async fn failed_json_shape_and_double_read_are_results_not_throw() {
    assert_eq!(
        run(r#"
async fn main() Promise<string> {
    const broken = unwrap(Response("{")) or { return "constructor"; };
    const wrong = unwrap(Response("\"no\"")) or { return "constructor"; };
    const nil = unwrap(Response("null")) or { return "constructor"; };
    const a = match await broken.json<number>() {Ok(v)=>"bad",Err(e)=>"syntax"};
    const b = match await wrong.json<number>() {Ok(v)=>"bad",Err(e)=>"shape"};
    const c = match await nil.json<number>() {Ok(v)=>"bad",Err(e)=>"null"};
    const d = match await broken.json<number>() {Ok(v)=>"bad",Err(e)=>e};
    return a + ";" + b + ";" + c + ";" + d;
}"#)
        .await,
        HostValue::String("syntax;shape;null;response body has already been consumed".into())
    );
}

#[tokio::test]
async fn bytes_are_exact_and_json_namespace_and_struct_methods_still_work() {
    assert_eq!(
        run(r#"
struct Local { n: number; }
fn (l Local) json() number { return l.n; }
alias Numbers = Array<number>;
async fn main() Promise<string> {
    const response = unwrap(Response("雪")) or { return "constructor"; };
    const b = unwrap(await response.bytes()) or { return "bytes"; };
    const decoder = unwrap(TextDecoder()) or { return "decoder"; };
    const text = unwrap(decoder.decode(b)) or { return "decode"; };
    const list = unwrap(Response("[1,2]")) or { return "constructor"; };
    const parsed = unwrap(await list.json<Numbers>()) or { return "json"; };
    return text + ";" + JSON.stringify(parsed) + ";" + string(Local{n:7}.json());
}"#)
        .await,
        HostValue::String("雪;[1,2];7".into())
    );
}

#[test]
fn invalid_body_arguments_properties_and_json_types_fail_before_execution() {
    for source in [
        "fn main() {Response();}",
        "fn main() {Response(7);}",
        "fn main() {let r=unwrap(Response(\"x\")) or{return;};r.status=201;}",
        "fn main() {let r=unwrap(Response(\"x\")) or{return;};r.headers=unwrap(Headers()) or{return;};}",
        "fn main() {const r=unwrap(Response(\"x\")) or{return;};r.text(7);}",
        "fn main() {const r=unwrap(Response(\"x\")) or{return;};r.json();}",
        "fn main() {const r=unwrap(Response(\"x\")) or{return;};r.json<number>(7);}",
        "fn main() {const r:Response={};r.text();}",
        "async fn main() Promise<string> {const r=unwrap(Response(\"x\")) or{return \"ctor\";};return await r.json<number>();}",
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
    for ty in ["Option<number>", "Result<number,string>", "number | string"] {
        let error = compiler::compile(&format!("alias T={ty};fn main(){{const r=unwrap(Response(\"x\")) or{{return;}};r.json<T>();}}"), &hosts()).unwrap_err();
        assert!(error.contains("APS 43 type-mapping decision"), "{error}");
    }
}

#[tokio::test]
async fn imported_response_and_private_nominal_json_factories_survive_source_deletion() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("model.ds"),r#"
struct Inner { x: number; }
interface Reader { fn read() number; }
fn (i Inner) read() number { return i.x; }
struct Outer { inner: Inner; }
export {Outer};
fn read(i: Reader) number { return i.read(); }
export fn describe(o: Outer) string { return o.inner.getType().toString()+":"+string(read(o.inner)); }
export fn make() { return Response("{\"Outer\":{\"inner\":{\"Inner\":{\"x\":7}}}}"); }
"#).unwrap();
    std::fs::write(
        dir.path().join("barrel.ds"),
        "export {Outer, describe, make} from \"./model.ds\";",
    )
    .unwrap();
    let entry = dir.path().join("main.ds");
    std::fs::write(&entry,r#"import {Outer, describe, make} from "./barrel.ds";
async fn main() Promise<string> { const r=unwrap(make()) or{return "constructor";};return match await r.json<Outer>() {Ok(o)=>describe(o),Err(e)=>e}; }"#).unwrap();
    let hosts = hosts();
    let program = compiler::compile_file(&entry, &hosts, Some("main")).unwrap();
    drop(dir);
    assert_eq!(
        execute(program, hosts).await,
        HostValue::String("Inner:7".into())
    );
}

#[cfg(feature = "ui")]
#[test]
fn desktop_async_handler_decodes_and_updates_then_stays_idle() {
    use deka_native_ui::Host;
    let hosts = hosts();
    let program = compiler::compile_entry(
        r#"
export fn App() {
    let message="Ready";
    return (<view><p>{message}</p><button onClick={async fn() {
        const r=unwrap(Response("\"After\"")) or{return;};
        message=match await r.json<string>() {Ok(text)=>text,Err(e)=>e};
    }}>Read</button></view>);
}"#,
        &hosts,
        "App",
    )
    .unwrap();
    let mut host = Host::new(ui::VmApp::with_hosts(program, hosts).unwrap());
    fn text(node: &deka_native_ui::Node) -> String {
        node.text.clone().unwrap_or_default() + &node.children.iter().map(text).collect::<String>()
    }
    assert_eq!(text(&host.render()), "ReadyRead");
    host.click(0);
    let mut turns = 0;
    while host.has_ready_work() {
        host.run_turn(1);
        turns += 1;
        assert!(turns < 2000);
    }
    assert_eq!(text(&host.render()), "AfterRead");
    let instructions = host.app.instructions();
    for _ in 0..32 {
        assert!(!host.run_turn(1));
    }
    assert_eq!(host.app.instructions(), instructions);
}

#[test]
fn json_body_catalog_cannot_declare_an_untyped_or_synchronous_reader() {
    for (asynchronous, channel, result) in [
        (false, true, HostType::String),
        (true, false, HostType::String),
        (true, true, HostType::Bytes),
    ] {
        let mut hosts = Hosts::default();
        let op = HostOp::new(
            "read",
            vec![HostType::Handle("Resource".into())],
            result,
            asynchronous,
            |_| HostReply::Ready(Ok(HostValue::String("7".into()))),
        )
        .with_receiver_method("Resource", "json")
        .with_json_body();
        let op = if channel {
            op.with_result_channel()
        } else {
            op
        };
        assert!(hosts.register(op).is_err());
    }
}
#[tokio::test]
async fn transport_created_absent_body_and_failed_status_have_the_same_language_contract() {
    let mut hosts = hosts();
    hosts
        .register(
            HostOp::new(
                "empty",
                vec![],
                HostType::Handle("Response".into()),
                false,
                |_| {
                    HostReply::Ready(
                        http_response::ResponseObject::from_parts(
                            404,
                            "Not Found".into(),
                            "https://example.test/missing".into(),
                            http_headers::HeaderList::default(),
                            None,
                        )
                        .map(http_response::ResponseObject::into_host_value),
                    )
                },
            )
            .with_result_channel()
            .with_global_binding(),
        )
        .unwrap();
    let program=compiler::compile(r#"
async fn main() Promise<string> {
    const r=unwrap(empty()) or{return "ctor";};
    const first=unwrap(await r.text()) or{return "first";};
    const second=unwrap(await r.text()) or{return "second";};
    const json=match await r.json<number>(){Ok(v)=>"accepted",Err(e)=>"parse error"};
    return string(r.status)+";"+string(r.ok)+";"+r.statusText+";"+r.url+";"+string(r.bodyUsed)+";"+first+second+json;
}"#,&hosts).unwrap();
    assert_eq!(
        execute(program, hosts).await,
        HostValue::String(
            "404;false;Not Found;https://example.test/missing;false;parse error".into()
        )
    );
}
