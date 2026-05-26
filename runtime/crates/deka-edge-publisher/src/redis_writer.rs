use anyhow::Result;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const EDGE_UPDATES_CHANNEL: &str = "edge-updates";
pub const EDGE_KEY_TTL_SECS: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRecordCache {
    #[serde(rename = "type")]
    pub record_type: String,
    pub name: String,
    pub value: String,
    pub ttl: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainCache {
    pub shop_id: String,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopEdgeRow {
    pub shop_id: String,
    pub shard_index: i64,
    pub subdomain: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainEdgeRow {
    pub name: String,
    pub shop_id: String,
    pub verified: bool,
    pub verification_token: Option<String>,
    pub records: Vec<DnsRecordCache>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EdgeDelta {
    Shop {
        shop_id: String,
        shard_index: i64,
        subdomain: Option<String>,
        updated_at: i64,
    },
    Domain {
        name: String,
        shop_id: String,
        verified: bool,
        records: Vec<DnsRecordCache>,
        updated_at: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisWrite {
    pub key: String,
    pub value: String,
}

pub fn shop_writes(row: &ShopEdgeRow) -> Vec<RedisWrite> {
    let mut writes = vec![RedisWrite {
        key: format!("shop:{}:shard", row.shop_id),
        value: row.shard_index.to_string(),
    }];

    if let Some(subdomain) = row.subdomain.as_deref().filter(|s| !s.is_empty()) {
        writes.push(RedisWrite {
            key: format!("subdomain:{}", subdomain),
            value: row.shop_id.clone(),
        });
    }

    writes
}

pub fn domain_writes(row: &DomainEdgeRow) -> Result<Vec<RedisWrite>> {
    let mut writes = vec![
        RedisWrite {
            key: format!("domain:{}", row.name),
            value: serde_json::to_string(&DomainCache {
                shop_id: row.shop_id.clone(),
                verified: row.verified,
            })?,
        },
        RedisWrite {
            key: format!("domain:{}:records", row.name),
            value: serde_json::to_string(&row.records)?,
        },
    ];

    if let Some(token) = row.verification_token.as_deref().filter(|s| !s.is_empty()) {
        writes.push(RedisWrite {
            key: format!("verify:{}", row.name),
            value: token.to_string(),
        });
    }

    Ok(writes)
}

pub fn shop_delta(row: &ShopEdgeRow) -> EdgeDelta {
    EdgeDelta::Shop {
        shop_id: row.shop_id.clone(),
        shard_index: row.shard_index,
        subdomain: row.subdomain.clone(),
        updated_at: row.updated_at,
    }
}

pub fn domain_delta(row: &DomainEdgeRow) -> EdgeDelta {
    EdgeDelta::Domain {
        name: row.name.clone(),
        shop_id: row.shop_id.clone(),
        verified: row.verified,
        records: row.records.clone(),
        updated_at: row.updated_at,
    }
}

pub struct RedisWriter<C = RedisClientConnector> {
    connector: C,
}

pub struct RedisClientConnector {
    client: redis::Client,
}

impl RedisWriter<RedisClientConnector> {
    pub fn new(redis_url: &str) -> Result<Self> {
        Ok(Self {
            connector: RedisClientConnector {
                client: redis::Client::open(redis_url)?,
            },
        })
    }
}

impl<C> RedisWriter<C>
where
    C: RedisConnector,
{
    pub async fn apply_shop(&self, row: &ShopEdgeRow) -> Result<()> {
        self.write_and_publish(shop_writes(row), &shop_delta(row))
            .await
    }

    pub async fn apply_domain(&self, row: &DomainEdgeRow) -> Result<()> {
        self.write_and_publish(domain_writes(row)?, &domain_delta(row))
            .await
    }

    async fn write_and_publish(&self, writes: Vec<RedisWrite>, delta: &EdgeDelta) -> Result<()> {
        let mut conn = self.connector.connection().await?;
        write_and_publish_on(&mut conn, writes, delta).await
    }
}

pub trait RedisConnector {
    type Connection: EdgeRedisConnection;

    async fn connection(&self) -> Result<Self::Connection>;
}

pub trait EdgeRedisConnection {
    async fn get_string(&mut self, key: &str) -> Result<Option<String>>;
    async fn setex(&mut self, key: &str, ttl_secs: usize, value: &str) -> Result<()>;
    async fn expire(&mut self, key: &str, ttl_secs: usize) -> Result<()>;
    async fn publish(&mut self, channel: &str, payload: &str) -> Result<usize>;
}

impl RedisConnector for RedisClientConnector {
    type Connection = redis::aio::MultiplexedConnection;

    async fn connection(&self) -> Result<Self::Connection> {
        Ok(self.client.get_multiplexed_async_connection().await?)
    }
}

impl EdgeRedisConnection for redis::aio::MultiplexedConnection {
    async fn get_string(&mut self, key: &str) -> Result<Option<String>> {
        Ok(self.get(key).await?)
    }

    async fn setex(&mut self, key: &str, ttl_secs: usize, value: &str) -> Result<()> {
        Ok(self.set_ex(key, value, ttl_secs as u64).await?)
    }

    async fn expire(&mut self, key: &str, ttl_secs: usize) -> Result<()> {
        let _: bool = AsyncCommands::expire(self, key, ttl_secs as i64).await?;
        Ok(())
    }

    async fn publish(&mut self, channel: &str, payload: &str) -> Result<usize> {
        Ok(AsyncCommands::publish(self, channel, payload).await?)
    }
}

async fn write_and_publish_on(
    conn: &mut impl EdgeRedisConnection,
    writes: Vec<RedisWrite>,
    delta: &EdgeDelta,
) -> Result<()> {
    let mut changed = false;

    for write in writes {
        let hash_key = value_hash_key(&write.key);
        let hash = value_hash(&write.value);
        let existing_hash = conn.get_string(&hash_key).await?;

        if existing_hash.as_deref() == Some(hash.as_str()) {
            conn.expire(&write.key, EDGE_KEY_TTL_SECS).await?;
            conn.expire(&hash_key, EDGE_KEY_TTL_SECS).await?;
        } else {
            conn.setex(&write.key, EDGE_KEY_TTL_SECS, &write.value)
                .await?;
            conn.setex(&hash_key, EDGE_KEY_TTL_SECS, &hash).await?;
            changed = true;
        }
    }

    if changed {
        let payload = serde_json::to_string(delta)?;
        let _: usize = conn.publish(EDGE_UPDATES_CHANNEL, &payload).await?;
    }

    Ok(())
}

fn value_hash_key(key: &str) -> String {
    format!("{key}:hash")
}

fn value_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn maps_shop_to_shard_and_subdomain_keys() {
        let row = ShopEdgeRow {
            shop_id: "shop_beta".to_string(),
            shard_index: 2,
            subdomain: Some("beta".to_string()),
            updated_at: 101,
        };

        assert_eq!(
            shop_writes(&row),
            vec![
                RedisWrite {
                    key: "shop:shop_beta:shard".to_string(),
                    value: "2".to_string(),
                },
                RedisWrite {
                    key: "subdomain:beta".to_string(),
                    value: "shop_beta".to_string(),
                },
            ]
        );
    }

    #[test]
    fn maps_domain_to_domain_record_and_verify_keys() {
        let row = DomainEdgeRow {
            name: "test-domain.example".to_string(),
            shop_id: "shop_beta".to_string(),
            verified: true,
            verification_token: Some("tok_123".to_string()),
            records: vec![DnsRecordCache {
                record_type: "A".to_string(),
                name: "@".to_string(),
                value: "203.0.113.10".to_string(),
                ttl: 300,
                priority: None,
            }],
            updated_at: 202,
        };

        let writes = domain_writes(&row).unwrap();
        assert_eq!(writes[0].key, "domain:test-domain.example");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&writes[0].value).unwrap(),
            json!({"shop_id": "shop_beta", "verified": true})
        );
        assert_eq!(writes[1].key, "domain:test-domain.example:records");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&writes[1].value).unwrap(),
            json!([{"type": "A", "name": "@", "value": "203.0.113.10", "ttl": 300}])
        );
        assert_eq!(
            writes[2],
            RedisWrite {
                key: "verify:test-domain.example".to_string(),
                value: "tok_123".to_string(),
            }
        );
    }

    #[test]
    fn domain_delta_json_shape_is_stable() {
        let row = DomainEdgeRow {
            name: "test-domain.example".to_string(),
            shop_id: "shop_beta".to_string(),
            verified: false,
            verification_token: None,
            records: vec![DnsRecordCache {
                record_type: "MX".to_string(),
                name: "@".to_string(),
                value: "mail.example".to_string(),
                ttl: 300,
                priority: Some(10),
            }],
            updated_at: 303,
        };

        let value = serde_json::to_value(domain_delta(&row)).unwrap();
        assert_eq!(
            value,
            json!({
                "kind": "domain",
                "name": "test-domain.example",
                "shop_id": "shop_beta",
                "verified": false,
                "records": [{"type": "MX", "name": "@", "value": "mail.example", "ttl": 300, "priority": 10}],
                "updated_at": 303
            })
        );
    }

    #[tokio::test]
    async fn apply_shop_skips_setex_and_publish_when_hash_is_unchanged() {
        let state = Arc::new(Mutex::new(MockRedisState::default()));
        let writer = RedisWriter {
            connector: MockRedisConnector {
                state: Arc::clone(&state),
            },
        };
        let row = ShopEdgeRow {
            shop_id: "shop_beta".to_string(),
            shard_index: 2,
            subdomain: None,
            updated_at: 101,
        };

        writer.apply_shop(&row).await.unwrap();
        writer.apply_shop(&row).await.unwrap();

        let state = state.lock().unwrap();
        assert_eq!(state.data_setex_count, 1);
        assert_eq!(state.publish_count, 1);
        assert_eq!(state.data_expire_count, 1);
        assert_eq!(
            state.values.get("shop:shop_beta:shard").map(String::as_str),
            Some("2")
        );
    }

    #[derive(Clone)]
    struct MockRedisConnector {
        state: Arc<Mutex<MockRedisState>>,
    }

    #[derive(Default)]
    struct MockRedisState {
        values: HashMap<String, String>,
        data_setex_count: usize,
        data_expire_count: usize,
        publish_count: usize,
    }

    struct MockRedisConnection {
        state: Arc<Mutex<MockRedisState>>,
    }

    impl RedisConnector for MockRedisConnector {
        type Connection = MockRedisConnection;

        async fn connection(&self) -> Result<Self::Connection> {
            Ok(MockRedisConnection {
                state: Arc::clone(&self.state),
            })
        }
    }

    impl EdgeRedisConnection for MockRedisConnection {
        async fn get_string(&mut self, key: &str) -> Result<Option<String>> {
            Ok(self.state.lock().unwrap().values.get(key).cloned())
        }

        async fn setex(&mut self, key: &str, _ttl_secs: usize, value: &str) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.values.insert(key.to_string(), value.to_string());
            if !key.ends_with(":hash") {
                state.data_setex_count += 1;
            }
            Ok(())
        }

        async fn expire(&mut self, key: &str, _ttl_secs: usize) -> Result<()> {
            if !key.ends_with(":hash") {
                self.state.lock().unwrap().data_expire_count += 1;
            }
            Ok(())
        }

        async fn publish(&mut self, _channel: &str, _payload: &str) -> Result<usize> {
            self.state.lock().unwrap().publish_count += 1;
            Ok(1)
        }
    }
}
