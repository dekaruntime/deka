#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    bytes::register(&mut hosts).unwrap();
    hosts
}
async fn run(source: &str) -> HostValue {
    let hosts = hosts();
    let program = compiler::compile(source, &hosts).unwrap();
    let program: Program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let result = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    result
}
#[tokio::test]
async fn unicode_binary_roundtrip_and_copying_slice_concat() {
    assert_eq!(run(r#"import {from_string,to_string,to_hex,len,slice,concat} from "bytes";
fn main() string {
const value=from_string("Aé🙂");
const shared=value;
const middle=slice(value,1,3);
const joined=concat(middle,slice(value,-4));
const text=unwrap(to_string(joined)) or {return "decode failed";};
return text+":"+to_hex(shared)+":"+string(len(value))+":"+to_hex(slice(value,5,2))+":"+to_hex(slice(value,-99,99));
}"#).await, HostValue::String("é🙂:41c3a9f09f9982:7::41c3a9f09f9982".into()));
}
#[tokio::test]
async fn arrays_and_indices_validate_without_coercion() {
    assert_eq!(run(r#"import {from_array,get,to_hex} from "bytes";
fn main() string {
const value=unwrap(from_array([0,128,255])) or {return "array failed";};
const high=unwrap(get(value,2)) or {return "index failed";};
const negative=match from_array([-1]) {Some(v)=>"wrong",None=>"negative"};
const fraction=match from_array([1.5]) {Some(v)=>"wrong",None=>"fraction"};
const overflow=match from_array([256]) {Some(v)=>"wrong",None=>"overflow"};
const bounds=match get(value,3) {Some(v)=>"wrong",None=>"bounds"};
const fractional=match get(value,0.5) {Some(v)=>"wrong",None=>"fractional"};
return to_hex(value)+":"+string(high)+":"+negative+":"+fraction+":"+overflow+":"+bounds+":"+fractional;
}"#).await, HostValue::String("0080ff:255:negative:fraction:overflow:bounds:fractional".into()));
}
#[tokio::test]
async fn strict_binary_decoding_is_data_and_preserves_bom() {
    assert_eq!(
        run(
            r#"import {from_hex,from_base64,to_base64,to_string} from "bytes";
fn main() string {
const raw=unwrap(from_hex("efbbbf41")) or {return "hex failed";};
const decoded=unwrap(to_string(raw)) or {return "UTF8 failed";};
const invalid=unwrap(from_hex("ff")) or {return "invalid hex";};
const utf8=match to_string(invalid) {Ok(v)=>"wrong",Err(e)=>"utf8"};
const hex=match from_hex("6g") {Some(v)=>"wrong",None=>"hex"};
const base64=match from_base64("Zg") {Some(v)=>"wrong",None=>"base64"};
const back=unwrap(from_base64("AP8=")) or {return "base64 failed";};
return decoded+":"+utf8+":"+hex+":"+base64+":"+to_base64(back);
}"#
        )
        .await,
        HostValue::String("\u{feff}A:utf8:hex:base64:AP8=".into())
    );
}
#[test]
fn text_and_bytes_never_implicitly_convert_and_results_are_checked() {
    for source in [
        r#"import {len} from "bytes"; fn main() {len("text");}"#,
        r#"import {from_array} from "bytes"; fn main() {from_array([true]);}"#,
        r#"import {from_array,len} from "bytes"; fn main() {len(from_array([255]));}"#,
        r#"import {from_string,to_string} from "bytes"; fn main() string {return to_string(from_string("text"));}"#,
        r#"import {to_hex} from "io"; fn main() {to_hex("text");}"#,
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
}
#[tokio::test]
async fn imports_aliases_exports_and_bytecode_survive_source_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let module = dir.path().join("value.ds");
    let entry = dir.path().join("main.ds");
    std::fs::write(
        &module,
        r#"import {from_array,from_string} from "bytes";
export const original=from_string("native");
export fn make() {return from_array([0,255]);}"#,
    )
    .unwrap();
    std::fs::write(&entry,r#"import {to_hex as hex,concat} from "bytes";
import {original,make} from "./value.ds";
fn main() string {const suffix=unwrap(make()) or {return "failed";};return hex(concat(original,suffix));}"#).unwrap();
    let hosts = hosts();
    let program = compiler::compile_file(&entry, &hosts, Some("main")).unwrap();
    let program: Program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    std::fs::remove_file(&entry).unwrap();
    std::fs::remove_file(&module).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String("6e617469766500ff".into())
    );
    assert_eq!(vm.stats().live, 0);
}

#[tokio::test]
async fn invalid_utf8_is_err_never_lossy_or_throw() {
    for hex in ["ff", "c0af", "eda080", "f4908080", "e282"] {
        let source = format!(
            r#"import {{from_hex,to_string}} from "bytes";
fn main() boolean {{
const value=unwrap(from_hex("{hex}")) or {{return false;}};
try {{return match to_string(value) {{Ok(s)=>false,Err(e)=>true}};}}
catch(e) {{return false;}}
}}"#
        );
        assert_eq!(run(&source).await, HostValue::Bool(true), "{hex}");
    }
}

#[tokio::test]
async fn empty_bytes_are_valid_some_and_successful_utf8() {
    assert_eq!(
        run(
            r#"import {from_array,from_hex,from_base64,to_string,len,concat} from "bytes";
fn main() string {
const a=unwrap(from_array([])) or {return "array failed";};
const b=unwrap(from_hex("")) or {return "hex failed";};
const c=unwrap(from_base64("")) or {return "base64 failed";};
const text=unwrap(to_string(concat(a,concat(b,c)))) or {return "text failed";};
return string(len(a))+":"+string(len(b))+":"+string(len(c))+":"+text;
}"#
        )
        .await,
        HostValue::String("0:0:0:".into())
    );
}
