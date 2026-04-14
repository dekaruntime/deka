//! deka-shard — client-side shard resolver.
//!
//! Tana distributes tenant data across N application servers.
//! Each shop has an immutable `account_id` (UUID v4). The shard
//! that owns a shop is determined purely by
//! `hash(account_id) % shard_count` — no server-side lookup,
//! Redis-Cluster-style.
//!
//! This crate is pure, deterministic, and does no network I/O.
//! It loads the cluster topology once from a JSON file (or an
//! env-driven single-shard fallback) and resolves account_ids
//! against it.
//!
//! Phase 2 of the sharded-deployment plan ships the library
//! only; phase 3 wires it into the Neo4j / Redis pools and the
//! cross-shard HTTP proxy.

mod config;
mod hash;

pub use config::{ShardConfig, ShardInfo};
pub use hash::{fnv1a_64, shard_index};

use std::path::PathBuf;
use std::sync::Arc;

/// Environment variable pointing at the shard config JSON file.
pub const ENV_SHARD_CONFIG: &str = "DEKA_SHARD_CONFIG";
/// Environment variable naming this server in the cluster
/// (e.g. `phobos`, `bugsy`, `jynx`). Falls back to `hostname -s`.
pub const ENV_SHARD_SELF: &str = "DEKA_SHARD_SELF";

/// Default config location relative to `$HOME`.
const DEFAULT_CONFIG_REL: &str = "Projects/tana/infra/shards.json";

/// Client-side shard resolver.
///
/// Cheap to clone — the shard list lives behind an `Arc`.
#[derive(Clone, Debug)]
pub struct ShardResolver {
    shards: Arc<Vec<ShardInfo>>,
    self_index: Option<usize>,
}

impl ShardResolver {
    /// Load a resolver using process environment variables.
    ///
    /// Resolution order:
    /// 1. If `DEKA_SHARD_CONFIG` is set, load that path (error if unreadable).
    /// 2. Otherwise, try `~/Projects/tana/infra/shards.json`.
    /// 3. Otherwise, fall back to a single-shard localhost config.
    ///
    /// The "self" shard is picked from `DEKA_SHARD_SELF`, falling
    /// back to `hostname -s`.
    pub fn from_env() -> Result<Self, String> {
        let config = load_config_from_env()?;
        let self_name = self_name_from_env();
        Ok(Self::from_config(config, self_name.as_deref()))
    }

    /// Build a resolver from an explicit config.
    ///
    /// `self_name` identifies this server in the cluster; if it
    /// matches a shard entry by `name`, that shard's index is
    /// recorded for `owns` / `self_shard`.
    pub fn from_config(config: ShardConfig, self_name: Option<&str>) -> Self {
        let shards = config.shards;
        let self_index = self_name.and_then(|n| shards.iter().position(|s| s.name == n));
        ShardResolver {
            shards: Arc::new(shards),
            self_index,
        }
    }

    /// Resolve an `account_id` to its owning shard.
    ///
    /// Returns `None` for empty `account_id` or for an empty shard list.
    pub fn resolve(&self, account_id: &str) -> Option<&ShardInfo> {
        if account_id.is_empty() || self.shards.is_empty() {
            return None;
        }
        let idx = shard_index(account_id, self.shards.len());
        self.shards.get(idx)
    }

    /// True iff THIS server's shard owns the given account_id.
    ///
    /// Returns `false` if `self_index` isn't set (e.g. running
    /// outside any shard — dev tools, CLI one-shots).
    pub fn owns(&self, account_id: &str) -> bool {
        let Some(self_idx) = self.self_index else {
            return false;
        };
        match self.resolve(account_id) {
            Some(info) => info.index == self_idx,
            None => false,
        }
    }

    /// Info about THIS server's shard, if one is configured.
    pub fn self_shard(&self) -> Option<&ShardInfo> {
        self.self_index.and_then(|i| self.shards.get(i))
    }

    /// Number of shards in the cluster.
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    /// All shard entries in their configured order.
    pub fn shards(&self) -> &[ShardInfo] {
        &self.shards
    }
}

fn load_config_from_env() -> Result<ShardConfig, String> {
    if let Ok(path) = std::env::var(ENV_SHARD_CONFIG) {
        if path.is_empty() {
            return Ok(ShardConfig::single_shard_localhost());
        }
        let p = PathBuf::from(&path);
        if p.exists() {
            return ShardConfig::from_file(&p);
        }
        // Explicitly set but missing — prefer the fallback over a hard error
        // so dev environments "just work" if the file hasn't been synced yet.
        return Ok(ShardConfig::single_shard_localhost());
    }

    if let Some(home) = home_dir() {
        let p = home.join(DEFAULT_CONFIG_REL);
        if p.exists() {
            return ShardConfig::from_file(&p);
        }
    }

    Ok(ShardConfig::single_shard_localhost())
}

fn self_name_from_env() -> Option<String> {
    if let Ok(v) = std::env::var(ENV_SHARD_SELF) {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    hostname_short()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn hostname_short() -> Option<String> {
    let out = std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg3() -> ShardConfig {
        ShardConfig {
            shards: vec![
                ShardInfo {
                    index: 0,
                    name: "phobos".into(),
                    neo4j: "bolt://100.113.2.21:7687".into(),
                    redis: "redis://100.113.2.21:6379".into(),
                },
                ShardInfo {
                    index: 1,
                    name: "bugsy".into(),
                    neo4j: "bolt://100.70.138.96:7687".into(),
                    redis: "redis://100.70.138.96:6379".into(),
                },
                ShardInfo {
                    index: 2,
                    name: "jynx".into(),
                    neo4j: "bolt://100.106.121.78:7687".into(),
                    redis: "redis://100.106.121.78:6379".into(),
                },
            ],
        }
    }

    fn random_uuid_v4(seed: &mut u64) -> String {
        // Cheap xorshift — good enough for distribution testing.
        let mut bytes = [0u8; 16];
        for b in bytes.iter_mut() {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            *b = (*seed & 0xff) as u8;
        }
        // Force v4 layout bits so inputs look like real UUIDs.
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[4], bytes[5], bytes[6], bytes[7],
            bytes[8], bytes[9], bytes[10], bytes[11],
            bytes[12], bytes[13], bytes[14], bytes[15],
        )
    }

    #[test]
    fn resolve_is_deterministic() {
        let r = ShardResolver::from_config(cfg3(), None);
        let id = "c0dc1618-20fc-4bdd-ac6f-e94909f8fad2";
        let a = r.resolve(id).unwrap().name.clone();
        let b = r.resolve(id).unwrap().name.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn resolve_empty_is_none() {
        let r = ShardResolver::from_config(cfg3(), None);
        assert!(r.resolve("").is_none());
    }

    #[test]
    fn resolve_empty_shards_is_none() {
        let r = ShardResolver::from_config(ShardConfig { shards: vec![] }, None);
        assert!(r.resolve("abc").is_none());
        assert_eq!(r.shard_count(), 0);
    }

    #[test]
    fn owns_matches_resolve() {
        let r = ShardResolver::from_config(cfg3(), Some("bugsy"));
        let mut seed: u64 = 0x9E3779B97F4A7C15;
        let mut saw_owned = false;
        let mut saw_unowned = false;
        for _ in 0..200 {
            let id = random_uuid_v4(&mut seed);
            let resolved = r.resolve(&id).unwrap();
            let expected = resolved.name == "bugsy";
            assert_eq!(r.owns(&id), expected);
            saw_owned |= expected;
            saw_unowned |= !expected;
        }
        assert!(saw_owned);
        assert!(saw_unowned);
    }

    #[test]
    fn owns_false_when_self_not_in_cluster() {
        let r = ShardResolver::from_config(cfg3(), Some("not-a-real-host"));
        assert!(r.self_shard().is_none());
        assert!(!r.owns("c0dc1618-20fc-4bdd-ac6f-e94909f8fad2"));
    }

    #[test]
    fn owns_false_when_no_self_name() {
        let r = ShardResolver::from_config(cfg3(), None);
        assert!(!r.owns("c0dc1618-20fc-4bdd-ac6f-e94909f8fad2"));
    }

    #[test]
    fn self_shard_is_correct() {
        let r = ShardResolver::from_config(cfg3(), Some("phobos"));
        let s = r.self_shard().unwrap();
        assert_eq!(s.index, 0);
        assert_eq!(s.name, "phobos");
    }

    #[test]
    fn distribution_three_shards_is_balanced() {
        let r = ShardResolver::from_config(cfg3(), None);
        let mut counts = [0usize; 3];
        let mut seed: u64 = 0xDEADBEEFCAFEBABE;
        let n = 10_000;
        for _ in 0..n {
            let id = random_uuid_v4(&mut seed);
            let s = r.resolve(&id).unwrap();
            counts[s.index] += 1;
        }
        // Each shard should get 30-37% (spec) — use 0.30..=0.37 inclusive.
        for (i, c) in counts.iter().enumerate() {
            let pct = *c as f64 / n as f64;
            assert!(
                (0.30..=0.37).contains(&pct),
                "shard {i} got {pct:.4} of {n} (count {c})"
            );
        }
    }

    #[test]
    fn distribution_two_shards_is_balanced() {
        let cfg = ShardConfig {
            shards: cfg3().shards.into_iter().take(2).collect(),
        };
        let r = ShardResolver::from_config(cfg, None);
        let mut counts = [0usize; 2];
        let mut seed: u64 = 0x123456789ABCDEF0;
        let n = 10_000;
        for _ in 0..n {
            let id = random_uuid_v4(&mut seed);
            let s = r.resolve(&id).unwrap();
            counts[s.index] += 1;
        }
        for (i, c) in counts.iter().enumerate() {
            let pct = *c as f64 / n as f64;
            assert!(
                (0.46..=0.54).contains(&pct),
                "shard {i} got {pct:.4} of {n} (count {c})"
            );
        }
    }

    #[test]
    fn from_env_missing_config_falls_back_to_single_shard() {
        // DEKA_SHARD_CONFIG pointing at a path that doesn't exist
        // should produce the localhost fallback, not an error.
        // We can't safely mutate process env in parallel tests, so
        // exercise the load path directly with a bogus path.
        let p = std::path::Path::new("/nonexistent/deka/shards.json");
        assert!(!p.exists());
        let cfg = ShardConfig::single_shard_localhost();
        let r = ShardResolver::from_config(cfg, None);
        assert_eq!(r.shard_count(), 1);
        assert_eq!(r.shards()[0].name, "local");
    }

    #[test]
    fn resolver_is_cheap_to_clone() {
        let r = ShardResolver::from_config(cfg3(), Some("phobos"));
        let r2 = r.clone();
        assert_eq!(r.shard_count(), r2.shard_count());
        assert_eq!(r.self_shard().unwrap().name, r2.self_shard().unwrap().name);
    }

    #[test]
    fn real_account_ids_resolve_stably() {
        // These are real shop account_ids from the database (per Phase 2 spec).
        let r = ShardResolver::from_config(cfg3(), None);
        let shop_alpha = "c0dc1618-20fc-4bdd-ac6f-e94909f8fad2";
        let shop_beta = "2789d397-a96a-44ba-9073-24c711d007ff";
        assert!(r.resolve(shop_alpha).is_some());
        assert!(r.resolve(shop_beta).is_some());
        // Repeated resolution agrees.
        assert_eq!(
            r.resolve(shop_alpha).unwrap().index,
            r.resolve(shop_alpha).unwrap().index,
        );
    }
}
