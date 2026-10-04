//! Synchronous crypto module exports (the global SubtleCrypto API remains async).
use crate::crypto::{self, Entropy};
use crate::{HostOp, HostReply, HostType, HostValue, Hosts, Result};
use std::rc::Rc;
#[derive(Clone, Copy)]
enum Function {
    RandomBytes,
    Digest,
    Hmac,
    Compare,
    Encrypt,
    Decrypt,
    Bcrypt,
    Sha256,
    Sha384,
    Sha512,
    Sha3,
    Blake3,
    Hs256,
    Hs384,
    Hs512,
    RandomHex,
    Uuid,
}
impl Function {
    const ALL: [Self; 17] = [
        Self::RandomBytes,
        Self::Digest,
        Self::Hmac,
        Self::Compare,
        Self::Encrypt,
        Self::Decrypt,
        Self::Bcrypt,
        Self::Sha256,
        Self::Sha384,
        Self::Sha512,
        Self::Sha3,
        Self::Blake3,
        Self::Hs256,
        Self::Hs384,
        Self::Hs512,
        Self::RandomHex,
        Self::Uuid,
    ];
    fn declaration(self) -> (&'static str, Vec<HostType>, HostType) {
        use HostType::{Bool, Bytes, Number, String};
        match self {
            Self::RandomBytes => ("random_bytes", vec![Number], Bytes),
            Self::Digest => ("digest", vec![String, Bytes], Bytes),
            Self::Hmac => ("hmac", vec![String, Bytes, Bytes], Bytes),
            Self::Compare => ("secure_compare", vec![Bytes, Bytes], Bool),
            Self::Encrypt => (
                "aes_256_gcm_encrypt",
                vec![Bytes, Bytes, Bytes, Bytes],
                Bytes,
            ),
            Self::Decrypt => (
                "aes_256_gcm_decrypt",
                vec![Bytes, Bytes, Bytes, Bytes],
                Bytes,
            ),
            Self::Bcrypt => ("bcrypt_verify", vec![String, String], Bool),
            Self::Sha256 => ("sha256", vec![Bytes], Bytes),
            Self::Sha384 => ("sha384", vec![Bytes], Bytes),
            Self::Sha512 => ("sha512", vec![Bytes], Bytes),
            Self::Sha3 => ("sha3_256", vec![Bytes], Bytes),
            Self::Blake3 => ("blake3", vec![Bytes], Bytes),
            Self::Hs256 => ("hs256", vec![Bytes, Bytes], Bytes),
            Self::Hs384 => ("hs384", vec![Bytes, Bytes], Bytes),
            Self::Hs512 => ("hs512", vec![Bytes, Bytes], Bytes),
            Self::RandomHex => ("random_hex", vec![Number], String),
            Self::Uuid => ("uuid_v4", vec![], String),
        }
    }
    fn invoke(self, args: &[HostValue], fill: &Entropy) -> Result<HostValue> {
        fn bytes(v: &HostValue) -> &[u8] {
            let HostValue::Bytes(v) = v else {
                unreachable!("checked bytes")
            };
            v
        }
        fn text(v: &HostValue) -> &str {
            let HostValue::String(v) = v else {
                unreachable!("checked string")
            };
            v
        }
        match self {
            Self::RandomBytes | Self::RandomHex => {
                let HostValue::Number(len) = args[0] else {
                    unreachable!("checked number")
                };
                let data = crypto::random_bytes(len, fill)?;
                Ok(if matches!(self, Self::RandomBytes) {
                    HostValue::Bytes(data)
                } else {
                    HostValue::String(crate::bytes::to_hex(&data))
                })
            }
            Self::Uuid => crypto::random_uuid(fill).map(HostValue::String),
            Self::Digest => crypto::digest(text(&args[0]), bytes(&args[1])).map(HostValue::Bytes),
            Self::Hmac => {
                crypto::hmac(text(&args[0]), bytes(&args[1]), bytes(&args[2])).map(HostValue::Bytes)
            }
            Self::Compare => {
                crypto::secure_compare(bytes(&args[0]), bytes(&args[1])).map(HostValue::Bool)
            }
            Self::Encrypt | Self::Decrypt => crypto::aes_256_gcm(
                bytes(&args[0]),
                bytes(&args[1]),
                bytes(&args[2]),
                bytes(&args[3]),
                matches!(self, Self::Decrypt),
            )
            .map(HostValue::Bytes),
            Self::Bcrypt => {
                crypto::bcrypt_verify(text(&args[0]), text(&args[1])).map(HostValue::Bool)
            }
            Self::Hs256 | Self::Hs384 | Self::Hs512 => {
                let alg = match self {
                    Self::Hs256 => "HS256",
                    Self::Hs384 => "HS384",
                    _ => "HS512",
                };
                crypto::hmac(alg, bytes(&args[0]), bytes(&args[1])).map(HostValue::Bytes)
            }
            Self::Sha256 | Self::Sha384 | Self::Sha512 | Self::Sha3 | Self::Blake3 => {
                let alg = match self {
                    Self::Sha256 => "SHA-256",
                    Self::Sha384 => "SHA-384",
                    Self::Sha512 => "SHA-512",
                    Self::Sha3 => "SHA3-256",
                    _ => "BLAKE3",
                };
                crypto::digest(alg, bytes(&args[0])).map(HostValue::Bytes)
            }
        }
    }
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    register_with_entropy(hosts, Rc::new(crypto::fill_random))
}
fn register_with_entropy(hosts: &mut Hosts, fill: Entropy) -> Result<()> {
    for function in Function::ALL {
        let (name, args, result) = function.declaration();
        let fill = fill.clone();
        hosts.register(
            HostOp::new(
                &format!("crypto_{name}"),
                args,
                result,
                false,
                move |args| HostReply::Ready(function.invoke(&args, &fill)),
            )
            .with_result_channel(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_entropy_is_used_and_failures_are_not_success() {
        let fill: Entropy = Rc::new(|out| {
            out.fill(0xa5);
            Ok(())
        });
        assert_eq!(
            Function::RandomHex
                .invoke(&[HostValue::Number(3.)], &fill)
                .unwrap(),
            HostValue::String("a5a5a5".into())
        );
        assert_eq!(
            Function::Uuid.invoke(&[], &fill).unwrap(),
            HostValue::String("a5a5a5a5-a5a5-45a5-a5a5-a5a5a5a5a5a5".into())
        );
        let fail: Entropy = Rc::new(|_| Err("entropy failed".into()));
        for f in [Function::RandomBytes, Function::RandomHex, Function::Uuid] {
            let args = if matches!(f, Function::Uuid) {
                vec![]
            } else {
                vec![HostValue::Number(8.)]
            };
            assert_eq!(f.invoke(&args, &fail).unwrap_err(), "entropy failed");
        }
        for len in [0., -1., 0.5, f64::NAN, f64::INFINITY, 1_048_577.] {
            assert!(crypto::random_bytes(len, &fill).is_err());
        }
        assert_eq!(
            crypto::random_bytes(1_048_576., &fill).unwrap().len(),
            1_048_576
        );
    }
    #[test]
    fn engine_bounds_and_authentication_failures_are_explicit() {
        use aes_gcm::aead::{AeadCore, KeyInit, OsRng};
        let key = aes_gcm::Aes256Gcm::generate_key(&mut OsRng).to_vec();
        let nonce = aes_gcm::Aes256Gcm::generate_nonce(&mut OsRng).to_vec();
        let mut wrong_key = key.clone();
        wrong_key[0] ^= 1;
        let large = vec![0; 16 * 1024 * 1024 + 1];
        let algorithm = "x".repeat(large.len());
        assert!(crypto::digest(&algorithm, &[]).is_err());
        assert!(crypto::hmac(&algorithm, &[], &[]).is_err());
        assert!(crypto::hmac("sha256", &[], &large).is_err());
        assert!(crypto::secure_compare(&large, &large).is_err());
        assert!(crypto::aes_256_gcm(&key, &nonce, &large, &[], false).is_err());
        assert_eq!(
            crypto::aes_256_gcm(&key, &nonce[..11], &[], &[], false).unwrap_err(),
            "nonce_length_invalid"
        );
        assert_eq!(
            crypto::aes_256_gcm(&key, &nonce, &[], &[], true).unwrap_err(),
            "aes_decrypt_failed"
        );
        let cipher = crypto::aes_256_gcm(&key, &nonce, b"native", b"aad", false).unwrap();
        let mut tampered = cipher.clone();
        tampered[0] ^= 1;
        for (key, data, aad) in [
            (wrong_key.as_slice(), &cipher[..], b"aad".as_slice()),
            (key.as_slice(), &tampered[..], b"aad".as_slice()),
            (key.as_slice(), &cipher[..], b"bad".as_slice()),
        ] {
            assert_eq!(
                crypto::aes_256_gcm(key, &nonce, data, aad, true).unwrap_err(),
                "aes_decrypt_failed"
            );
        }
        assert!(crypto::secure_compare(&[], &[]).unwrap());
        assert!(!crypto::secure_compare(b"a", b"aa").unwrap());
    }
}
