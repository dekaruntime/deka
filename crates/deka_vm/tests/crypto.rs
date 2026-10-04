#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    text_codec::register(&mut hosts).unwrap();
    crypto::register(&mut hosts).unwrap();
    hosts
}
async fn run(source: &str, hosts: Hosts) -> HostValue {
    let program = compiler::compile(source, &hosts).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let value = tokio::time::timeout(std::time::Duration::from_secs(10), vm.run())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vm.stats().live, 0);
    value
}
#[tokio::test]
async fn digest_returns_known_bytes_through_the_typed_global_and_worker() {
    assert_eq!(run(r#"async fn main() Promise<bytes> {
        const subtle=crypto.subtle;
        const hash=unwrap(await subtle.digest("SHA-256",TextEncoder().encode("abc"))) or{return TextEncoder().encode("failed");}; return hash;
    }"#,hosts()).await,HostValue::Bytes(vec![0xba,0x78,0x16,0xbf,0x8f,0x01,0xcf,0xea,0x41,0x41,0x40,0xde,0x5d,0xae,0x22,0x23,0xb0,0x03,0x61,0xa3,0x96,0x17,0x7a,0x9c,0xb4,0x10,0xff,0x61,0xf2,0x00,0x15,0xad]));
}
#[tokio::test]
async fn digest_errors_are_results_and_do_not_enter_catch() {
    assert_eq!(run(r#"async fn main() Promise<string> {
        try {return match await crypto.subtle.digest("md5",TextEncoder().encode("abc")){Ok(data)=>"bad",Err(error)=>error};}
        catch(error){return "threw";}
    }"#,hosts()).await,HostValue::String("unknown digest algorithm 'md5'".into()));
}
#[tokio::test]
async fn all_ported_digest_algorithms_join_with_concrete_bytes() {
    assert_eq!(run(r#"async fn main() Promise<string> {
        const c=crypto;const input=TextEncoder().encode("abc");
        const hashes=await Promise.all([c.subtle.digest("SHA-256",input),c.subtle.digest("SHA-384",input),c.subtle.digest("SHA-512",input),c.subtle.digest("SHA3-256",input),c.subtle.digest("BLAKE3",input)]);
        let sizes="";for(const result of hashes){const hash=unwrap(result) or{return "failed";};sizes=sizes+string(hash.length)+":";}
        return sizes;
    }"#,hosts()).await,HostValue::String("32:48:64:32:32:".into()));
}
#[tokio::test]
async fn globals_and_subtle_properties_are_canonical_rust_handles() {
    let mut h = hosts();
    for (name, kind) in [("sameCrypto", "Crypto"), ("sameSubtle", "SubtleCrypto")] {
        let ty = HostType::Handle(kind.into());
        h.register(
            HostOp::new(name, vec![ty.clone(), ty], HostType::Bool, false, |args| {
                HostReply::Ready(Ok(HostValue::Bool(args[0] == args[1])))
            })
            .with_global_binding(),
        )
        .unwrap();
    }
    assert_eq!(run("fn main() boolean {const c=crypto;const s=c.subtle;return sameCrypto(c,crypto)&&sameSubtle(s,crypto.subtle);}",h).await,HostValue::Bool(true));
    assert_eq!(
        run(
            "fn main() number {const crypto={value:42};return crypto.value;}",
            hosts()
        )
        .await,
        HostValue::Number(42.)
    );
}
#[tokio::test]
async fn random_values_keep_input_immutable_and_uuid_has_version_four_bits() {
    assert_eq!(
        run(
            r#"fn main() string {
        const input=TextEncoder().encode("abc");
        const random=unwrap(crypto.getRandomValues(input)) or{return "random failed";};
        const original=unwrap(TextDecoder()) or{return "decoder failed";};
        const text=unwrap(original.decode(input)) or{return "decode failed";};return string(random.length)+":"+text;
    }"#,
            hosts()
        )
        .await,
        HostValue::String("3:abc".into())
    );
    let HostValue::String(value) = run(
        r#"fn main() string {const id=unwrap(crypto.randomUUID()) or{return "failed";};return id;}"#,
        hosts(),
    )
    .await
    else {
        panic!("UUID must be a string")
    };
    assert_eq!(value.len(), 36);
    assert_eq!(&value[14..15], "4");
    assert!(matches!(value.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
    for (i, b) in value.bytes().enumerate() {
        if [8, 13, 18, 23].contains(&i) {
            assert_eq!(b, b'-');
        } else {
            assert!(b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        }
    }
}
#[test]
fn incorrect_calls_forged_handles_and_readonly_assignment_fail_to_compile() {
    for source in [
        "fn main(){crypto();}",
        "fn main(){crypto.nonexistent();}",
        "fn main(){crypto.randomUUID(1);}",
        "fn main(){crypto.getRandomValues([1,2]);}",
        "fn main(){crypto.subtle.digest(1,TextEncoder().encode());}",
        "fn main(){crypto.subtle.digest(\"SHA-256\",\"abc\");}",
        "fn main(){const c:Crypto={};c.randomUUID();}",
        "fn main(){const s:SubtleCrypto={};s.digest(\"SHA-256\",TextEncoder().encode());}",
        "fn main(){crypto.subtle=crypto.subtle;}",
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
}
#[tokio::test]
async fn exported_global_values_survive_source_deletion() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("crypto.ds"),
        "export const c=crypto;export const subtle=c.subtle;",
    )
    .unwrap();
    let entry = dir.path().join("main.ds");
    std::fs::write(&entry,r#"import {c,subtle} from "./crypto.ds";
    async fn main() Promise<number> {const hash=unwrap(await subtle.digest("SHA-256",TextEncoder().encode("abc"))) or{return -1;};const id=unwrap(c.randomUUID()) or{return -2;};return hash[0]+id.length;}"#).unwrap();
    let h = hosts();
    let program = compiler::compile_file(&entry, &h, Some("main")).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    drop(dir);
    let mut vm = Vm::new(program, h).unwrap();
    assert_eq!(vm.run().await.unwrap(), HostValue::Number(222.));
    assert_eq!(vm.stats().live, 0);
}
#[test]
fn ambient_value_catalog_rejects_collisions_and_non_opaque_getters() {
    fn getter() -> HostOp {
        HostOp::new(
            "getter",
            vec![],
            HostType::Handle("Resource".into()),
            false,
            |_| HostReply::Ready(Ok(HostValue::Handle(HostHandle::new("Resource", ())))),
        )
        .with_global_value_binding("ambient")
    }
    fn function() -> HostOp {
        HostOp::new("ambient", vec![], HostType::Unit, false, |_| {
            HostReply::Ready(Ok(HostValue::Unit))
        })
        .with_global_binding()
    }
    for value_first in [true, false] {
        let mut h = Hosts::default();
        if value_first {
            h.register(getter()).unwrap();
            assert!(h.register(function()).is_err());
        } else {
            h.register(function()).unwrap();
            assert!(h.register(getter()).is_err());
        }
    }
    for change in [0, 1, 2, 3] {
        let mut op = getter();
        match change {
            0 => op.asynchronous = true,
            1 => op.args.push(HostType::Number),
            2 => op.result = HostType::Number,
            _ => op.result_channel = true,
        };
        assert!(Hosts::default().register(op).is_err());
    }
}

#[tokio::test]
async fn byte_quotas_fail_as_results_before_unbounded_work() {
    let mut h = hosts();
    for (name, size) in [("tooRandom", 65537), ("tooHash", 16 * 1024 * 1024 + 1)] {
        h.register(
            HostOp::new(name, vec![], HostType::Bytes, false, move |_| {
                HostReply::Ready(Ok(HostValue::Bytes(vec![0; size])))
            })
            .with_global_binding(),
        )
        .unwrap();
    }
    assert_eq!(run(r#"async fn main() Promise<string> {
        const random=match crypto.getRandomValues(tooRandom()){Ok(bytes)=>"bad",Err(error)=>error};
        const hash=match await crypto.subtle.digest("SHA-256",tooHash()){Ok(bytes)=>"bad",Err(error)=>error};
        return random+":"+hash;
    }"#,h).await,HostValue::String("random byte quota exceeds 65536 bytes:input too large".into()));
}
