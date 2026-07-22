//! Deterministic hashing for shard assignment.
//!
//! Uses FNV-1a 64-bit — a fast, deterministic, std-only hash.
//! The hash is stable across processes and hosts, which is a hard
//! requirement for client-side shard routing: every server must
//! agree on which shard owns a given shop_id slug.

/// FNV-1a 64-bit hash.
///
/// Deterministic, std-only, no allocations. Suitable for shard
/// routing where every node must produce the same hash for the
/// same input.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Given a shop_id slug and shard count, return the shard index.
///
/// Returns 0 when `shard_count` is 0 (degenerate / single-shard fallback).
pub fn shard_index(shop_id: &str, shard_count: usize) -> usize {
    if shard_count == 0 {
        return 0;
    }
    (fnv1a_64(shop_id.as_bytes()) % shard_count as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_is_deterministic() {
        let a = fnv1a_64(b"c0dc1618-20fc-4bdd-ac6f-e94909f8fad2");
        let b = fnv1a_64(b"c0dc1618-20fc-4bdd-ac6f-e94909f8fad2");
        assert_eq!(a, b);
    }

    #[test]
    fn fnv1a_known_values() {
        // FNV-1a reference vectors.
        assert_eq!(fnv1a_64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a_64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn shard_index_is_deterministic() {
        let shop_id = "shop_beta";
        let a = shard_index(shop_id, 3);
        let b = shard_index(shop_id, 3);
        assert_eq!(a, b);
    }

    #[test]
    fn shard_index_matches_fnv1a_mod_shard_count() {
        let shop_id = "shop_alpha";
        let shard_count = 3;
        assert_eq!(
            shard_index(shop_id, shard_count),
            (fnv1a_64(shop_id.as_bytes()) % shard_count as u64) as usize
        );
    }

    #[test]
    fn shard_index_zero_count_is_safe() {
        assert_eq!(shard_index("anything", 0), 0);
    }

    #[test]
    fn shard_index_is_bounded() {
        for i in 0..1000 {
            let shop_id = format!("shop_{i}");
            assert!(shard_index(&shop_id, 3) < 3);
            assert!(shard_index(&shop_id, 7) < 7);
        }
    }
}
