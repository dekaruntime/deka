//! Request and response shapes for the data-layer boundary.

use serde::{Deserialize, Serialize};

// ---- CQL ------------------------------------------------------------------

/// Zega CQL bridge → backend: run a CQL query (`query` = rows wanted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CqlQueryRequest {
    pub cypher: String,
    pub params: serde_json::Value,
}

/// Response to `query`: a list of result rows plus the count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CqlQueryResponse {
    pub ok: bool,
    pub rows: Vec<serde_json::Value>,
    pub count: i64,
    #[serde(default)]
    pub error: Option<String>,
}

/// Response to `execute` (no rows wanted — write/DDL).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CqlExecuteResponse {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

// ---- KV -------------------------------------------------------------------

/// KV client → backend: a keyed read (`get`) or delete (`del`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KvKeyRequest {
    pub key: String,
}

/// KV client → backend: a keyed write (`set`), optional TTL in seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KvSetRequest {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub ttl: Option<i64>,
}

/// Response to `get`: the stored value, or `None` on miss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KvGetResponse {
    pub ok: bool,
    pub value: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Response to `set`: bare acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KvSetResponse {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Response to `del`: number of keys removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KvDelResponse {
    pub ok: bool,
    pub deleted: i64,
    #[serde(default)]
    pub error: Option<String>,
}
