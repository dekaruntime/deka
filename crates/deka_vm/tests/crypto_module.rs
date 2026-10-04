#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
fn hosts() -> Hosts {
    let mut h = Hosts::default();
    builtin_crypto::register(&mut h).unwrap();
    bytes::register(&mut h).unwrap();
    crypto::register(&mut h).unwrap();
    text_codec::register(&mut h).unwrap();
    h
}
async fn run(source: &str) -> HostValue {
    let h = hosts();
    let p = compiler::compile(source, &h).unwrap();
    let p = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    let value = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    value
}
#[tokio::test]
async fn all_digest_exports_are_synchronous_typed_results() {
    for (name, hex) in [
        (
            "sha256",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "sha384",
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
        ),
        (
            "sha512",
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        ),
        (
            "sha3_256",
            "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532",
        ),
        (
            "blake3",
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
        ),
    ] {
        let source = format!(
            r#"import {{{name} as hash}} from "crypto"; import {{from_string,to_hex}} from "bytes";
fn main() string {{const value=unwrap(hash(from_string("abc"))) or {{return "failed";}};return to_hex(value);}}"#
        );
        assert_eq!(run(&source).await, HostValue::String(hex.into()), "{name}");
    }
    assert_eq!(run(r#"import {digest} from "crypto";import {from_string,to_hex} from "bytes";
fn main() string {const value=unwrap(digest("SHA_256",from_string("abc"))) or {return "failed";};return to_hex(value);}"#).await,HostValue::String("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()));
}
#[tokio::test]
async fn hmac_aliases_and_constant_time_comparison_use_actual_results() {
    for (name, algorithm) in [
        ("hs256", "SHA-256"),
        ("hs384", "SHA-384"),
        ("hs512", "SHA-512"),
    ] {
        let source = format!(
            r#"import {{hmac,{name},secure_compare}} from "crypto";import {{from_string,to_hex}} from "bytes";
fn main() boolean {{const key=from_string("key");const data=from_string("The quick brown fox jumps over the lazy dog");const a=unwrap({name}(key,data)) or {{return false;}};const b=unwrap(hmac("{algorithm}",key,data)) or {{return false;}};const equal=unwrap(secure_compare(a,b)) or {{return false;}};return equal;}}"#
        );
        assert_eq!(run(&source).await, HostValue::Bool(true));
    }
    assert_eq!(run(r#"import {hs256} from "crypto";import {from_string,to_hex} from "bytes";
fn main() string {const result=unwrap(hs256(from_string("key"),from_string("The quick brown fox jumps over the lazy dog"))) or {return "failed";};return to_hex(result);}"#).await,HostValue::String("f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8".into()));
    assert_eq!(run(r#"import {secure_compare} from "crypto";import {from_string} from "bytes";
fn main() boolean {const equal=unwrap(secure_compare(from_string("abc"),from_string("abd"))) or {return true;};return equal;}"#).await,HostValue::Bool(false));
}
#[tokio::test]
async fn aes_gcm_authenticates_the_ciphertext_and_aad() {
    assert_eq!(run(r#"import {aes_256_gcm_encrypt,aes_256_gcm_decrypt} from "crypto";import {from_hex,from_string,to_string,to_hex} from "bytes";
fn main() string {
const key=unwrap(from_hex("0000000000000000000000000000000000000000000000000000000000000000")) or {return "key";};
const nonce=unwrap(from_hex("000000000000000000000000")) or {return "nonce";};
const empty=from_string("");const cipher=unwrap(aes_256_gcm_encrypt(key,nonce,empty,empty)) or {return "encrypt";};
const known=to_hex(cipher);
const encrypted=unwrap(aes_256_gcm_encrypt(key,nonce,from_string("native"),from_string("context"))) or {return "encrypt";};
const plain=unwrap(aes_256_gcm_decrypt(key,nonce,encrypted,from_string("context"))) or {return "decrypt";};
const text=unwrap(to_string(plain)) or {return "UTF8";};
const failed=match aes_256_gcm_decrypt(key,nonce,encrypted,empty) {Ok(v)=>"bad",Err(e)=>e};
return known+":"+text+":"+failed;
}"#).await,HostValue::String("530f8afbc74536b9a963b4f1c4cb738b:native:aes_decrypt_failed".into()));
}
#[tokio::test]
async fn malformed_inputs_are_err_values_not_throws() {
    assert_eq!(run(r#"import {digest,hmac,aes_256_gcm_encrypt,bcrypt_verify,random_bytes,random_hex} from "crypto";import {from_string} from "bytes";
fn main() string {const empty=from_string("");
try {
const a=match digest("unknown",empty) {Ok(v)=>"bad",Err(e)=>"digest"};
const b=match hmac("unknown",empty,empty) {Ok(v)=>"bad",Err(e)=>"hmac"};
const c=match aes_256_gcm_encrypt(empty,empty,empty,empty) {Ok(v)=>"bad",Err(e)=>e};
const d=match bcrypt_verify("password","invalid") {Ok(v)=>"bad",Err(e)=>e};
const e=match random_bytes(0) {Ok(v)=>"bad",Err(e)=>"zero"};
const f=match random_hex(1.5) {Ok(v)=>"bad",Err(e)=>"fraction"};
return a+":"+b+":"+c+":"+d+":"+e+":"+f;
} catch(e) {return "threw";}}
"#).await,HostValue::String("digest:hmac:key_length_invalid:bcrypt_verify_failed:zero:fraction".into()));
}
#[tokio::test]
async fn bcrypt_accepts_known_passwords_rejects_wrong_and_overlong() {
    let hash = "$2y$04$6ck.DhBYy8ZL.sAFGrLZtuLI1F5cSwaM5XHJRxoZFRMx1n8ZpYtJa";
    for (password, expected) in [
        ("password".to_owned(), true),
        ("wrong".to_owned(), false),
        ("x".repeat(73), false),
        ("é".repeat(37), false),
    ] {
        let source = format!(
            r#"import {{bcrypt_verify}} from "crypto";fn main() boolean {{const valid=unwrap(bcrypt_verify("{password}","{hash}")) or {{return false;}};return valid;}}"#
        );
        assert_eq!(run(&source).await, HostValue::Bool(expected));
    }
}
#[tokio::test]
async fn random_exports_allocate_immutable_bytes_and_uuid_v4() {
    assert_eq!(run(r#"import {random_bytes,random_hex,uuid_v4} from "crypto";import {len} from "bytes";
fn main() string {const a=unwrap(random_bytes(16)) or {return "failed";};const b=unwrap(random_hex(8)) or {return "failed";};const c=unwrap(uuid_v4()) or {return "failed";};return string(len(a))+":"+string(b.length)+":"+string(c.length);}"#).await,HostValue::String("16:16:36".into()));
}
#[test]
fn wrong_types_and_unhandled_results_are_compile_errors() {
    for source in [
        r#"import {sha256} from "crypto";fn main(){sha256("abc");}"#,
        r#"import {sha256} from "crypto";import {from_string,len} from "bytes";fn main(){len(sha256(from_string("abc")));}"#,
        r#"import {bcrypt_verify} from "crypto";fn main(){bcrypt_verify(3,"hash");}"#,
        r#"import {sha256} from "bytes";fn main(){sha256;}"#,
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
}
#[tokio::test]
async fn crypto_barrels_and_first_class_callbacks_work_after_source_deletion() {
    let d = tempfile::tempdir().unwrap();
    let entry = d.path().join("main.ds");
    let barrel = d.path().join("crypto.ds");
    std::fs::write(&barrel, r#"export {sha256 as hash} from "crypto";"#).unwrap();
    std::fs::write(&entry,r#"import {hash} from "./crypto.ds";import {from_string,to_hex} from "bytes";fn main() string {const callback=hash;const value=unwrap(callback(from_string("abc"))) or {return "failed";};return to_hex(value);}"#).unwrap();
    let h = hosts();
    let p = compiler::compile_file(&entry, &h, Some("main")).unwrap();
    let p = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    std::fs::remove_file(entry).unwrap();
    std::fs::remove_file(barrel).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()
        )
    );
    assert_eq!(vm.stats().live, 0);
}
