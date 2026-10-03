#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    text_codec::register(&mut hosts).unwrap();
    blob::register(&mut hosts).unwrap();
    hosts
}
async fn run(source: &str) -> HostValue {
    let hosts = hosts();
    let program = compiler::compile(source, &hosts).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let output = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    output
}
#[tokio::test]
async fn blob_parts_slice_and_repeatable_readers_use_the_same_immutable_snapshot() {
    assert_eq!(run(r#"async fn main() Promise<string>{
        const e=TextEncoder();const b=unwrap(Blob([e.encode("abc"),e.encode("def")])) or{return "blob";};const shared=b;
        const slice=unwrap(shared.slice(-4,-1,"TEXT/PLAIN")) or{return "slice";};
        const text=unwrap(await slice.text()) or{return "text";};const again=unwrap(await shared.text()) or{return "again";};
        const first=unwrap(await b.bytes()) or{return "bytes";};const second=unwrap(await b.arrayBuffer()) or{return "buffer";};
        return text+":"+slice.type+":"+again+":"+string(b.size)+":"+string(first[0])+":"+string(second.length);
    }"#).await,HostValue::String("cde:text/plain:abcdef:6:97:6".into()));
}
#[tokio::test]
async fn file_is_named_immutable_data_and_slice_returns_a_blob() {
    assert_eq!(
        run(r#"async fn main() Promise<string>{
        const file=unwrap(File([TextEncoder().encode("notes")],"notes.txt")) or{return "file";};
        const blob=unwrap(file.slice(1,4,"TEXT/PLAIN")) or{return "slice";};
        const text=unwrap(await blob.text()) or{return "text";};
        return file.name+":"+string(file.size)+":"+string(file.lastModified>0)+":"+text;
    }"#)
        .await,
        HostValue::String("notes.txt:5:true:ote".into())
    );
}
#[test]
fn wrong_parts_names_ranges_and_readonly_fields_are_type_errors() {
    for source in [
        "fn main(){Blob([\"text\"]);}",
        "fn main(){File([],7);}",
        "fn main(){const b=unwrap(Blob()) or{return;};b.size=3;}",
        "fn main(){const b=unwrap(Blob()) or{return;};b.slice(\"a\");}",
        "fn main(){const b:Blob={};b.text();}",
        "fn main(){const f=unwrap(File([],\"name\")) or{return;};f.name=\"changed\";}",
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
}

#[tokio::test]
async fn empty_defaults_and_invalid_slice_numbers_return_results() {
    assert_eq!(
        run(r#"async fn main() Promise<string>{
        const b=unwrap(Blob()) or{return "blob";};const empty=unwrap(b.slice()) or{return "slice";};
        const text=unwrap(await empty.text()) or{return "text";};
        const error=match b.slice(0.5){Ok(value)=>"bad",Err(error)=>error};
        return string(empty.size)+":"+text+":"+error;
    }"#)
        .await,
        HostValue::String("0::expected a whole safe integer".into())
    );
}
#[tokio::test]
async fn malformed_utf8_text_repeats_without_consuming_or_mutating_raw_bytes() {
    let mut h = hosts();
    h.register(
        HostOp::new("raw", vec![], HostType::Bytes, false, |_| {
            HostReply::Ready(Ok(HostValue::Bytes(vec![0xef, 0xbb, 0xbf, b'A', 0xff])))
        })
        .with_global_binding(),
    )
    .unwrap();
    let source = r#"async fn main() Promise<string>{const b=unwrap(Blob([raw()])) or{return "blob";};
        const a=unwrap(await b.text()) or{return "a";};const c=unwrap(await b.text()) or{return "c";};
        const bytes=unwrap(await b.bytes()) or{return "bytes";};return a+":"+c+":"+string(bytes[0])+":"+string(bytes.length);}"#;
    let mut vm = Vm::new(compiler::compile(source, &h).unwrap(), h).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String("A�:A�:239:5".into())
    );
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test]
async fn receiver_arguments_preserve_opaque_identity_as_well_as_return_values() {
    let mut h = hosts();
    let ty = HostType::Handle("Blob".into());
    h.register(
        HostOp::new(
            "same",
            vec![ty.clone(), ty],
            HostType::Bool,
            false,
            |args| {
                let (HostValue::Handle(a), HostValue::Handle(b)) = (&args[0], &args[1]) else {
                    unreachable!()
                };
                HostReply::Ready(Ok(HostValue::Bool(
                    a.downcast_ref::<blob::BlobObject>().unwrap().as_bytes()
                        == b.downcast_ref::<blob::BlobObject>().unwrap().as_bytes(),
                )))
            },
        )
        .with_receiver_method("Blob", "same"),
    )
    .unwrap();
    let source = r#"fn main() boolean{const a=unwrap(Blob([TextEncoder().encode("a")])) or{return false;};const b=unwrap(Blob([TextEncoder().encode("a")])) or{return false;};const c=unwrap(Blob([TextEncoder().encode("b")])) or{return false;};return a.same(b)&&!a.same(c);}"#;
    let mut vm = Vm::new(compiler::compile(source, &h).unwrap(), h).unwrap();
    assert_eq!(vm.run().await.unwrap(), HostValue::Bool(true));
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test]
async fn imported_snapshot_factories_survive_source_deletion_and_serialization() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("data.ds"),
        r#"export fn make(){return File([TextEncoder().encode("saved")],"notes.txt");}"#,
    )
    .unwrap();
    let entry = dir.path().join("main.ds");
    std::fs::write(&entry,r#"import {make} from "./data.ds";
    async fn main() Promise<string>{const file=unwrap(make()) or{return "file";};const slice=unwrap(file.slice(1,4)) or{return "slice";};const text=unwrap(await slice.text()) or{return "text";};return file.name+":"+text;}"#).unwrap();
    let h = hosts();
    let program = compiler::compile_file(&entry, &h, Some("main")).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    drop(dir);
    let mut vm = Vm::new(program, h).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String("notes.txt:ave".into())
    );
    assert_eq!(vm.stats().live, 0);
}
