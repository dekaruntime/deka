#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
fn hosts() -> Hosts {
    let mut h = Hosts::default();
    builtin_crypto::register(&mut h).unwrap();
    text_codec::register(&mut h).unwrap();
    time::register(&mut h).unwrap();
    h.register(HostOp::new(
        "bytes_sample",
        vec![],
        HostType::Bytes,
        false,
        |_| HostReply::Ready(Ok(HostValue::Bytes(vec![7]))),
    ))
    .unwrap();
    h
}
async fn execute(program: Program, hosts: Hosts) -> HostValue {
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let value = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    value
}
#[tokio::test]
async fn crypto_import_and_alias_dispatch_known_digest_bytes() {
    let h = hosts();
    let p=compiler::compile(r#"import {sha256 as hash} from "crypto";
    fn main() bytes {const value=unwrap(hash(TextEncoder().encode("abc"))) or{return TextEncoder().encode("failed");};return value;}"#,&h).unwrap();
    assert_eq!(
        execute(p, h).await,
        HostValue::Bytes(vec![
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad
        ])
    );
}
#[tokio::test]
async fn prefixed_operations_are_scoped_and_first_class() {
    let h = hosts();
    let p = compiler::compile(
        r#"import {sample as read} from "bytes";
    fn main() number {const callback=read;return callback()[0];}"#,
        &h,
    )
    .unwrap();
    assert_eq!(execute(p, h).await, HostValue::Number(7.));
}
#[test]
fn signatures_reject_wrong_arguments_and_cross_module_leaks() {
    for source in [
        r#"import {sha256} from "crypto"; fn main(){sha256("abc");}"#,
        r#"import {sha256} from "bytes"; fn main(){return sha256;}"#,
        r#"import {sleep} from "crypto"; fn main(){return sleep;}"#,
        r#"import {sample} from "io"; fn main(){return sample;}"#,
        r#"import {aesEncrypt} from "crypto"; fn main(){return aesEncrypt;}"#,
    ] {
        assert!(
            compiler::compile(source, &hosts()).is_err(),
            "accepted {source}"
        );
    }
}
#[test]
fn recognized_but_unimplemented_builtin_exports_fail_without_a_package_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.ds");
    std::fs::write(&file, r#"import {read} from "fs"; fn main(){return read;}"#).unwrap();
    let error = compiler::compile_file(&file, &hosts(), Some("main")).unwrap_err();
    assert!(error.contains("read"), "{error}");
    assert!(!error.contains("deka.json"), "{error}");
}
#[tokio::test]
async fn builtin_barrels_preserve_typed_host_dispatch_after_source_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("main.ds");
    let barrel = dir.path().join("hash.ds");
    std::fs::write(&barrel, r#"export {sha256 as hash} from "crypto";"#).unwrap();
    std::fs::write(&entry,r#"import {hash} from "./hash.ds";
    fn main() number{const data=unwrap(hash(TextEncoder().encode("abc"))) or{return 0;};return data[0];}"#).unwrap();
    let h = hosts();
    let program = compiler::compile_file(&entry, &h, Some("main")).unwrap();
    assert_eq!(compiler::source_files(&entry).unwrap().len(), 2);
    std::fs::remove_file(entry).unwrap();
    std::fs::remove_file(barrel).unwrap();
    assert_eq!(execute(program, h).await, HostValue::Number(186.));
}
