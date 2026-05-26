use crate::redis_writer::{DnsRecordCache, DomainEdgeRow, ShopEdgeRow};
use anyhow::{Context, Result, anyhow};
use neo4rs::{Graph, Row, query};
use std::collections::BTreeMap;

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

const SHOP_FIELDS: &str = "\
coalesce(s.id, s.shop_id) AS shop_id, \
coalesce(s.shard_index, 0) AS shard_index, \
coalesce(s.subdomain, s.slug) AS subdomain, \
coalesce(s.updated_at, 0) AS updated_at";

const DOMAIN_FIELDS: &str = "\
d.name AS domain_name, \
d.shop_id AS shop_id, \
coalesce(d.verified, false) AS verified, \
d.verification_token AS verification_token, \
coalesce(d.updated_at, 0) AS domain_updated_at, \
coalesce(max_record_updated, coalesce(d.updated_at, 0)) AS updated_at, \
r.`type` AS record_type, \
r.name AS record_name, \
r.value AS record_value, \
coalesce(r.ttl, 300) AS record_ttl, \
r.priority AS record_priority";

pub struct Neo4jReader {
    graph: Graph,
}

impl Neo4jReader {
    pub async fn connect(uri: &str, user: &str, password: &str) -> Result<Self> {
        let config = neo4rs::ConfigBuilder::default()
            .uri(uri)
            .user(user)
            .password(password)
            .db("neo4j")
            .build()
            .context("failed to build neo4j config")?;

        let graph = tokio::time::timeout(CONNECT_TIMEOUT, Graph::connect(config))
            .await
            .context("neo4j connect timed out")?
            .context("failed to connect to neo4j")?;

        Ok(Self { graph })
    }

    pub async fn full_sync(&self) -> Result<EdgeSnapshot> {
        Ok(EdgeSnapshot {
            shops: self.all_shops().await?,
            domains: self.all_domains().await?,
        })
    }

    pub async fn changes_since(&self, last_seen_timestamp: i64) -> Result<EdgeSnapshot> {
        Ok(EdgeSnapshot {
            shops: self.changed_shops(last_seen_timestamp).await?,
            domains: self.changed_domains(last_seen_timestamp).await?,
        })
    }

    async fn all_shops(&self) -> Result<Vec<ShopEdgeRow>> {
        let cypher = format!("MATCH (s:Shop) RETURN {SHOP_FIELDS} ORDER BY shop_id");
        self.query_shops(query(&cypher)).await
    }

    async fn changed_shops(&self, last_seen_timestamp: i64) -> Result<Vec<ShopEdgeRow>> {
        let cypher = format!(
            "MATCH (s:Shop) \
             WHERE coalesce(s.updated_at, 0) > $lastSeen \
             RETURN {SHOP_FIELDS} ORDER BY updated_at, shop_id"
        );
        self.query_shops(query(&cypher).param("lastSeen", last_seen_timestamp))
            .await
    }

    async fn all_domains(&self) -> Result<Vec<DomainEdgeRow>> {
        let cypher = format!(
            "MATCH (d:Domain) \
             OPTIONAL MATCH (d)-[:HAS_RECORD]->(changed:DnsRecord) \
             WITH d, max(coalesce(changed.updated_at, 0)) AS max_record_updated \
             OPTIONAL MATCH (d)-[:HAS_RECORD]->(r:DnsRecord) \
             RETURN {DOMAIN_FIELDS} \
             ORDER BY domain_name, record_type, record_name, record_value"
        );
        self.query_domains(query(&cypher)).await
    }

    async fn changed_domains(&self, last_seen_timestamp: i64) -> Result<Vec<DomainEdgeRow>> {
        let cypher = format!(
            "MATCH (d:Domain) \
             OPTIONAL MATCH (d)-[:HAS_RECORD]->(changed:DnsRecord) \
             WITH d, max(coalesce(changed.updated_at, 0)) AS max_record_updated \
             WHERE coalesce(d.updated_at, 0) > $lastSeen \
                OR coalesce(max_record_updated, 0) > $lastSeen \
             OPTIONAL MATCH (d)-[:HAS_RECORD]->(r:DnsRecord) \
             RETURN {DOMAIN_FIELDS} \
             ORDER BY updated_at, domain_name, record_type, record_name, record_value"
        );
        self.query_domains(query(&cypher).param("lastSeen", last_seen_timestamp))
            .await
    }

    async fn query_shops(&self, q: neo4rs::Query) -> Result<Vec<ShopEdgeRow>> {
        let mut result = tokio::time::timeout(QUERY_TIMEOUT, self.graph.execute(q))
            .await
            .context("neo4j shop query timed out")?
            .context("neo4j shop query failed")?;

        let mut shops = Vec::new();
        while let Some(row) = result.next().await.context("failed reading shop row")? {
            if let Some(shop) = shop_from_row(&row)? {
                shops.push(shop);
            }
        }
        Ok(shops)
    }

    async fn query_domains(&self, q: neo4rs::Query) -> Result<Vec<DomainEdgeRow>> {
        let mut result = tokio::time::timeout(QUERY_TIMEOUT, self.graph.execute(q))
            .await
            .context("neo4j domain query timed out")?
            .context("neo4j domain query failed")?;

        let mut grouped: BTreeMap<String, DomainEdgeRow> = BTreeMap::new();
        while let Some(row) = result.next().await.context("failed reading domain row")? {
            merge_domain_row(&mut grouped, &row)?;
        }
        Ok(grouped.into_values().collect())
    }
}

#[derive(Debug, Clone, Default)]
pub struct EdgeSnapshot {
    pub shops: Vec<ShopEdgeRow>,
    pub domains: Vec<DomainEdgeRow>,
}

fn shop_from_row(row: &Row) -> Result<Option<ShopEdgeRow>> {
    let Some(shop_id) = optional_string(row, "shop_id")? else {
        return Ok(None);
    };

    Ok(Some(ShopEdgeRow {
        shop_id,
        shard_index: row.get::<i64>("shard_index").unwrap_or(0),
        subdomain: optional_string(row, "subdomain")?,
        updated_at: row.get::<i64>("updated_at").unwrap_or(0),
    }))
}

fn merge_domain_row(grouped: &mut BTreeMap<String, DomainEdgeRow>, row: &Row) -> Result<()> {
    let name: String = row
        .get("domain_name")
        .map_err(|e| anyhow!("domain row missing domain_name: {e}"))?;
    let shop_id: String = row
        .get("shop_id")
        .map_err(|e| anyhow!("domain row missing shop_id for {name}: {e}"))?;

    let entry = grouped
        .entry(name.clone())
        .or_insert_with(|| DomainEdgeRow {
            name,
            shop_id,
            verified: row.get::<bool>("verified").unwrap_or(false),
            verification_token: optional_string(row, "verification_token").ok().flatten(),
            records: Vec::new(),
            updated_at: row.get::<i64>("updated_at").unwrap_or(0),
        });

    if let Some(record) = record_from_row(row)? {
        entry.records.push(record);
    }

    Ok(())
}

fn record_from_row(row: &Row) -> Result<Option<DnsRecordCache>> {
    let Some(record_type) = optional_string(row, "record_type")? else {
        return Ok(None);
    };
    let Some(name) = optional_string(row, "record_name")? else {
        return Ok(None);
    };
    let Some(value) = optional_string(row, "record_value")? else {
        return Ok(None);
    };

    Ok(Some(DnsRecordCache {
        record_type,
        name,
        value,
        ttl: row.get::<i64>("record_ttl").unwrap_or(300),
        priority: optional_i64(row, "record_priority")?,
    }))
}

fn optional_string(row: &Row, key: &str) -> Result<Option<String>> {
    match row.get::<Option<String>>(key) {
        Ok(value) => Ok(value),
        Err(_) => match row.get::<String>(key) {
            Ok(value) => Ok(Some(value)),
            Err(_) => Ok(None),
        },
    }
}

fn optional_i64(row: &Row, key: &str) -> Result<Option<i64>> {
    match row.get::<Option<i64>>(key) {
        Ok(value) => Ok(value),
        Err(_) => match row.get::<i64>(key) {
            Ok(value) => Ok(Some(value)),
            Err(_) => Ok(None),
        },
    }
}
