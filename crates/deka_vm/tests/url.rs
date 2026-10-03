#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    deka_vm::url::register(&mut hosts).unwrap();
    hosts
}
async fn run(source: &str) -> HostValue {
    let hosts = hosts();
    let program = compiler::compile(source, &hosts).unwrap();
    // Standalone bytecode has all property/method dispatch; no source at runtime.
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let value = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    value
}
#[tokio::test]
async fn url_properties_and_fallible_constructor_are_typed_globals() {
    assert_eq!(run(r#"
fn label(u: URL) string {
 return u.href+"|"+u.origin+"|"+u.protocol+"|"+u.username+"|"+u.password+"|"+u.host+"|"+u.hostname+"|"+u.port+"|"+u.pathname+"|"+u.search+"|"+u.hash;
}
fn main() string {
 return match URL("../world?q=hello world#hi",Some("https://User:Pass@EXAMPLE.com:443/a/b/")) {
  Ok(u)=>label(u),Err(e)=>"failed:"+e
 };
}
"#).await,HostValue::String("https://User:Pass@example.com/a/world?q=hello%20world#hi|https://example.com|https:|User|Pass|example.com|example.com||/a/world|?q=hello%20world|#hi".into()));
    assert_eq!(run(r#"fn main() string { try { return match URL("://bad") { Ok(u)=>"wrong",Err(e)=>"handled" }; } catch (e) { return "threw"; } }"#).await,HostValue::String("handled".into()));
    assert_eq!(run(r#"fn main() string { return match URL("https://example.com",Some("")) { Ok(u)=>"wrong",Err(e)=>"empty base fails" }; }"#).await,HostValue::String("empty base fails".into()));
}
#[tokio::test]
async fn linked_search_params_are_live_and_same_object() {
    assert_eq!(run(r#"
fn inspect(u: URL) string {
 const p=u.searchParams; const other=u.searchParams;
 p.sort(); other.append("a","second");
 const before=u.href;
 p.delete("a");
 return before+"|"+u.href+"|"+string(p.size)+"|"+u.toString()+"|"+u.toJSON();
}
fn main() string { return match URL("https://example.com/?a=b%20~#hash") { Ok(u)=>inspect(u),Err(e)=>e }; }
"#).await,HostValue::String("https://example.com/?a=b+%7E&a=second#hash|https://example.com/#hash|0|https://example.com/#hash|https://example.com/#hash".into()));
}
#[tokio::test]
async fn params_missing_empty_and_value_filters_stay_distinct() {
    assert_eq!(
        run(r#"
fn main() string {
 const p=URLSearchParams("a=&a=one&a=two");
 const first=string(p.get("a")); const missing=string(p.get("missing"));
 p.delete("a",Some(""));
 const filtered=p.has("a",Some("two"));
 p.set("a","last");p.append("plus","a+b");
 return first+"|"+missing+"|"+string(filtered)+"|"+p.toString();
}
"#)
        .await,
        HostValue::String("Some(\"\")|None|true|a=last&plus=a%2Bb".into())
    );
    assert_eq!(
        run(r#"fn main() string { return URLSearchParams().toString(); }"#).await,
        HostValue::String("".into())
    );
}
#[tokio::test]
async fn params_collection_methods_return_typed_snapshots() {
    assert_eq!(
        run(r#"
fn main() string {
 const p=URLSearchParams("a=1&a=2&b=3");
 let output="";
 for (const pair of p.entries()) { const [key,value]=pair; output=output+key+":"+value+";"; }
 p.append("c","4");
 return output;
}
"#)
        .await,
        HostValue::String("a:1;a:2;b:3;".into())
    );
    assert_eq!(
        run(r#"fn main() Array<string> { return URLSearchParams("a=1&a=2&b=3").getAll("a"); }"#)
            .await,
        HostValue::List(vec![
            HostValue::String("1".into()),
            HostValue::String("2".into())
        ])
    );
    assert_eq!(
        run(r#"fn main() Array<string> { return URLSearchParams("a=1&a=2&b=3").keys(); }"#).await,
        HostValue::List(vec![
            HostValue::String("a".into()),
            HostValue::String("a".into()),
            HostValue::String("b".into())
        ])
    );
    assert_eq!(
        run(r#"fn main() Array<string> { return URLSearchParams("a=1&a=2&b=3").values(); }"#).await,
        HostValue::List(vec![
            HostValue::String("1".into()),
            HostValue::String("2".into()),
            HostValue::String("3".into())
        ])
    );
}
#[test]
fn wrong_types_forged_handles_and_readonly_properties_are_compile_errors() {
    for source in [
        r#"fn main() { URL(7); }"#,
        r#"fn main() { URLSearchParams().append("a",7); }"#,
        r#"fn main() { URLSearchParams().get(7); }"#,
        r#"fn main() { const p=URLSearchParams();p.size=3; }"#,
        r#"fn main() { const p=URLSearchParams();p.href; }"#,
        r#"fn main() { const p=URLSearchParams();p.delete("a",Some(7)); }"#,
        r#"fn wrong(u: URL) { } fn main() { wrong({href:"fake"}); }"#,
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
}
#[tokio::test]
async fn global_aliases_and_lexical_shadowing_preserve_ordinary_source_rules() {
    assert_eq!(
        run(
            r#"const Params=URLSearchParams; fn main() string { return Params("x=1").toString(); }"#
        )
        .await,
        HostValue::String("x=1".into())
    );
    assert_eq!(run(r#"fn main() string { const URL=fn(value: string) string { return "local:"+value; }; return URL("value"); }"#).await,HostValue::String("local:value".into()));
}

#[tokio::test]
async fn imported_url_functions_and_constructor_aliases_keep_native_types() {
    use std::fs;
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("address.ds"), r#"
export const Params = URLSearchParams;
export fn address(input: string) Result<URL,string> { return URL(input,Some("https://example.com/base/")); }
"#).unwrap();
    fs::write(
        dir.path().join("barrel.ds"),
        r#"export { Params, address } from "./address.ds";"#,
    )
    .unwrap();
    let entry = dir.path().join("main.ds");
    fs::write(&entry,r#"
import { Params, address } from "./barrel.ds";
fn main() string {
    const params=Params("empty=");
    return string(params.get("empty"))+":"+match address("../tour") { Ok(value)=>value.href,Err(error)=>error };
}
"#).unwrap();
    let hosts = hosts();
    let program = compiler::compile_file(&entry, &hosts, Some("main")).unwrap();
    let program = serde_json::from_str(&serde_json::to_string(&program).unwrap()).unwrap();
    drop(dir);
    let mut vm = Vm::new(program, hosts).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String("Some(\"\"):https://example.com/tour".into())
    );
    assert_eq!(vm.stats().live, 0);
}
