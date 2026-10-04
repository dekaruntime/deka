#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::rc::Rc;
const VECTOR: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.XbPfbIHMI6arZ3Y922BhjWgQzWXcXNrz0ogtVhfEd2o";
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    jwt::register_with_clock(&mut hosts, Rc::new(|| Ok(1516239022.))).unwrap();
    bytes::register(&mut hosts).unwrap();
    hosts
}
async fn execute(program: Program) -> HostValue {
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts()).unwrap();
    let value = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    value
}
async fn run(source: &str) -> HostValue {
    execute(compiler::compile(source, &hosts()).unwrap()).await
}
#[tokio::test]
async fn direct_named_alias_signs_complete_custom_payload_with_authoritative_vector() {
    assert_eq!(run(r#"import {sign as encode} from "jwt";import {from_string} from "bytes";
fn main() string {const token=unwrap(encode({sub:"1234567890",name:"John Doe",iat:1516239022},from_string("secret"))) or {return "failed";};return token;}"#).await,HostValue::String(VECTOR.into()));
}
#[tokio::test]
async fn top_level_struct_is_flattened_but_ordinary_json_format_is_preserved() {
    assert_eq!(run(r#"import {sign} from "jwt";import {from_string} from "bytes";import {stringify} from "json";
struct Claims {sub:string;name:string;iat:number;}
fn main() string {const claims=Claims{sub:"1234567890",name:"John Doe",iat:1516239022};const token=unwrap(sign(claims,from_string("secret"),{})) or {return "failed";};return token+":"+stringify({z:1,a:2});}"#).await,HostValue::String(format!("{VECTOR}:{{\"a\":2,\"z\":1}}")));
}
#[tokio::test]
async fn nested_claim_order_and_typed_options_are_checked_and_serialized_once() {
    let token = jwt::sign(
        r#"{"z":{"z":1,"a":2},"sub":"1234567890","iat":1516239022}"#,
        b"secret",
        r#"{"aud":"test"}"#,
        1516239022.,
    )
    .unwrap();
    assert_eq!(run(r#"import {sign} from "jwt";import {from_string} from "bytes";
fn main() string {let calls=0;const claims=fn(){calls=calls+1;return {z:{z:1,a:2},sub:"1234567890",iat:1516239022};};const key=fn(){calls=calls+1;return from_string("secret");};const options=fn(){calls=calls+1;return {aud:"test"};};const token=unwrap(sign(claims(),key(),options())) or {return "failed";};return token+":"+string(calls);}"#).await,HostValue::String(format!("{token}:3")));
}
#[tokio::test]
async fn verify_hydrates_declared_optional_fields_and_propagates_authentication_err() {
    let source = format!(
        r#"import {{verify}} from "jwt";import {{from_string}} from "bytes";
fn main() string {{const claims=unwrap(verify("{VECTOR}",from_string("secret"))) or {{return "failed";}};const absent=match claims.exp {{Some(n)=>"bad",None=>"none"}};const sub=match claims.sub {{Some(s)=>s,None=>"bad"}};const tamper=match verify("{VECTOR}",from_string("wrong"),{{}}) {{Ok(v)=>"bad",Err(e)=>e}};return sub+":"+absent+":"+tamper;}}"#
    );
    assert_eq!(
        run(&source).await,
        HostValue::String("1234567890:none:invalid jwt signature".into())
    );
    assert_eq!(run(r#"import {sign,verify} from "jwt";import {from_string} from "bytes";
fn main() string {const key=from_string("key");const token=unwrap(sign({},key,{exp:1})) or {return "sign";};try {return match verify(token,key) {Ok(v)=>"bad",Err(e)=>e};}catch(e){return "threw";}}"#).await,HostValue::String("jwt expired".into()));
}
#[test]
fn incompatible_claims_options_keys_and_dynamic_intrinsic_values_are_refused() {
    for (body, message) in [
        (
            "sign({iat:\"wrong\"},from_string(\"key\"))",
            "JWT field `iat` must be number",
        ),
        (
            "sign({sub:7},from_string(\"key\"))",
            "JWT field `sub` must be string",
        ),
        (
            "sign({},from_string(\"key\"),{exp_in:\"wrong\"})",
            "JWT field `exp_in` must be number",
        ),
        ("sign({},\"key\")", "JWT secret must be bytes"),
        (
            "sign([1],from_string(\"key\"))",
            "claims and options must be a checked record",
        ),
        (
            "sign({custom:fn(){return 1;}},from_string(\"key\"))",
            "cannot describe",
        ),
        (
            "verify(7,from_string(\"key\"))",
            "JWT token must be a string",
        ),
        ("const other=sign;", "must be called directly"),
    ] {
        let source = format!(
            "import {{sign,verify}} from \"jwt\";import {{from_string}} from \"bytes\";fn main(){{{body};}}"
        );
        let error = compiler::compile(&source, &hosts()).unwrap_err();
        assert!(error.contains(message), "{body}: {error}");
    }
}
#[tokio::test]
async fn reexported_alias_bytecode_runs_after_its_entire_source_tree_is_deleted() {
    let root = std::env::temp_dir().join(format!("deka-jwt-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("barrel.ds"),
        r#"export {sign as encode,verify as decode} from "jwt";"#,
    )
    .unwrap();
    std::fs::write(root.join("app.ds"),r#"import {encode,decode} from "./barrel.ds";import {from_string} from "bytes";
fn main() string {const token=unwrap(encode({sub:"1234567890",name:"John Doe",iat:1516239022},from_string("secret"))) or {return "sign";};const claims=unwrap(decode(token,from_string("secret"))) or {return "verify";};return match claims.sub {Some(s)=>s,None=>"none"};}"#).unwrap();
    let program = compiler::compile_file(&root.join("app.ds"), &hosts(), Some("main")).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(
        execute(program).await,
        HostValue::String("1234567890".into())
    );
}
#[tokio::test]
async fn explicit_option_claims_and_nested_nominal_custom_data_follow_shared_json_policy() {
    let expected = jwt::sign(
        r#"{"sub":null,"custom":{"Profile":{"Profile":"same-name","z":1}},"iat":1516239022}"#,
        b"key",
        "{}",
        1516239022.,
    )
    .unwrap();
    let value=run(r#"import {sign,verify} from "jwt";import {from_string} from "bytes";
struct Profile {Profile:string;z:number;}
fn main() string {const absent:Option<string> = None;const payload={sub:absent,custom:Profile{Profile:"same-name",z:1},iat:1516239022};const key=from_string("key");const token=unwrap(sign(payload,key)) or {return "sign";};const claims=unwrap(verify(token,key)) or {return "verify";};const empty=match claims.sub {Some(s)=>"bad",None=>"none"};return token+":"+empty;}"#).await;
    assert_eq!(value, HostValue::String(format!("{expected}:none")));
}

#[tokio::test]
async fn serialization_errors_are_results_and_all_arguments_evaluate_once_in_order() {
    assert_eq!(run(r#"import {sign} from "jwt";import {from_string} from "bytes";
fn main() string {let order="";const payload=fn(){order=order+"p";return {iat:1/0};};const key=fn(){order=order+"k";return from_string("key");};const options=fn(){order=order+"o";return {};};const result=sign(payload(),key(),options());return match result {Ok(token)=>"bad",Err(e)=>order+":"+e};}"#).await,HostValue::String("pko:failed to encode jwt payload".into()));
    let source = format!(
        r#"import {{verify}} from "jwt";import {{from_string}} from "bytes";
fn main() string {{let order="";const token=fn(){{order=order+"t";return "{VECTOR}";}};const key=fn(){{order=order+"k";return from_string("secret");}};const options=fn(){{order=order+"o";return {{leeway:0/0}};}};const result=verify(token(),key(),options());return match result {{Ok(claims)=>"bad",Err(e)=>order+":"+e}};}}"#
    );
    assert_eq!(
        run(&source).await,
        HostValue::String("tko:invalid jwt options".into())
    );
}

#[tokio::test]
async fn bounded_serialization_returns_err_without_running_crypto_or_clock() {
    use std::cell::Cell;
    let clock_calls = Rc::new(Cell::new(0));
    let called = clock_calls.clone();
    let mut registry = Hosts::default();
    jwt::register_with_clock(
        &mut registry,
        Rc::new(move || {
            called.set(called.get() + 1);
            Ok(1516239022.)
        }),
    )
    .unwrap();
    bytes::register(&mut registry).unwrap();
    registry
        .register(HostOp::new("huge", vec![], HostType::String, false, |_| {
            HostReply::Ready(Ok(HostValue::String("x".repeat(16 * 1024 * 1024))))
        }))
        .unwrap();
    let source = r#"import {sign,verify} from "jwt";import {from_string} from "bytes";import {huge} from "vm:host";
fn main() string {const key=from_string("key");const payload=match sign({sub:huge()},key) {Ok(token)=>"bad",Err(e)=>e};const options=match verify("invalid",key,{iss:huge()}) {Ok(claims)=>"bad",Err(e)=>e};return payload+":"+options;}"#;
    let program = compiler::compile(source, &registry).unwrap();
    let mut vm = Vm::new(program, registry).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String("failed to encode jwt payload:invalid jwt options".into())
    );
    assert_eq!(
        clock_calls.get(),
        0,
        "serialization failure must not invoke the native engine"
    );
    assert_eq!(vm.stats().live, 0);
}
