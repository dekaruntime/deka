use super::security::{enforce_env, set_security_privileged};
use super::*;

#[op2(fast)]
pub(super) fn op_php_set_privileged(#[number] enabled: i64, #[string] label: String) {
    let label = if label.trim().is_empty() {
        None
    } else {
        Some(label)
    };
    set_security_privileged(enabled != 0, label);
}

#[op2]
#[string]
pub(super) fn op_php_sha256(#[string] data: String) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let digest = hasher.finalize();
    format!("{:x}", digest)
}

#[op2]
#[buffer]
pub(super) fn op_php_random_bytes(#[number] len: i64) -> Vec<u8> {
    if len <= 0 {
        return Vec::new();
    }
    if len > (1024 * 1024) {
        return Vec::new();
    }
    let mut out = vec![0u8; len as usize];
    if getrandom::getrandom(&mut out).is_err() {
        return Vec::new();
    }
    out
}

// AES-256-GCM encrypt / decrypt — used by @deka/payments to store provider
// OAuth tokens at rest (Square in particular; issue #119).
//
// Layout notes for callers: the PHPX side composes a `v1:{base64(nonce)}:{base64(ct||tag)}`
// wire format. These ops stay small: take raw bytes in, emit raw bytes out.
// A ciphertext-and-tag layout (tag appended to ciphertext) is what
// `aes-gcm` and every other AEAD crate expects, so PHPX just slices the
// last 16 bytes as the tag at decrypt time.
//
// Returns `{ok: true, data: Vec<u8>}` on success, `{ok: false, error: string}`
// on failure. We never leak WHICH input was wrong on decrypt — a key
// mismatch, a bit-flipped ciphertext, and a tampered tag all return the
// same generic "aes_decrypt_failed" error.

#[op2]
#[serde]
pub(super) fn op_php_aes_256_gcm_encrypt(
    #[buffer] key: &[u8],
    #[buffer] nonce: &[u8],
    #[buffer] plaintext: &[u8],
    #[buffer] aad: &[u8],
) -> serde_json::Value {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes256Gcm, Key, Nonce};

    if key.len() != 32 {
        return serde_json::json!({
            "ok": false,
            "error": "key_length_invalid",
        });
    }
    if nonce.len() != 12 {
        return serde_json::json!({
            "ok": false,
            "error": "nonce_length_invalid",
        });
    }

    let key = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce);
    let payload = Payload {
        msg: plaintext,
        aad,
    };
    match cipher.encrypt(nonce, payload) {
        Ok(ct) => serde_json::json!({
            "ok": true,
            "data": ct,
        }),
        Err(_) => serde_json::json!({
            "ok": false,
            "error": "aes_encrypt_failed",
        }),
    }
}

#[op2]
#[serde]
pub(super) fn op_php_aes_256_gcm_decrypt(
    #[buffer] key: &[u8],
    #[buffer] nonce: &[u8],
    #[buffer] ciphertext: &[u8],
    #[buffer] aad: &[u8],
) -> serde_json::Value {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes256Gcm, Key, Nonce};

    if key.len() != 32 {
        return serde_json::json!({
            "ok": false,
            "error": "key_length_invalid",
        });
    }
    if nonce.len() != 12 {
        return serde_json::json!({
            "ok": false,
            "error": "nonce_length_invalid",
        });
    }
    // ciphertext must include the trailing 16-byte GCM tag.
    if ciphertext.len() < 16 {
        return serde_json::json!({
            "ok": false,
            "error": "aes_decrypt_failed",
        });
    }

    let key = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce);
    let payload = Payload {
        msg: ciphertext,
        aad,
    };
    match cipher.decrypt(nonce, payload) {
        Ok(pt) => serde_json::json!({
            "ok": true,
            "data": pt,
        }),
        Err(_) => serde_json::json!({
            "ok": false,
            "error": "aes_decrypt_failed",
        }),
    }
}

pub(super) fn bcrypt_verify_impl(password: String, hash: String) -> serde_json::Value {
    if password.len() > 72 {
        return serde_json::json!({
            "ok": true,
            "valid": false,
        });
    }

    match bcrypt::verify(password, &hash) {
        Ok(valid) => serde_json::json!({
            "ok": true,
            "valid": valid,
        }),
        Err(_) => serde_json::json!({
            "ok": false,
            "error": "bcrypt_verify_failed",
        }),
    }
}

#[op2]
#[serde]
pub(super) fn op_php_bcrypt_verify(
    #[string] password: String,
    #[string] hash: String,
) -> serde_json::Value {
    bcrypt_verify_impl(password, hash)
}

const MAX_CRYPTO_INPUT: usize = 16 * 1024 * 1024;

fn normalize_alg(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .replace('_', "-")
        .replace(' ', "")
}

fn crypto_too_large(n: usize) -> bool {
    n > MAX_CRYPTO_INPUT
}

pub(super) fn digest_impl(algorithm: &str, data: &[u8]) -> serde_json::Value {
    if crypto_too_large(data.len()) {
        return serde_json::json!({ "ok": false, "error": "input too large" });
    }
    let alg = normalize_alg(algorithm);
    let digest = match alg.as_str() {
        "sha256" | "sha-256" => {
            use sha2::{Digest, Sha256};
            Some(Sha256::digest(data).to_vec())
        }
        "sha384" | "sha-384" => {
            use sha2::{Digest, Sha384};
            Some(Sha384::digest(data).to_vec())
        }
        "sha512" | "sha-512" => {
            use sha2::{Digest, Sha512};
            Some(Sha512::digest(data).to_vec())
        }
        "sha3-256" | "sha3" => {
            use sha3::{Digest, Sha3_256};
            Some(Sha3_256::digest(data).to_vec())
        }
        "blake3" => Some(blake3::hash(data).as_bytes().to_vec()),
        _ => None,
    };
    match digest {
        Some(data) => serde_json::json!({ "ok": true, "data": data }),
        None => serde_json::json!({
            "ok": false,
            "error": format!("unknown digest algorithm '{algorithm}'"),
        }),
    }
}

pub(super) fn hmac_impl(algorithm: &str, key: &[u8], data: &[u8]) -> serde_json::Value {
    if crypto_too_large(key.len()) || crypto_too_large(data.len()) {
        return serde_json::json!({ "ok": false, "error": "input too large" });
    }
    let alg = normalize_alg(algorithm);
    match alg.as_str() {
        "sha256" | "sha-256" | "hs256" => hmac_with_sha256(key, data),
        "sha384" | "sha-384" | "hs384" => hmac_with_sha384(key, data),
        "sha512" | "sha-512" | "hs512" => hmac_with_sha512(key, data),
        _ => serde_json::json!({
            "ok": false,
            "error": format!("unknown hmac algorithm '{algorithm}'"),
        }),
    }
}

fn hmac_with_sha256(key: &[u8], data: &[u8]) -> serde_json::Value {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    hmac_finish(Hmac::<Sha256>::new_from_slice(key), data)
}

fn hmac_with_sha384(key: &[u8], data: &[u8]) -> serde_json::Value {
    use hmac::{Hmac, Mac};
    use sha2::Sha384;
    hmac_finish(Hmac::<Sha384>::new_from_slice(key), data)
}

fn hmac_with_sha512(key: &[u8], data: &[u8]) -> serde_json::Value {
    use hmac::{Hmac, Mac};
    use sha2::Sha512;
    hmac_finish(Hmac::<Sha512>::new_from_slice(key), data)
}

fn hmac_finish<M: hmac::Mac>(
    mac: Result<M, impl std::fmt::Debug>,
    data: &[u8],
) -> serde_json::Value {
    let mut mac = match mac {
        Ok(mac) => mac,
        Err(_) => {
            return serde_json::json!({ "ok": false, "error": "invalid hmac key" });
        }
    };
    mac.update(data);
    serde_json::json!({ "ok": true, "data": mac.finalize().into_bytes().to_vec() })
}

pub(super) fn secure_compare_impl(a: &[u8], b: &[u8]) -> serde_json::Value {
    serde_json::json!({ "ok": true, "data": constant_time_eq(a, b) })
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    std::hint::black_box(diff) == 0
}

#[op2]
#[serde]
pub(super) fn op_php_digest(
    #[string] algorithm: String,
    #[buffer] data: &[u8],
) -> serde_json::Value {
    digest_impl(&algorithm, data)
}

#[op2]
#[serde]
pub(super) fn op_php_hmac(
    #[string] algorithm: String,
    #[buffer] key: &[u8],
    #[buffer] data: &[u8],
) -> serde_json::Value {
    hmac_impl(&algorithm, key, data)
}

#[op2]
#[serde]
pub(super) fn op_php_secure_compare(#[buffer] a: &[u8], #[buffer] b: &[u8]) -> serde_json::Value {
    secure_compare_impl(a, b)
}

#[op2]
#[serde]
pub(super) fn op_php_read_env() -> HashMap<String, String> {
    if enforce_env(None).is_err() {
        return HashMap::new();
    }
    let mut merged = HashMap::new();
    for (key, value) in std::env::vars() {
        merged.insert(key, value);
    }
    for (key, value) in read_dotenv_from_cwd() {
        merged.insert(key, value);
    }
    merged
}

fn read_dotenv_from_cwd() -> HashMap<String, String> {
    let out = HashMap::new();
    let Ok(cwd) = std::env::current_dir() else {
        return out;
    };
    let path = cwd.join(".env");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return out;
    };
    parse_dotenv(&raw)
}

fn parse_dotenv(raw: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let body = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let Some((key_raw, value_raw)) = body.split_once('=') else {
            continue;
        };
        let key = key_raw.trim();
        if key.is_empty() {
            continue;
        }
        let mut value = value_raw.trim().to_string();
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            value = decode_double_quoted(&value[1..value.len() - 1]);
        } else if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
            value = value[1..value.len() - 1].to_string();
        } else if let Some(idx) = value.find(" #") {
            value = value[..idx].trim_end().to_string();
        }
        out.insert(key.to_string(), value);
    }
    out
}

fn decode_double_quoted(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        match next {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{digest_impl, hmac_impl, secure_compare_impl};

    #[test]
    fn sha256_empty_is_known_vector() {
        let out = digest_impl("sha256", b"");
        assert_eq!(out["ok"], true);
        let data = out["data"]
            .as_array()
            .expect("digest data")
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect::<Vec<_>>();
        let hex = data.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hmac_sha256_rfc4231_case_1() {
        let key = [0x0bu8; 20];
        let out = hmac_impl("sha256", &key, b"Hi There");
        assert_eq!(out["ok"], true);
        let data = out["data"]
            .as_array()
            .expect("hmac data")
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect::<Vec<_>>();
        let hex = data.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn digest_rejects_unknown_algorithm() {
        let out = digest_impl("md5", b"nope");
        assert_eq!(out["ok"], false);
        assert!(
            out["error"]
                .as_str()
                .unwrap_or("")
                .contains("unknown digest algorithm")
        );
    }

    #[test]
    fn secure_compare_is_length_and_value_sensitive() {
        let same = secure_compare_impl(b"abc", b"abc");
        let diff = secure_compare_impl(b"abc", b"abd");
        let len = secure_compare_impl(b"abc", b"ab");
        assert_eq!(same["data"], true);
        assert_eq!(diff["data"], false);
        assert_eq!(len["data"], false);
    }
}
