//! Native crypto engines ported from deka_host, without V8/JSON envelopes.
use crate::{HostHandle, HostOp, HostReply, HostType, HostValue, Hosts, Result};
use std::rc::Rc;
const MAX_CRYPTO_INPUT: usize = 16 * 1024 * 1024;
const MAX_RANDOM_BYTES: usize = 65536;
type Entropy = Rc<dyn Fn(&mut [u8]) -> Result<()>>;

pub fn digest(algorithm: &str, data: &[u8]) -> Result<Vec<u8>> {
    if data.len() > MAX_CRYPTO_INPUT {
        return Err("input too large".into());
    }
    let name = algorithm
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-")
        .replace(' ', "");
    use sha2::Digest;
    Ok(match name.as_str() {
        "sha256" | "sha-256" => sha2::Sha256::digest(data).to_vec(),
        "sha384" | "sha-384" => sha2::Sha384::digest(data).to_vec(),
        "sha512" | "sha-512" => sha2::Sha512::digest(data).to_vec(),
        "sha3-256" | "sha3" => sha3::Sha3_256::digest(data).to_vec(),
        "blake3" => blake3::hash(data).as_bytes().to_vec(),
        _ => return Err(format!("unknown digest algorithm '{algorithm}'")),
    })
}
pub fn fill_random(output: &mut [u8]) -> Result<()> {
    getrandom::getrandom(output).map_err(|error| format!("OS entropy unavailable: {error}"))
}
fn random_values(mut bytes: Vec<u8>, fill: &Entropy) -> Result<Vec<u8>> {
    if bytes.len() > MAX_RANDOM_BYTES {
        return Err("random byte quota exceeds 65536 bytes".into());
    }
    fill(&mut bytes)?;
    Ok(bytes)
}
fn random_uuid(fill: &Entropy) -> Result<String> {
    let mut bytes = [0; 16];
    fill(&mut bytes)?;
    Ok(uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string())
}
struct SubtleCrypto;
struct Crypto {
    subtle: HostHandle,
}
fn check_crypto(value: &HostValue) -> Result<()> {
    let HostValue::Handle(handle) = value else {
        unreachable!("checked Crypto receiver")
    };
    handle
        .downcast_ref::<Crypto>()
        .map(|_| ())
        .ok_or_else(|| "invalid Crypto resource".into())
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    register_with_entropy(hosts, Rc::new(fill_random))
}
fn register_with_entropy(hosts: &mut Hosts, fill: Entropy) -> Result<()> {
    let subtle = HostHandle::new("SubtleCrypto", SubtleCrypto);
    let crypto = HostHandle::new("Crypto", Crypto { subtle });
    let ty = HostType::Handle("Crypto".into());
    hosts.register(
        HostOp::new("__crypto_global", vec![], ty.clone(), false, move |_| {
            HostReply::Ready(Ok(HostValue::Handle(crypto.clone())))
        })
        .with_global_value_binding("crypto"),
    )?;
    let entropy = fill.clone();
    hosts.register(
        HostOp::new(
            "__crypto_uuid",
            vec![ty.clone()],
            HostType::String,
            false,
            move |args| {
                HostReply::Ready(
                    check_crypto(&args[0])
                        .and_then(|_| random_uuid(&entropy))
                        .map(HostValue::String),
                )
            },
        )
        .with_receiver_method("Crypto", "randomUUID")
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "__crypto_random",
            vec![ty.clone(), HostType::Bytes],
            HostType::Bytes,
            false,
            move |args| {
                let result = check_crypto(&args[0]);
                let mut args = args.into_iter();
                args.next();
                let Some(HostValue::Bytes(bytes)) = args.next() else {
                    unreachable!("checked random bytes")
                };
                HostReply::Ready(
                    result
                        .and_then(|_| random_values(bytes, &fill))
                        .map(HostValue::Bytes),
                )
            },
        )
        .with_receiver_method("Crypto", "getRandomValues")
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "__crypto_subtle",
            vec![ty],
            HostType::Handle("SubtleCrypto".into()),
            false,
            |args| {
                let HostValue::Handle(handle) = &args[0] else {
                    unreachable!("checked crypto receiver")
                };
                HostReply::Ready(
                    handle
                        .downcast_ref::<Crypto>()
                        .map(|value| HostValue::Handle(value.subtle.clone()))
                        .ok_or_else(|| "invalid Crypto resource".into()),
                )
            },
        )
        .with_receiver_property("Crypto", "subtle"),
    )?;
    hosts.register(
        HostOp::new(
            "__crypto_digest",
            vec![
                HostType::Handle("SubtleCrypto".into()),
                HostType::String,
                HostType::Bytes,
            ],
            HostType::Bytes,
            true,
            |args| {
                let mut args = args.into_iter();
                let Some(HostValue::Handle(handle)) = args.next() else {
                    unreachable!("checked digest receiver")
                };
                if handle.downcast_ref::<SubtleCrypto>().is_none() {
                    return HostReply::Ready(Err("invalid SubtleCrypto resource".into()));
                }
                let Some(HostValue::String(algorithm)) = args.next() else {
                    unreachable!("checked algorithm")
                };
                let Some(HostValue::Bytes(data)) = args.next() else {
                    unreachable!("checked digest data")
                };
                if data.len() > MAX_CRYPTO_INPUT {
                    return HostReply::Ready(Err("input too large".into()));
                }
                HostReply::Pending(Box::pin(async move {
                    tokio::task::spawn_blocking(move || digest(&algorithm, &data))
                        .await
                        .map_err(|error| format!("digest worker failed: {error}"))?
                        .map(HostValue::Bytes)
                }))
            },
        )
        .with_receiver_method("SubtleCrypto", "digest")
        .with_result_channel(),
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn hex(data: &[u8]) -> String {
        data.iter().map(|b| format!("{b:02x}")).collect()
    }
    #[test]
    fn known_hash_vectors_and_normalization_match_the_ported_engines() {
        assert_eq!(
            hex(&digest("SHA_256", b"abc").unwrap()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&digest("sha-384", b"abc").unwrap()),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
        assert_eq!(
            hex(&digest("sha512", b"").unwrap()),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
        assert_eq!(
            hex(&digest("SHA3", b"abc").unwrap()),
            "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"
        );
        assert_eq!(
            hex(&digest("blake3", b"").unwrap()),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert!(digest("md5", b"abc").is_err());
        assert!(digest("SHA-256", &vec![0; MAX_CRYPTO_INPUT + 1]).is_err());
    }
    #[test]
    fn entropy_is_called_uuid_bits_are_set_and_errors_are_not_empty_successes() {
        let fill: Entropy = Rc::new(|bytes| {
            bytes.fill(0xa5);
            Ok(())
        });
        assert_eq!(
            random_uuid(&fill).unwrap(),
            "a5a5a5a5-a5a5-45a5-a5a5-a5a5a5a5a5a5"
        );
        assert_eq!(random_values(vec![0; 4], &fill).unwrap(), [0xa5; 4]);
        let fail: Entropy = Rc::new(|_| Err("entropy failed".into()));
        assert_eq!(random_uuid(&fail).unwrap_err(), "entropy failed");
        assert_eq!(
            random_values(vec![0; 4], &fail).unwrap_err(),
            "entropy failed"
        );
        assert!(random_values(vec![0; MAX_RANDOM_BYTES + 1], &fill).is_err());
    }
    #[cfg(feature = "compiler")]
    #[tokio::test]
    async fn real_source_uses_entropy_and_retains_immutable_input() {
        let mut hosts = Hosts::default();
        crate::text_codec::register(&mut hosts).unwrap();
        register_with_entropy(
            &mut hosts,
            Rc::new(|bytes| {
                bytes.fill(0xa5);
                Ok(())
            }),
        )
        .unwrap();
        let source = r#"fn main() string {
            const input=TextEncoder().encode("abc");
            const random=unwrap(crypto.getRandomValues(input)) or{return "failed";};
            const id=unwrap(crypto.randomUUID()) or{return "failed";};
            return string(random[0])+":"+string(input[0])+":"+id;
        }"#;
        let mut vm =
            crate::Vm::new(crate::compiler::compile(source, &hosts).unwrap(), hosts).unwrap();
        assert_eq!(
            vm.run().await.unwrap(),
            HostValue::String("165:97:a5a5a5a5-a5a5-45a5-a5a5-a5a5a5a5a5a5".into())
        );
        assert_eq!(vm.stats().live, 0);
    }

    #[cfg(feature = "compiler")]
    #[tokio::test]
    async fn entropy_failure_reaches_the_actual_source_result_channels() {
        let mut hosts = Hosts::default();
        crate::text_codec::register(&mut hosts).unwrap();
        register_with_entropy(&mut hosts, Rc::new(|_| Err("entropy failed".into()))).unwrap();
        let source = r#"fn main() string {
            const input=TextEncoder().encode("data");
            const a=match crypto.randomUUID(){Ok(value)=>"bad",Err(error)=>error};
            const b=match crypto.getRandomValues(input){Ok(value)=>"bad",Err(error)=>error};
            return a+":"+b;
        }"#;
        let mut vm =
            crate::Vm::new(crate::compiler::compile(source, &hosts).unwrap(), hosts).unwrap();
        assert_eq!(
            vm.run().await.unwrap(),
            HostValue::String("entropy failed:entropy failed".into())
        );
        assert_eq!(vm.stats().live, 0);
    }
}
