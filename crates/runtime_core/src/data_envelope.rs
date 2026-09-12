//! Seam contract for the **data-layer envelope** — the request/response shapes
//! exchanged at the storefront's data boundary:
//!
//! ```text
//!   PHPX stdlib and KV clients  ⇄  zega_backend  ⇄  zega-server
//! ```
//!
//! This boundary historically broke exactly the way the seam system exists to
//! prevent: a producer returns `{ok, result}` while the consumer expects
//! `{ok, value}`, or returns a raw value where an envelope was expected. The
//! `zega_backend` networked path (deka #88) normalises three producers
//! (embedded `ZegaManager`, networked `CanonicalClient`, the `zega-server` HTTP
//! API) onto one shape — and *this* contract is the single source of truth for
//! what that shape is. It is also the spec the simplicity rewrite (tana #558)
//! implements: deterministic, declared once, diffed mechanically.
//!
//! Scope: the ops the live storefront exercises — CQL `query`/`execute` and KV
//! `get`/`set`/`del`. The remaining ops (connect/close/exists/incr/expire) grow
//! the same contract as they enter a hot path.
//!
//! Dynamic fields (Cypher params, result rows) follow the same lossy convention
//! as `storefront_envelope` — `serde_json::Value` renders as `Map<String,String>` —
//! because their per-query shape is not statically known; the *envelope* around
//! them is what this contract pins.

use std::collections::BTreeMap;

use crate::seam::{SeamBoundary, SeamContract, SeamDefinition, SeamRecord, SeamType};
use serde::{Deserialize, Serialize};

use crate::storefront_envelope::ToSeam;

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

// ---- ToSeam ---------------------------------------------------------------

fn named(name: &str) -> SeamType {
    SeamType::Named {
        name: name.to_string(),
    }
}

fn record(name: &str, fields: BTreeMap<String, SeamType>) -> SeamDefinition {
    SeamDefinition::Record(SeamRecord {
        name: name.to_string(),
        fields,
    })
}

macro_rules! to_seam_record {
    ($ty:ident { $($field:literal : $fty:ty),+ $(,)? }) => {
        impl ToSeam for $ty {
            fn seam_type() -> SeamType {
                named(stringify!($ty))
            }
            fn seam_definition() -> Option<SeamDefinition> {
                let mut fields = BTreeMap::new();
                $( fields.insert($field.to_string(), <$fty>::seam_type()); )+
                Some(record(stringify!($ty), fields))
            }
            fn seam_definitions() -> Vec<SeamDefinition> {
                Self::seam_definition().into_iter().collect()
            }
        }
    };
}

to_seam_record!(CqlQueryRequest { "cypher": String, "params": serde_json::Value });
to_seam_record!(CqlQueryResponse { "ok": bool, "rows": Vec<serde_json::Value>, "count": i64, "error": Option<String> });
to_seam_record!(CqlExecuteResponse { "ok": bool, "error": Option<String> });
to_seam_record!(KvKeyRequest { "key": String });
to_seam_record!(KvSetRequest { "key": String, "value": String, "ttl": Option<i64> });
to_seam_record!(KvGetResponse { "ok": bool, "value": Option<String>, "error": Option<String> });
to_seam_record!(KvSetResponse { "ok": bool, "error": Option<String> });
to_seam_record!(KvDelResponse { "ok": bool, "deleted": i64, "error": Option<String> });

// ---- Contract -------------------------------------------------------------

/// The canonical data-layer seam contract. One boundary per live op; every
/// producer (embedded / networked / server) and the PHPX consumer are diffed
/// against this single declaration.
pub fn data_backend_contract() -> SeamContract {
    let mut contract = SeamContract::new("data_backend", 1);

    let boundaries = [
        ("cql_query", "CqlQueryRequest", "CqlQueryResponse"),
        ("cql_execute", "CqlQueryRequest", "CqlExecuteResponse"),
        ("kv_get", "KvKeyRequest", "KvGetResponse"),
        ("kv_set", "KvSetRequest", "KvSetResponse"),
        ("kv_del", "KvKeyRequest", "KvDelResponse"),
    ];
    for (function, request, response) in boundaries {
        contract.boundaries.push(SeamBoundary {
            function: function.to_string(),
            request: request.to_string(),
            response: response.to_string(),
        });
    }

    // Definitions: one copy of each referenced record, de-duplicated by name.
    let mut defs: BTreeMap<String, SeamDefinition> = BTreeMap::new();
    for def in [
        CqlQueryRequest::seam_definitions(),
        CqlQueryResponse::seam_definitions(),
        CqlExecuteResponse::seam_definitions(),
        KvKeyRequest::seam_definitions(),
        KvSetRequest::seam_definitions(),
        KvGetResponse::seam_definitions(),
        KvSetResponse::seam_definitions(),
        KvDelResponse::seam_definitions(),
    ]
    .into_iter()
    .flatten()
    {
        let name = match &def {
            SeamDefinition::Record(r) => r.name.clone(),
            SeamDefinition::Enum(e) => e.name.clone(),
        };
        defs.entry(name).or_insert(def);
    }
    contract.definitions = defs.into_values().collect();

    contract
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn data_backend_contract_round_trips_json() {
        let contract = data_backend_contract();
        let actual = serde_json::to_value(&contract).unwrap();

        assert_eq!(
            actual,
            json!({
                "format": "seam.contract@1",
                "name": "data_backend",
                "version": 1,
                "boundaries": [
                    { "function": "cql_query", "request": "CqlQueryRequest", "response": "CqlQueryResponse" },
                    { "function": "cql_execute", "request": "CqlQueryRequest", "response": "CqlExecuteResponse" },
                    { "function": "kv_get", "request": "KvKeyRequest", "response": "KvGetResponse" },
                    { "function": "kv_set", "request": "KvSetRequest", "response": "KvSetResponse" },
                    { "function": "kv_del", "request": "KvKeyRequest", "response": "KvDelResponse" }
                ],
                "definitions": [
                    { "kind": "record", "name": "CqlExecuteResponse", "fields": {
                        "error": { "kind": "option", "item": { "kind": "primitive", "name": "String" } },
                        "ok": { "kind": "primitive", "name": "Bool" }
                    }},
                    { "kind": "record", "name": "CqlQueryRequest", "fields": {
                        "cypher": { "kind": "primitive", "name": "String" },
                        "params": { "kind": "map",
                            "key": { "kind": "primitive", "name": "String" },
                            "value": { "kind": "primitive", "name": "String" } }
                    }},
                    { "kind": "record", "name": "CqlQueryResponse", "fields": {
                        "count": { "kind": "primitive", "name": "Int" },
                        "error": { "kind": "option", "item": { "kind": "primitive", "name": "String" } },
                        "ok": { "kind": "primitive", "name": "Bool" },
                        "rows": { "kind": "list", "item": { "kind": "map",
                            "key": { "kind": "primitive", "name": "String" },
                            "value": { "kind": "primitive", "name": "String" } } }
                    }},
                    { "kind": "record", "name": "KvDelResponse", "fields": {
                        "deleted": { "kind": "primitive", "name": "Int" },
                        "error": { "kind": "option", "item": { "kind": "primitive", "name": "String" } },
                        "ok": { "kind": "primitive", "name": "Bool" }
                    }},
                    { "kind": "record", "name": "KvGetResponse", "fields": {
                        "error": { "kind": "option", "item": { "kind": "primitive", "name": "String" } },
                        "ok": { "kind": "primitive", "name": "Bool" },
                        "value": { "kind": "option", "item": { "kind": "primitive", "name": "String" } }
                    }},
                    { "kind": "record", "name": "KvKeyRequest", "fields": {
                        "key": { "kind": "primitive", "name": "String" }
                    }},
                    { "kind": "record", "name": "KvSetRequest", "fields": {
                        "key": { "kind": "primitive", "name": "String" },
                        "ttl": { "kind": "option", "item": { "kind": "primitive", "name": "Int" } },
                        "value": { "kind": "primitive", "name": "String" }
                    }},
                    { "kind": "record", "name": "KvSetResponse", "fields": {
                        "error": { "kind": "option", "item": { "kind": "primitive", "name": "String" } },
                        "ok": { "kind": "primitive", "name": "Bool" }
                    }}
                ]
            })
        );
    }
}
