use anyhow::Result;
use std::sync::{Mutex, OnceLock};
use zega_core::Zega;

use crate::redis_writer::ShopEdgeRow;

/// Global embedded Zega instance for edge publisher subdomain writes.
fn global_edge_zega() -> Option<std::sync::MutexGuard<'static, Zega>> {
    static ZEGA: OnceLock<Mutex<Zega>> = OnceLock::new();
    let zega = ZEGA.get_or_init(|| {
        let path = std::env::var("DEKA_EDGE_PUBLISHER_ZEGA_PATH")
            .expect("DEKA_EDGE_PUBLISHER_ZEGA_PATH must be set to use Zega writes");
        let zega = Zega::open(&path)
            .build()
            .expect("failed to open edge publisher zega");
        Mutex::new(zega)
    });
    zega.lock().ok()
}

/// Write a subdomain record from a shop edge row to Zega KV.
///
/// The key is `subdomain:{subdomain}` and the value is JSON
/// `{shop_id, account_id}` (or just the shop_id if no account_id is
/// available from the Neo4j row — the publisher currently only has
/// shop_id in the row).
pub fn write_shop_subdomain(row: &ShopEdgeRow) -> Result<()> {
    let Some(subdomain) = row.subdomain.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let zega = global_edge_zega().ok_or_else(|| {
        anyhow::anyhow!("edge publisher zega not available")
    })?;
    let key = format!("subdomain:{subdomain}");
    let value = serde_json::json!({
        "shop_id": row.shop_id,
    })
    .to_string();
    zega.kv_set(key, value.into(), None)
        .map_err(|e| anyhow::anyhow!("zega kv_set failed: {e}"))?;
    Ok(())
}

/// True when the edge publisher should also write subdomain records to Zega.
pub fn zega_writes_enabled() -> bool {
    std::env::var("DEKA_EDGE_PUBLISHER_ZEGA_PATH").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_shop_subdomain_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        unsafe {
            std::env::set_var("DEKA_EDGE_PUBLISHER_ZEGA_PATH", path);
        }

        let row = ShopEdgeRow {
            shop_id: "shop_zega_test".to_string(),
            shard_index: 2,
            subdomain: Some("zega-test".to_string()),
            updated_at: 101,
        };

        write_shop_subdomain(&row).expect("write should succeed");

        // Read back directly from Zega
        let zega = Zega::open(path).build().unwrap();
        let value = zega.kv_get("subdomain:zega-test").expect("key should exist");
        let raw = value.as_string().expect("value should be a string");
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed["shop_id"], "shop_zega_test");
    }

    #[test]
    fn write_skips_empty_subdomain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        unsafe {
            std::env::set_var("DEKA_EDGE_PUBLISHER_ZEGA_PATH", path);
        }

        let row = ShopEdgeRow {
            shop_id: "shop_no_sub".to_string(),
            shard_index: 0,
            subdomain: None,
            updated_at: 0,
        };

        write_shop_subdomain(&row).expect("write should succeed without subdomain");
    }

    #[test]
    fn zega_writes_enabled_reads_env() {
        unsafe {
            std::env::remove_var("DEKA_EDGE_PUBLISHER_ZEGA_PATH");
        }
        assert!(!zega_writes_enabled());
        unsafe {
            std::env::set_var("DEKA_EDGE_PUBLISHER_ZEGA_PATH", "/tmp/test-zega");
        }
        assert!(zega_writes_enabled());
    }
}
