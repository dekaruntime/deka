use super::*;
use super::security::{enforce_env, set_security_privileged};

#[op2(fast)]
pub(super) fn op_php_set_privileged(#[number] enabled: i64, #[string] label: String) {
    let label = if label.trim().is_empty() { None } else { Some(label) };
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
