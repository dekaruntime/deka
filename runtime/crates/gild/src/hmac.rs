use sha2::{Digest, Sha256};
use std::{env, fs};

use crate::Result;

pub fn dispatch_secret() -> Result<String> {
    if let Ok(secret) = env::var("GILD_DISPATCH_SECRET") {
        return Ok(secret);
    }
    if let Ok(path) = env::var("GILD_DISPATCH_SECRET_FILE") {
        return Ok(fs::read_to_string(path)?.trim().to_string());
    }
    Ok("gild-skeleton-dev-secret".to_string())
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
    use super::hmac_sha256;

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
}
