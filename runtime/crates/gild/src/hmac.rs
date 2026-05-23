use sha2::{Digest, Sha256};
use std::{env, fs};

use crate::Result;

const TEST_DISPATCH_SECRET: &str = "gild-test-only-dispatch-secret";

pub fn dispatch_secret() -> Result<String> {
    if let Ok(secret) = env::var("GILD_DISPATCH_SECRET") {
        if secret.is_empty() {
            return Err("GILD_DISPATCH_SECRET is set but empty".into());
        }
        return Ok(secret);
    }
    if let Ok(path) = env::var("GILD_DISPATCH_SECRET_FILE") {
        let secret = fs::read_to_string(path)?.trim().to_string();
        if secret.is_empty() {
            return Err("GILD_DISPATCH_SECRET_FILE points to an empty secret".into());
        }
        return Ok(secret);
    }
    if env::var("GILD_DISPATCH_SECRET_TEST").as_deref() == Ok("1") {
        return Ok(TEST_DISPATCH_SECRET.to_string());
    }
    Err("missing gild dispatch secret: set GILD_DISPATCH_SECRET or GILD_DISPATCH_SECRET_FILE (GILD_DISPATCH_SECRET_TEST=1 is only for tests/dev)".into())
}

pub fn sign(timestamp: u64, body: &[u8], secret: &[u8]) -> String {
    let mut message = timestamp.to_string().into_bytes();
    message.push(b'.');
    message.extend_from_slice(body);
    hex(&hmac_sha256(secret, &message))
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut outer = [0x5c; BLOCK_SIZE];
    let mut inner = [0x36; BLOCK_SIZE];
    for index in 0..BLOCK_SIZE {
        outer[index] ^= key_block[index];
        inner[index] ^= key_block[index];
    }

    let mut inner_hash = Sha256::new();
    inner_hash.update(inner);
    inner_hash.update(message);
    let inner_digest = inner_hash.finalize();

    let mut outer_hash = Sha256::new();
    outer_hash.update(outer);
    outer_hash.update(inner_digest);
    outer_hash.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(TABLE[(byte >> 4) as usize] as char);
        encoded.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{dispatch_secret, hmac_sha256, TEST_DISPATCH_SECRET};
    use std::{
        env, fs,
        sync::{Mutex, MutexGuard},
        time::{SystemTime, UNIX_EPOCH},
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn clear_secret_env() {
        env::remove_var("GILD_DISPATCH_SECRET");
        env::remove_var("GILD_DISPATCH_SECRET_FILE");
        env::remove_var("GILD_DISPATCH_SECRET_TEST");
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_case_1() {
        let digest = hmac_sha256(&[0x0b; 20], b"Hi There");
        assert_eq!(
            super::hex(&digest),
            "b0344c61d8db38535ca8afceaf0bf12b\
             881dc200c9833da726e9376c2e32cff7"
                .replace(' ', "")
        );
    }

    #[test]
    fn dispatch_secret_reads_env_secret() {
        let _guard = lock_env();
        clear_secret_env();
        env::set_var("GILD_DISPATCH_SECRET", "env-secret");

        assert_eq!(dispatch_secret().unwrap(), "env-secret");

        clear_secret_env();
    }

    #[test]
    fn dispatch_secret_reads_secret_file() {
        let _guard = lock_env();
        clear_secret_env();
        let path = env::temp_dir().join(format!(
            "gild-dispatch-secret-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, "file-secret\n").unwrap();
        env::set_var("GILD_DISPATCH_SECRET_FILE", &path);

        assert_eq!(dispatch_secret().unwrap(), "file-secret");

        clear_secret_env();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn dispatch_secret_allows_explicit_test_fallback() {
        let _guard = lock_env();
        clear_secret_env();
        env::set_var("GILD_DISPATCH_SECRET_TEST", "1");

        assert_eq!(dispatch_secret().unwrap(), TEST_DISPATCH_SECRET);

        clear_secret_env();
    }

    #[test]
    fn dispatch_secret_fails_closed_without_secret() {
        let _guard = lock_env();
        clear_secret_env();

        let err = dispatch_secret().unwrap_err().to_string();
        assert!(err.contains("missing gild dispatch secret"), "{err}");

        clear_secret_env();
    }
}
