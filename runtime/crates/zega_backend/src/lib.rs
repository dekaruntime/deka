use deno_core::op2;
use serde_json::{Value as JsonValue, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use zega_core::{Value as ZegaValue, Zega};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Backend {
    Neo4j,
    Zega,
}

pub struct ZegaManager {
    tenants_dir: PathBuf,
    instances: RwLock<HashMap<String, Arc<Zega>>>,
}

impl ZegaManager {
    pub fn new(tenants_dir: impl Into<PathBuf>) -> Self {
        Self {
            tenants_dir: tenants_dir.into(),
            instances: RwLock::new(HashMap::new()),
        }
    }

    pub fn backend_for(&self, shop_id: &str) -> Backend {
        if !valid_shop_id(shop_id) {
            return Backend::Zega;
        }
        let marker = self.tenants_dir.join(shop_id).join("backend");
        // Zega is THE backend for every shop. Neo4j is opt-in only — kept solely as a
        // parity oracle via an explicit `backend=neo4j` marker. There are no Neo4j customers.
        match std::fs::read_to_string(marker) {
            Ok(value) if value.trim().eq_ignore_ascii_case("neo4j") => Backend::Neo4j,
            _ => Backend::Zega,
        }
    }

    pub fn get(&self, shop_id: &str) -> Result<Arc<Zega>, String> {
        if !valid_shop_id(shop_id) {
            return Err("invalid shop_id".to_string());
        }
        if let Some(zega) = self.instances.read().unwrap().get(shop_id).cloned() {
            return Ok(zega);
        }

        let mut instances = self.instances.write().unwrap();
        if let Some(zega) = instances.get(shop_id).cloned() {
            return Ok(zega);
        }

        let path = self.tenants_dir.join(shop_id).join("zega");
        let path = path
            .to_str()
            .ok_or_else(|| "tenant Zega path is not valid UTF-8".to_string())?;
        let zega = Arc::new(
            Zega::open(path)
                .build()
                .map_err(|err| format!("failed to open Zega for {shop_id}: {err}"))?,
        );
        instances.insert(shop_id.to_string(), Arc::clone(&zega));
        Ok(zega)
    }

    pub fn cql_call(&self, shop_id: &str, action: &str, args: &JsonValue) -> JsonValue {
        match action {
            "connect" => json!({ "ok": true, "handle": 1 }),
            "close" => json!({ "ok": true }),
            "query" => self.query(shop_id, args, true),
            "execute" => self.query(shop_id, args, false),
            _ => json!({ "ok": false, "error": format!("unknown Zega CQL action '{action}'") }),
        }
    }

    fn query(&self, shop_id: &str, args: &JsonValue, collect_rows: bool) -> JsonValue {
        let query = match args.get("cypher").and_then(JsonValue::as_str) {
            Some(query) => query,
            None => return json!({ "ok": false, "error": "missing 'cypher' field" }),
        };
        let params = match json_params(args.get("params").unwrap_or(&JsonValue::Null)) {
            Ok(params) => params,
            Err(error) => return json!({ "ok": false, "error": error }),
        };
        let zega = match self.get(shop_id) {
            Ok(zega) => zega,
            Err(error) => return json!({ "ok": false, "error": error }),
        };

        match zega.query(query, params) {
            Ok(rows) if collect_rows => {
                let rows: Vec<JsonValue> = rows
                    .into_iter()
                    .map(|row| {
                        JsonValue::Object(
                            row.fields
                                .into_iter()
                                .map(|(key, value)| (key, zega_to_json(value)))
                                .collect(),
                        )
                    })
                    .collect();
                json!({ "ok": true, "rows": rows, "count": rows.len() })
            }
            Ok(_) => json!({ "ok": true }),
            Err(error) => json!({ "ok": false, "error": error.to_string() }),
        }
    }

    pub fn kv_call(&self, shop_id: &str, action: &str, args: &JsonValue) -> JsonValue {
        if matches!(
            action.to_ascii_lowercase().as_str(),
            "flush" | "flushdb" | "flushall"
        ) {
            return json!({ "ok": false, "error": "redis admin action blocked in user pool" });
        }
        match action {
            "connect" => return json!({ "ok": true, "handle": 1 }),
            "close" => return json!({ "ok": true }),
            _ => {}
        }
        let key = match args.get("key").and_then(JsonValue::as_str) {
            Some(key) => key,
            None => return json!({ "ok": false, "error": "missing 'key'" }),
        };
        let zega = match self.get(shop_id) {
            Ok(zega) => zega,
            Err(error) => return json!({ "ok": false, "error": error }),
        };

        match action {
            "get" => json!({ "ok": true, "value": zega.kv_get(key).map(zega_to_json) }),
            "set" => {
                let value = match args.get("value") {
                    Some(value) => match json_to_zega(value) {
                        Ok(value) => value,
                        Err(error) => return json!({ "ok": false, "error": error }),
                    },
                    None => return json!({ "ok": false, "error": "missing 'value'" }),
                };
                let ttl = args.get("ttl").and_then(JsonValue::as_u64);
                result_ok(zega.kv_set(key.to_string(), value, ttl))
            }
            "del" => match zega.kv_del(key) {
                Ok(deleted) => json!({ "ok": true, "deleted": i64::from(deleted) }),
                Err(error) => json!({ "ok": false, "error": error.to_string() }),
            },
            "exists" => json!({ "ok": true, "exists": zega.kv_get(key).is_some() }),
            "incr" if args.get("by").and_then(JsonValue::as_i64).unwrap_or(1) == 1 => {
                let mut params = HashMap::new();
                params.insert("key".to_string(), ZegaValue::String(key.to_string()));
                match zega.query("INCR KEY $key", params) {
                    Ok(rows) => json!({
                        "ok": true,
                        "value": rows.first().and_then(|row| row.fields.get("value")).cloned().map(zega_to_json)
                    }),
                    Err(error) => json!({ "ok": false, "error": error.to_string() }),
                }
            }
            "incr" => {
                json!({ "ok": false, "error": "Zega currently supports increment by 1 only" })
            }
            _ => {
                json!({ "ok": false, "error": format!("Redis action '{action}' is not supported by Zega") })
            }
        }
    }
}

fn valid_shop_id(shop_id: &str) -> bool {
    !shop_id.is_empty()
        && shop_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn json_params(value: &JsonValue) -> Result<HashMap<String, ZegaValue>, String> {
    match value {
        JsonValue::Null => Ok(HashMap::new()),
        JsonValue::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), json_to_zega(value)?)))
            .collect(),
        _ => Err("params must be an object".to_string()),
    }
}

fn json_to_zega(value: &JsonValue) -> Result<ZegaValue, String> {
    Ok(match value {
        JsonValue::Null => ZegaValue::Null,
        JsonValue::Bool(value) => ZegaValue::Bool(*value),
        JsonValue::Number(value) if value.is_i64() => ZegaValue::Int(value.as_i64().unwrap()),
        JsonValue::Number(value) => ZegaValue::from_f64(
            value
                .as_f64()
                .ok_or_else(|| "number is outside Zega's supported range".to_string())?,
        ),
        JsonValue::String(value) => ZegaValue::String(value.clone()),
        JsonValue::Array(values) => {
            ZegaValue::List(values.iter().map(json_to_zega).collect::<Result<_, _>>()?)
        }
        JsonValue::Object(values) => ZegaValue::Map(
            values
                .iter()
                .map(|(key, value)| Ok((key.clone(), json_to_zega(value)?)))
                .collect::<Result<_, String>>()?,
        ),
    })
}

fn zega_to_json(value: ZegaValue) -> JsonValue {
    match value {
        ZegaValue::Null => JsonValue::Null,
        ZegaValue::Bool(value) => JsonValue::Bool(value),
        ZegaValue::Int(value) => json!(value),
        ZegaValue::Float(bits) => json!(f64::from_bits(bits)),
        ZegaValue::String(value) => JsonValue::String(value),
        ZegaValue::List(values) => JsonValue::Array(values.into_iter().map(zega_to_json).collect()),
        ZegaValue::Map(values) => JsonValue::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, zega_to_json(value)))
                .collect(),
        ),
    }
}

fn result_ok(result: zega_core::Result<()>) -> JsonValue {
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(error) => json!({ "ok": false, "error": error.to_string() }),
    }
}

fn global_manager() -> &'static ZegaManager {
    static MANAGER: OnceLock<ZegaManager> = OnceLock::new();
    MANAGER.get_or_init(|| {
        let tenants_dir =
            std::env::var_os("DEKA_TENANTS_DIR").unwrap_or_else(|| "store/tenants".into());
        ZegaManager::new(Path::new(&tenants_dir))
    })
}

#[op2]
#[string]
fn op_zega_backend(#[string] shop_id: String) -> String {
    match global_manager().backend_for(&shop_id) {
        Backend::Zega => "zega",
        Backend::Neo4j => "neo4j",
    }
    .to_string()
}

#[op2]
#[serde]
fn op_zega_cql_call(
    #[string] shop_id: String,
    #[string] action: String,
    #[serde] args: JsonValue,
) -> JsonValue {
    global_manager().cql_call(&shop_id, &action, &args)
}

#[op2]
#[serde]
fn op_zega_kv_call(
    #[string] shop_id: String,
    #[string] action: String,
    #[serde] args: JsonValue,
) -> JsonValue {
    global_manager().kv_call(&shop_id, &action, &args)
}

deno_core::extension!(
    zega_backend_extension,
    ops = [op_zega_backend, op_zega_cql_call, op_zega_kv_call],
);

pub fn init() -> deno_core::Extension {
    zega_backend_extension::init()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_defaults_to_zega_and_reads_neo4j_marker() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ZegaManager::new(dir.path());
        // No marker -> Zega. Zega is the default (and only) production backend.
        assert_eq!(manager.backend_for("shop_existing"), Backend::Zega);

        let tenant = dir.path().join("shop_neo");
        std::fs::create_dir_all(&tenant).unwrap();
        std::fs::write(tenant.join("backend"), "neo4j\n").unwrap();
        assert_eq!(manager.backend_for("shop_neo"), Backend::Neo4j);

        std::fs::write(tenant.join("backend"), "zega\n").unwrap();
        assert_eq!(manager.backend_for("shop_neo"), Backend::Zega);
    }

    #[test]
    fn zega_cql_route_matches_run_cql_result_shape_and_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let tenant = dir.path().join("shop_new");
        std::fs::create_dir_all(&tenant).unwrap();
        std::fs::write(tenant.join("backend"), "zega").unwrap();

        let manager = ZegaManager::new(dir.path());
        let created = manager.cql_call(
            "shop_new",
            "execute",
            &json!({
                "cypher": "CREATE (p:Product {sku: $sku, name: $name})",
                "params": { "sku": "sku-1", "name": "Hat" }
            }),
        );
        assert_eq!(created, json!({ "ok": true }));

        drop(manager);
        let recovered = ZegaManager::new(dir.path()).cql_call(
            "shop_new",
            "query",
            &json!({
                "cypher": "MATCH (p:Product {sku: $sku}) RETURN p.name AS name",
                "params": { "sku": "sku-1" }
            }),
        );
        assert_eq!(
            recovered,
            json!({ "ok": true, "rows": [{ "name": "Hat" }], "count": 1 })
        );
    }

    #[test]
    fn zega_kv_route_is_per_shop() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ZegaManager::new(dir.path());
        assert_eq!(
            manager.kv_call("shop_a", "set", &json!({ "key": "cart", "value": "a" })),
            json!({ "ok": true })
        );
        assert_eq!(
            manager.kv_call("shop_a", "get", &json!({ "key": "cart" })),
            json!({ "ok": true, "value": "a" })
        );
        assert_eq!(
            manager.kv_call("shop_b", "get", &json!({ "key": "cart" })),
            json!({ "ok": true, "value": null })
        );
        assert_eq!(
            manager.kv_call("shop_a", "close", &json!({ "handle": 1 })),
            json!({ "ok": true })
        );
    }

    #[test]
    fn zega_kv_route_rejects_redis_admin_wipe_verbs() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ZegaManager::new(dir.path());
        for action in ["flush", "FLUSHDB", "flushall"] {
            assert_eq!(
                manager.kv_call("shop_a", action, &json!({ "handle": 1 })),
                json!({ "ok": false, "error": "redis admin action blocked in user pool" }),
                "{action} must not expose an unprefixed wipe through Zega"
            );
        }
    }
}
