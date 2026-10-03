#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    http_headers::register(&mut hosts).unwrap();
    http_request::register(&mut hosts).unwrap();
    hosts
}
async fn run(source: &str) -> HostValue {
    let hosts = hosts();
    let program = compiler::compile(source, &hosts).unwrap();
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let result = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    result
}
#[tokio::test]
async fn typed_request_constructor_and_readonly_properties_execute() {
    assert_eq!(run(r#"
fn describe(request: Request) string {
    return request.method+":"+request.url+":"+string(request.headers.entries().length);
}
fn main() string {
    return match Request("https://EXAMPLE.com:443/a/../tour?q=hello world#part") { Ok(request)=>describe(request),Err(error)=>error };
}
"#).await, HostValue::String("GET:https://example.com/tour?q=hello%20world#part:0".into()));
}
#[tokio::test]
async fn constructor_failure_is_a_result_and_never_a_throw() {
    for input in ["://bad", "/relative", "https://user:pass@example.com"] {
        let input = serde_json::to_string(input).unwrap();
        let source = format!(
            r#"fn main() string {{ try {{ return match Request({input}) {{ Ok(request)=>"unexpected",Err(error)=>"handled" }}; }} catch (error) {{ return "threw"; }} }}"#
        );
        assert_eq!(run(&source).await, HostValue::String("handled".into()));
    }
}
#[tokio::test]
async fn request_headers_are_live_and_survive_the_request_binding() {
    assert_eq!(run(r#"
fn retain(request: Request) Headers {
    const first=request.headers; const second=request.headers;
    const result=first.set("X-Project","Deka");
    return second;
}
fn read(headers: Headers) string {
    return match headers.get("x-project") { Ok(value)=>string(value),Err(error)=>error };
}
fn main() string {
    return match Request("https://example.com") { Ok(request)=>read(retain(request)),Err(error)=>error };
}
"#).await, HostValue::String("Some(\"Deka\")".into()));
}
#[tokio::test]
async fn ordinary_global_aliases_shadowing_and_module_exports_work() {
    assert_eq!(run(r#"const Make=Request; fn main() string { return match Make("https://example.com") { Ok(request)=>request.url,Err(error)=>error }; }"#).await, HostValue::String("https://example.com/".into()));
    assert_eq!(run(r#"fn main() string { const Request=fn(input:string) string { return "local:"+input; }; return Request("value"); }"#).await, HostValue::String("local:value".into()));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("request.ds"), r#"export const Make=Request; export fn label(request:Request) string { return request.method+":"+request.url; }"#).unwrap();
    let entry = dir.path().join("main.ds");
    std::fs::write(&entry, r#"import { Make,label } from "./request.ds"; fn main() string { return match Make("https://example.com#fragment") { Ok(value)=>label(value),Err(error)=>error }; }"#).unwrap();
    let hosts = hosts();
    let program = compiler::compile_file(&entry, &hosts, Some("main")).unwrap();
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    drop(dir);
    let mut vm = Vm::new(program, hosts).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String("GET:https://example.com/#fragment".into())
    );
    assert_eq!(vm.stats().live, 0);
}
#[test]
fn invalid_arguments_forged_resources_and_property_assignment_fail_at_compile_time() {
    for source in [
        r#"fn main() { Request(7); }"#,
        r#"fn main() { Request(); }"#,
        r#"fn main() { let request=unwrap(Request("https://example.com")) or {return;}; request.url="https://other.example"; }"#,
        r#"fn main() { Request("https://example.com",{method:"POST"}); }"#,
        r#"fn change(request:Request) { request.url="https://other.example"; }"#,
        r#"fn change(request:Request) { request.method="POST"; }"#,
        r#"fn change(request:Request,headers:Headers) { request.headers=headers; }"#,
        r#"fn consume(request:Request) {} fn main() { consume({url:"https://example.com",method:"GET"}); }"#,
        r#"fn change(request:Request) { request.headers.set("x",7); }"#,
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
}
