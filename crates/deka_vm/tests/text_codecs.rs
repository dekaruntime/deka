#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    text_codec::register(&mut hosts).unwrap();
    hosts
        .register(HostOp::new(
            "raw",
            vec![],
            HostType::Bytes,
            false,
            None,
            |_| HostReply::Ready(Ok(HostValue::Bytes(vec![0xe2, 0x82, 0xac, 0xff]))),
        ))
        .unwrap();
    hosts
}
async fn run(source: &str) -> HostValue {
    let hosts = hosts();
    let program = compiler::compile(source, &hosts).unwrap();
    let program: Program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let value = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    value
}
#[tokio::test]
async fn utf8_bytes_and_unicode_roundtrip_use_opaque_method_dispatch() {
    assert_eq!(
        run(r#"fn main() string {
const make = TextEncoder; const encoder = make(); const bytes = encoder.encode("Aé🙂");
const makeDecoder = TextDecoder; const decoder = unwrap(makeDecoder()) or {return "constructor failed";};
const back = unwrap(decoder.decode(bytes)) or {return "decode failed";};
return string(bytes.length)+":"+string(bytes[1])+":"+back+":"+string(encoder.encode().length);
}"#)
        .await,
        HostValue::String("7:195:Aé🙂:0".into())
    );
}
#[tokio::test]
async fn unsupported_labels_and_fatal_decode_are_results_not_throws() {
    assert_eq!(
        run(r#"import {raw} from "vm:host";
fn main() string {
const bad = match TextDecoder("not-an-encoding") {Ok(d)=>"accepted",Err(e)=>"unsupported"};
const strict = unwrap(TextDecoder("utf-8",{fatal:true,ignoreBOM:false})) or {return "constructor";};
const failed = match strict.decode(raw()) {Ok(s)=>"accepted",Err(e)=>"malformed"};
const reset = unwrap(strict.decode(TextEncoder().encode("okay"))) or {return "not reset";};
const lossy = unwrap(TextDecoder()) or {return "lossy constructor";};
const replaced = unwrap(lossy.decode(raw())) or {return "lossy decode";};
return bad+":"+failed+":"+reset+":"+replaced;
}"#)
        .await,
        HostValue::String("unsupported:malformed:okay:€�".into())
    );
}
#[tokio::test]
async fn streaming_aliases_retain_decoder_state_and_final_decode_resets_it() {
    let mut hosts = hosts();
    for (name, bytes) in [
        ("first", vec![0xef, 0xbb]),
        ("second", vec![0xbf, 0xf0, 0x9f]),
        ("third", vec![0x99, 0x82]),
    ] {
        hosts
            .register(HostOp::new(
                name,
                vec![],
                HostType::Bytes,
                false,
                None,
                move |_| HostReply::Ready(Ok(HostValue::Bytes(bytes.clone()))),
            ))
            .unwrap();
    }
    let source = r#"import {first,second,third} from "vm:host";
fn main() string {const d=unwrap(TextDecoder()) or {return "constructor";};const shared=d;
const a=unwrap(d.decode(first(),{stream:true})) or {return "a";};
const b=unwrap(shared.decode(second(),{stream:true})) or {return "b";};
const c=unwrap(d.decode(third())) or {return "c";};
const after=unwrap(shared.decode(TextEncoder().encode("again"))) or {return "after";};return a+b+c+after;}"#;
    let program = compiler::compile(source, &hosts).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    assert_eq!(vm.run().await.unwrap(), HostValue::String("🙂again".into()));
}
#[tokio::test]
async fn legacy_labels_bom_options_and_large_output_are_real_decodes() {
    assert_eq!(run(r#"import {raw} from "vm:host";
fn main() string {const keep=unwrap(TextDecoder("utf8",{fatal:false,ignoreBOM:true})) or {return "keep";};
const bom=unwrap(keep.decode(TextEncoder().encode("﻿ok"))) or {return "bom";};
const latin=unwrap(TextDecoder("latin1")) or {return "latin";};
const text=unwrap(latin.decode(raw())) or {return "text";};return bom+":"+text;}"#).await,HostValue::String("\u{feff}ok:â‚¬ÿ".into()));
    let source = format!(
        "fn main() number {{const d=unwrap(TextDecoder()) or {{return -1;}};const s=unwrap(d.decode(TextEncoder().encode(\"{}\"))) or {{return -2;}};return s.length;}}",
        "é".repeat(9000)
    );
    assert_eq!(run(&source).await, HostValue::Number(9000.));
}
#[test]
fn bad_calls_and_forged_handles_are_compile_errors() {
    for source in [
        "fn main() {TextEncoder().encode(7);}",
        "fn main() {TextEncoder(7);}",
        "fn main() {TextDecoder(true);}",
        "fn main() {TextDecoder(\"utf-8\",{fatal:7,ignoreBOM:false});}",
        "fn main() {const d=unwrap(TextDecoder()) or {return;};d.decode(\"text\");}",
        "fn main() {const x:TextEncoder={};x.encode(\"hi\");}",
        "fn main() {TextEncoder().missing();}",
        "struct TextEncoder {x:number;} fn main() {const e=TextEncoder{x:1};e.encode(\"hi\");}",
        "fn main() {let e=TextEncoder();e.encoding=\"latin1\";}",
        "fn main() {TextEncoder().encode<number>(\"hi\");}",
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
}
#[tokio::test]
async fn local_constructor_binding_shadows_the_global() {
    assert_eq!(
        run("fn main() number {const TextEncoder=fn() number {return 42;};return TextEncoder();}")
            .await,
        HostValue::Number(42.)
    );
}
#[tokio::test]
async fn codecs_survive_module_factory_aliases_and_type_annotations() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("factory.ds"),
        "export fn make() {return TextEncoder();}",
    )
    .unwrap();
    let entry = dir.path().join("main.ds");
    std::fs::write(
        &entry,
        r#"import {make} from "./factory.ds";
fn use(e:TextEncoder) bytes {return e.encode("hello");}
fn main() number {const e=make();return use(e).length;}"#,
    )
    .unwrap();
    let hosts = hosts();
    let program = compiler::compile_file(&entry, &hosts, Some("main")).unwrap();
    assert_eq!(
        Vm::new(program, hosts).unwrap().run().await.unwrap(),
        HostValue::Number(5.)
    );
}

#[tokio::test]
async fn readonly_properties_have_checked_types_and_real_host_values() {
    assert_eq!(run(r#"fn main() string {const e=TextEncoder();const d=unwrap(TextDecoder(" LATIN1 ",{fatal:true,ignoreBOM:true})) or {return "constructor";}; return e.encoding+":"+d.encoding+":"+string(d.fatal)+":"+string(d.ignoreBOM);}"#).await,HostValue::String("utf-8:windows-1252:true:true".into()));
}
#[tokio::test]
async fn host_methods_retain_aliases_then_release_the_rust_resource() {
    use std::{cell::Cell, rc::Rc};
    struct Resource(Rc<Cell<usize>>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let drops = Rc::new(Cell::new(0));
    let count = drops.clone();
    let mut hosts = Hosts::default();
    hosts
        .register(
            HostOp::new(
                "Resource",
                vec![],
                HostType::Handle("Resource".into()),
                false,
                None,
                move |_| {
                    HostReply::Ready(Ok(HostValue::Handle(HostHandle::new(
                        "Resource",
                        Resource(count.clone()),
                    ))))
                },
            )
            .with_global_binding(),
        )
        .unwrap();
    hosts
        .register(
            HostOp::new(
                "alive",
                vec![HostType::Handle("Resource".into())],
                HostType::Bool,
                false,
                None,
                |args| {
                    let HostValue::Handle(handle) = &args[0] else {
                        unreachable!()
                    };
                    HostReply::Ready(Ok(HostValue::Bool(
                        handle.downcast_ref::<Resource>().is_some(),
                    )))
                },
            )
            .with_receiver_method("Resource", "alive"),
        )
        .unwrap();
    let program = compiler::compile(
        "fn main() boolean {const a=Resource();const b=a;return b.alive();}",
        &hosts,
    )
    .unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    assert_eq!(vm.run().await.unwrap(), HostValue::Bool(true));
    assert_eq!(drops.get(), 1);
    assert_eq!(vm.stats().live, 0);
}

#[test]
fn shared_host_handles_preserve_identity_without_retaining_a_dead_resource() {
    use std::rc::Rc;
    let resource = Rc::new(String::from("node"));
    let weak = Rc::downgrade(&resource);
    let first = HostHandle::from_shared("TestNode", resource.clone());
    let second = HostHandle::from_shared("TestNode", resource.clone());
    assert_eq!(first, second);
    assert_eq!(first.downcast_ref::<String>(), Some(resource.as_ref()));
    assert_ne!(first, HostHandle::new("TestNode", String::from("node")));
    drop(resource);
    drop(first);
    assert!(weak.upgrade().is_some());
    drop(second);
    assert!(weak.upgrade().is_none());
}

#[tokio::test]
async fn imported_codec_factories_and_promise_aliases_share_export_inference() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("factory.ds"),
        r#"export fn encoder() { return TextEncoder(); }
export fn decoder() { return TextDecoder(); }
export fn charset() { return TextEncoder().encoding; }
export const P = Promise;
export const all = P.all;
export const race = P.race;"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("barrel.ds"),
        "export {encoder, decoder, charset, all, race} from \"./factory.ds\";",
    )
    .unwrap();
    let entry = dir.path().join("main.ds");
    std::fs::write(
        &entry,
        r#"import {encoder, decoder, charset, all, race} from "./barrel.ds";
async fn encoded(text: string) Promise<bytes> { return encoder().encode(text); }
fn decoded(d: TextDecoder, data: bytes) string {
    const text = unwrap(d.decode(data)) or { return "decode failed"; };
    return text;
}
async fn main() Promise<string> {
    const d = unwrap(decoder()) or { return "constructor failed"; };
    const values = await all([encoded("hi"), encoded("🙂")]);
    const winner = await race([encoded("one"), encoded("two")]);
    const second = values.has(1) ? values[1] : encoder().encode("missing");
    const text = unwrap(JSON.parse<string>(JSON.stringify(decoded(d, second)))) or { return "json failed"; };
    return charset() + ":" + text + ":" + decoded(d, winner);
}"#,
    )
    .unwrap();
    let catalog = hosts();
    let program = compiler::compile_file(&entry, &catalog, Some("main")).unwrap();
    let program: Program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, catalog).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String("utf-8:🙂:one".into())
    );
    assert_eq!(vm.stats().live, 0);
    // Imported inferred returns must remain concrete, not Error/Infer values
    // that silently accept an incompatible annotated return.
    std::fs::write(
        &entry,
        r#"import {charset} from "./barrel.ds";
fn main() number { return charset(); }"#,
    )
    .unwrap();
    assert!(compiler::compile_file(&entry, &hosts(), Some("main")).is_err());
}
