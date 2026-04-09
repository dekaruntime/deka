// Minimal PHP runtime module - no heavy dependencies

use bumpalo::Bump;
use deno_core::op2;
use mysql::prelude::Queryable;
use mysql::{OptsBuilder, Params as MyParams, Pool as MyPool, Value as MyValue};
use native_tls::{TlsConnector, TlsStream};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use php_rs::parser::ast::{ClassKind, ClassMember, Program, Stmt, Type as AstType};
use php_rs::parser::lexer::Lexer;
use php_rs::parser::lexer::token::Token;
use php_rs::parser::parser::{Parser, ParserMode, detect_parser_mode};
use bytes::BytesMut;
use postgres::{
    types::{to_sql_checked, IsNull, ToSql, Type as PgType},
    Client, NoTls,
};
use std::error::Error as StdError;
use serde_json::{Map, Value};
use prost::Message as ProstMessage;
use runtime_core::security_policy::{RuleList, SecurityPolicy, parse_deka_security_policy};
use rusqlite::types::ValueRef as SqliteValueRef;
use rusqlite::{Connection as SqliteConnection, params_from_iter as sqlite_params_from_iter};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fs::{File as StdFile, OpenOptions};
use std::io::{IsTerminal, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use wit_parser::{Resolve, Results, Type, TypeDefKind, TypeId, WorldItem, WorldKey};

/// Embedded PHP WASM binary produced by the `php-rs` crate.
mod proto {
    pub mod bridge_v1 {
        include!(concat!(env!("OUT_DIR"), "/deka.bridge.v1.rs"));
    }
}

#[derive(serde::Serialize)]
struct PhpDirEntry {
    name: String,
    is_dir: bool,
    is_file: bool,
}

#[derive(serde::Serialize)]
struct WitSchema {
    world: String,
    functions: Vec<WitFunction>,
    interfaces: Vec<WitInterface>,
}

#[derive(serde::Serialize)]
struct WitInterface {
    name: String,
    functions: Vec<WitFunction>,
}

#[derive(serde::Serialize)]
struct WitFunction {
    name: String,
    params: Vec<WitParam>,
    result: Option<WitType>,
}

#[derive(serde::Serialize)]
struct WitParam {
    name: String,
    #[serde(rename = "type")]
    ty: WitType,
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WitType {
    Bool,
    U8,
    U16,
    U32,
    U64,
    S8,
    S16,
    S32,
    S64,
    F32,
    F64,
    Char,
    String,
    List {
        element: Box<WitType>,
    },
    Record {
        fields: Vec<WitField>,
    },
    Tuple {
        items: Vec<WitType>,
    },
    Option {
        some: Box<WitType>,
    },
    Result {
        ok: Option<Box<WitType>>,
        err: Option<Box<WitType>>,
    },
    Enum {
        cases: Vec<String>,
    },
    Flags {
        flags: Vec<String>,
    },
    Variant {
        cases: Vec<WitVariantCase>,
    },
    Resource,
    Unsupported {
        detail: String,
    },
}

#[derive(serde::Serialize)]
struct WitField {
    name: String,
    #[serde(rename = "type")]
    ty: WitType,
}

#[derive(serde::Serialize)]
struct WitVariantCase {
    name: String,
    #[serde(rename = "type")]
    ty: Option<WitType>,
}

#[derive(serde::Serialize, Clone)]
#[allow(dead_code)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BridgeType {
    Unknown,
    Mixed,
    Primitive {
        name: String,
    },
    Array {
        element: Option<Box<BridgeType>>,
    },
    Object,
    ObjectShape {
        fields: Vec<BridgeField>,
    },
    Struct {
        name: String,
        fields: Vec<BridgeField>,
    },
    Union {
        types: Vec<BridgeType>,
    },
    Option {
        inner: Option<Box<BridgeType>>,
    },
    Result {
        ok: Option<Box<BridgeType>>,
        err: Option<Box<BridgeType>>,
    },
    Applied {
        base: String,
        args: Vec<BridgeType>,
    },
}

#[derive(serde::Serialize, Clone)]
struct BridgeField {
    name: String,
    #[serde(rename = "type")]
    ty: BridgeType,
    optional: bool,
}

#[derive(serde::Serialize)]
struct BridgeParam {
    #[serde(rename = "type")]
    ty: Option<BridgeType>,
    required: bool,
    variadic: bool,
}

#[derive(serde::Serialize)]
struct BridgeFunction {
    params: Vec<BridgeParam>,
    #[serde(rename = "return")]
    return_type: Option<BridgeType>,
    variadic: bool,
}

#[derive(serde::Serialize)]
struct BridgeStruct {
    fields: Vec<BridgeField>,
}

#[derive(serde::Serialize)]
struct BridgeModuleTypes {
    functions: HashMap<String, BridgeFunction>,
    structs: HashMap<String, BridgeStruct>,
}

struct DbConn {
    key: String,
    config: DbDriverConfig,
}

#[derive(Clone)]
struct PgConnConfig {
    host: String,
    port: u16,
    database: String,
    user: String,
    password: String,
}

#[derive(Clone)]
struct SqliteConnConfig {
    path: String,
}

#[derive(Clone)]
struct MysqlConnConfig {
    host: String,
    port: u16,
    database: String,
    user: String,
    password: String,
}

#[derive(Clone)]
enum DbDriverConfig {
    Postgres(PgConnConfig),
    Sqlite(SqliteConnConfig),
    Mysql(MysqlConnConfig),
}

impl DbDriverConfig {
    fn driver_name(&self) -> &'static str {
        match self {
            DbDriverConfig::Postgres(_) => "postgres",
            DbDriverConfig::Sqlite(_) => "sqlite",
            DbDriverConfig::Mysql(_) => "mysql",
        }
    }
}

#[derive(Clone, serde::Serialize)]
struct DbMetric {
    calls: u64,
    errors: u64,
    total_ms: u64,
}

struct DbState {
    next_handle: u64,
    handles: HashMap<u64, DbConn>,
    key_to_handle: HashMap<String, u64>,
    statement_cache: HashMap<u64, HashSet<String>>,
    statement_cache_hits: u64,
    statement_cache_misses: u64,
    metrics: HashMap<String, DbMetric>,
}

impl DbState {
    fn new() -> Self {
        Self {
            next_handle: 1,
            handles: HashMap::new(),
            key_to_handle: HashMap::new(),
            statement_cache: HashMap::new(),
            statement_cache_hits: 0,
            statement_cache_misses: 0,
            metrics: HashMap::new(),
        }
    }

    fn record_metric(&mut self, action: &str, driver: &str, elapsed_ms: u64, is_error: bool) {
        let key = format!("{}:{}", action, driver);
        let metric = self.metrics.entry(key).or_insert(DbMetric {
            calls: 0,
            errors: 0,
            total_ms: 0,
        });
        metric.calls += 1;
        if is_error {
            metric.errors += 1;
        }
        metric.total_ms = metric.total_ms.saturating_add(elapsed_ms);
    }

    fn touch_statement_cache(&mut self, handle: u64, sql: &str) {
        let entry = self.statement_cache.entry(handle).or_default();
        if entry.contains(sql) {
            self.statement_cache_hits = self.statement_cache_hits.saturating_add(1);
            return;
        }
        entry.insert(sql.to_string());
        self.statement_cache_misses = self.statement_cache_misses.saturating_add(1);
    }

    fn statement_cache_entries(&self) -> u64 {
        self.statement_cache
            .values()
            .map(|set| set.len() as u64)
            .sum::<u64>()
    }
}

static DB_STATE: OnceLock<Mutex<DbState>> = OnceLock::new();

fn db_state() -> &'static Mutex<DbState> {
    DB_STATE.get_or_init(|| Mutex::new(DbState::new()))
}

enum NetConn {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
}

struct NetState {
    next_handle: u64,
    handles: HashMap<u64, NetConn>,
}

impl NetState {
    fn new() -> Self {
        Self {
            next_handle: 1,
            handles: HashMap::new(),
        }
    }
}

static NET_STATE: OnceLock<Mutex<NetState>> = OnceLock::new();

fn net_state() -> &'static Mutex<NetState> {
    NET_STATE.get_or_init(|| Mutex::new(NetState::new()))
}

struct FsState {
    next_handle: u64,
    handles: HashMap<u64, StdFile>,
}

impl FsState {
    fn new() -> Self {
        Self {
            next_handle: 1,
            handles: HashMap::new(),
        }
    }
}

static FS_STATE: OnceLock<Mutex<FsState>> = OnceLock::new();

fn fs_state() -> &'static Mutex<FsState> {
    FS_STATE.get_or_init(|| Mutex::new(FsState::new()))
}

#[derive(Clone, serde::Serialize)]
struct BridgeProtoMetric {
    calls: u64,
    total_req_bytes: u64,
    total_resp_bytes: u64,
    total_us: u64,
    avg_us: u64,
}

static BRIDGE_PROTO_METRICS: OnceLock<Mutex<HashMap<String, BridgeProtoMetric>>> = OnceLock::new();

fn bridge_proto_metrics() -> &'static Mutex<HashMap<String, BridgeProtoMetric>> {
    BRIDGE_PROTO_METRICS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record_bridge_proto_metric(kind: &str, req_len: usize, resp_len: usize, elapsed_us: u64) {
    if let Ok(mut metrics) = bridge_proto_metrics().lock() {
        let metric = metrics
            .entry(kind.to_string())
            .or_insert(BridgeProtoMetric {
                calls: 0,
                total_req_bytes: 0,
                total_resp_bytes: 0,
                total_us: 0,
                avg_us: 0,
            });
        metric.calls += 1;
        metric.total_req_bytes = metric.total_req_bytes.saturating_add(req_len as u64);
        metric.total_resp_bytes = metric.total_resp_bytes.saturating_add(resp_len as u64);
        metric.total_us = metric.total_us.saturating_add(elapsed_us);
        metric.avg_us = if metric.calls == 0 {
            0
        } else {
            metric.total_us / metric.calls
        };
    }
}

fn sanitize_conn_value(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

fn with_pg_client<T>(
    cfg: PgConnConfig,
    f: impl FnOnce(&mut Client) -> Result<T, deno_core::error::CoreError> + Send + 'static,
) -> Result<T, deno_core::error::CoreError>
where
    T: Send + 'static,
{
    std::thread::spawn(move || {
        let host = sanitize_conn_value(&cfg.host);
        let user = sanitize_conn_value(&cfg.user);
        let database = sanitize_conn_value(&cfg.database);
        let password = sanitize_conn_value(&cfg.password);

        let mut dsn = format!(
            "host={} port={} user={} dbname={}",
            host, cfg.port, user, database
        );
        if !password.is_empty() {
            dsn.push_str(" password=");
            dsn.push_str(&password);
        }

        let url = if password.is_empty() {
            format!("postgres://{}@{}:{}/{}", user, host, cfg.port, database)
        } else {
            format!(
                "postgres://{}:{}@{}:{}/{}",
                user, password, host, cfg.port, database
            )
        };

        let mut client = match Client::connect(&dsn, NoTls) {
            Ok(client) => client,
            Err(err_dsn) => Client::connect(&url, NoTls).map_err(|err_url| {
                deno_core::error::CoreError::from(std::io::Error::other(format!(
                    "postgres connect failed: {} (dsn={}); fallback failed: {} (url={})",
                    err_dsn, dsn, err_url, url
                )))
            })?,
        };

        f(&mut client)
    })
    .join()
    .map_err(|_| {
        deno_core::error::CoreError::from(std::io::Error::other("db worker thread panicked"))
    })?
}

fn json_to_pg_param(value: &serde_json::Value) -> Box<dyn ToSql + Sync> {
    match value {
        serde_json::Value::Null => Box::new(PgNullParam),
        serde_json::Value::Bool(v) => Box::new(*v),
        serde_json::Value::Number(v) => Box::new(PgNumericParam::from_number(v)),
        serde_json::Value::String(v) => Box::new(PgStringParam(v.clone())),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => Box::new(value.to_string()),
    }
}

#[derive(Debug)]
enum PgNumericParam {
    I64(i64),
    U64(u64),
    F64(f64),
}

impl PgNumericParam {
    fn from_number(value: &serde_json::Number) -> Self {
        if let Some(i) = value.as_i64() {
            return PgNumericParam::I64(i);
        }
        if let Some(u) = value.as_u64() {
            return PgNumericParam::U64(u);
        }
        PgNumericParam::F64(value.as_f64().unwrap_or(0.0))
    }
}

impl ToSql for PgNumericParam {
    fn to_sql(&self, ty: &PgType, out: &mut BytesMut) -> Result<IsNull, Box<dyn StdError + Sync + Send>> {
        match *ty {
            PgType::INT2 => {
                let v = self.as_i64()? as i16;
                v.to_sql(ty, out)
            }
            PgType::INT4 => {
                let v = self.as_i64()? as i32;
                v.to_sql(ty, out)
            }
            PgType::INT8 => {
                let v = self.as_i64()?;
                v.to_sql(ty, out)
            }
            PgType::FLOAT4 => {
                let v = self.as_f64()? as f32;
                v.to_sql(ty, out)
            }
            PgType::FLOAT8 => {
                let v = self.as_f64()?;
                v.to_sql(ty, out)
            }
            _ => Err("unsupported numeric parameter type".into()),
        }
    }

    fn accepts(ty: &PgType) -> bool {
        matches!(
            *ty,
            PgType::INT2 | PgType::INT4 | PgType::INT8 | PgType::FLOAT4 | PgType::FLOAT8
        )
    }

    to_sql_checked!();
}

impl PgNumericParam {
    fn as_i64(&self) -> Result<i64, Box<dyn StdError + Sync + Send>> {
        match *self {
            PgNumericParam::I64(v) => Ok(v),
            PgNumericParam::U64(v) => Ok(v.min(i64::MAX as u64) as i64),
            PgNumericParam::F64(v) => Ok(v as i64),
        }
    }

    fn as_f64(&self) -> Result<f64, Box<dyn StdError + Sync + Send>> {
        match *self {
            PgNumericParam::I64(v) => Ok(v as f64),
            PgNumericParam::U64(v) => Ok(v as f64),
            PgNumericParam::F64(v) => Ok(v),
        }
    }
}

#[derive(Debug)]
struct PgStringParam(String);

#[derive(Debug)]
struct PgNullParam;

impl ToSql for PgNullParam {
    fn to_sql(&self, _ty: &PgType, _out: &mut BytesMut) -> Result<IsNull, Box<dyn StdError + Sync + Send>> {
        Ok(IsNull::Yes)
    }

    fn accepts(_ty: &PgType) -> bool {
        true
    }

    to_sql_checked!();
}

impl ToSql for PgStringParam {
    fn to_sql(&self, ty: &PgType, out: &mut BytesMut) -> Result<IsNull, Box<dyn StdError + Sync + Send>> {
        match *ty {
            PgType::INT2 => {
                let v: i16 = self.0.parse()?;
                v.to_sql(ty, out)
            }
            PgType::INT4 => {
                let v: i32 = self.0.parse()?;
                v.to_sql(ty, out)
            }
            PgType::INT8 => {
                let v: i64 = self.0.parse()?;
                v.to_sql(ty, out)
            }
            PgType::FLOAT4 => {
                let v: f32 = self.0.parse()?;
                v.to_sql(ty, out)
            }
            PgType::FLOAT8 => {
                let v: f64 = self.0.parse()?;
                v.to_sql(ty, out)
            }
            PgType::TIMESTAMPTZ => {
                if let Ok(dt) = DateTime::parse_from_rfc3339(&self.0) {
                    return dt.with_timezone(&Utc).to_sql(ty, out);
                }
                if let Ok(dt) = DateTime::parse_from_str(&self.0, "%Y-%m-%dT%H:%M:%S%z") {
                    return dt.with_timezone(&Utc).to_sql(ty, out);
                }
                if let Ok(dt) = NaiveDateTime::parse_from_str(&self.0, "%Y-%m-%d %H:%M:%S") {
                    return DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc).to_sql(ty, out);
                }
                Err("invalid timestamptz string".into())
            }
            PgType::TIMESTAMP => {
                if let Ok(dt) = NaiveDateTime::parse_from_str(&self.0, "%Y-%m-%d %H:%M:%S") {
                    return dt.to_sql(ty, out);
                }
                if let Ok(dt) = NaiveDateTime::parse_from_str(&self.0, "%Y-%m-%dT%H:%M:%S") {
                    return dt.to_sql(ty, out);
                }
                if let Ok(dt) = DateTime::parse_from_rfc3339(&self.0) {
                    return dt.naive_utc().to_sql(ty, out);
                }
                if let Ok(dt) = DateTime::parse_from_str(&self.0, "%Y-%m-%dT%H:%M:%S%z") {
                    return dt.naive_utc().to_sql(ty, out);
                }
                Err("invalid timestamp string".into())
            }
            PgType::DATE => {
                if let Ok(date) = NaiveDate::parse_from_str(&self.0, "%Y-%m-%d") {
                    return date.to_sql(ty, out);
                }
                Err("invalid date string".into())
            }
            PgType::TIME => {
                if let Ok(time) = NaiveTime::parse_from_str(&self.0, "%H:%M:%S") {
                    return time.to_sql(ty, out);
                }
                Err("invalid time string".into())
            }
            _ => self.0.to_sql(ty, out),
        }
    }

    fn accepts(ty: &PgType) -> bool {
        matches!(
            *ty,
            PgType::TEXT
                | PgType::VARCHAR
                | PgType::BPCHAR
                | PgType::INT2
                | PgType::INT4
                | PgType::INT8
                | PgType::FLOAT4
                | PgType::FLOAT8
                | PgType::TIMESTAMPTZ
                | PgType::TIMESTAMP
                | PgType::DATE
                | PgType::TIME
        )
    }

    to_sql_checked!();
}

fn pg_cell_to_json(row: &postgres::Row, idx: usize) -> serde_json::Value {
    let col = &row.columns()[idx];
    match col.type_().name() {
        "bool" => row
            .try_get::<usize, Option<bool>>(idx)
            .ok()
            .flatten()
            .map(serde_json::Value::Bool)
            .unwrap_or(serde_json::Value::Null),
        "int2" => row
            .try_get::<usize, Option<i16>>(idx)
            .ok()
            .flatten()
            .map(|v| serde_json::Value::Number(serde_json::Number::from(v as i64)))
            .unwrap_or(serde_json::Value::Null),
        "int4" => row
            .try_get::<usize, Option<i32>>(idx)
            .ok()
            .flatten()
            .map(|v| serde_json::Value::Number(serde_json::Number::from(v as i64)))
            .unwrap_or(serde_json::Value::Null),
        "int8" => row
            .try_get::<usize, Option<i64>>(idx)
            .ok()
            .flatten()
            .map(|v| serde_json::Value::Number(serde_json::Number::from(v)))
            .unwrap_or(serde_json::Value::Null),
        "float4" => row
            .try_get::<usize, Option<f32>>(idx)
            .ok()
            .flatten()
            .and_then(|v| serde_json::Number::from_f64(v as f64))
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        "float8" => row
            .try_get::<usize, Option<f64>>(idx)
            .ok()
            .flatten()
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        "json" | "jsonb" => row
            .try_get::<usize, Option<String>>(idx)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .unwrap_or(serde_json::Value::Null),
        _ => row
            .try_get::<usize, Option<String>>(idx)
            .ok()
            .flatten()
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    }
}

fn with_sqlite_conn<T>(
    cfg: SqliteConnConfig,
    f: impl FnOnce(&SqliteConnection) -> Result<T, deno_core::error::CoreError> + Send + 'static,
) -> Result<T, deno_core::error::CoreError>
where
    T: Send + 'static,
{
    std::thread::spawn(move || {
        let path = sanitize_conn_value(&cfg.path);
        let conn = SqliteConnection::open(&path).map_err(|e| {
            deno_core::error::CoreError::from(std::io::Error::other(format!(
                "sqlite open failed: {} (path={})",
                e, path
            )))
        })?;
        f(&conn)
    })
    .join()
    .map_err(|_| {
        deno_core::error::CoreError::from(std::io::Error::other("db worker thread panicked"))
    })?
}

fn json_to_sqlite_value(value: &serde_json::Value) -> rusqlite::types::Value {
    match value {
        serde_json::Value::Null => rusqlite::types::Value::Null,
        serde_json::Value::Bool(v) => rusqlite::types::Value::Integer(if *v { 1 } else { 0 }),
        serde_json::Value::Number(v) => {
            if let Some(i) = v.as_i64() {
                rusqlite::types::Value::Integer(i)
            } else if let Some(f) = v.as_f64() {
                rusqlite::types::Value::Real(f)
            } else {
                rusqlite::types::Value::Null
            }
        }
        serde_json::Value::String(v) => rusqlite::types::Value::Text(v.clone()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            rusqlite::types::Value::Text(value.to_string())
        }
    }
}

fn sqlite_cell_to_json(row: &rusqlite::Row<'_>, idx: usize) -> serde_json::Value {
    match row.get_ref(idx) {
        Ok(SqliteValueRef::Null) => serde_json::Value::Null,
        Ok(SqliteValueRef::Integer(v)) => serde_json::Value::Number(serde_json::Number::from(v)),
        Ok(SqliteValueRef::Real(v)) => serde_json::Number::from_f64(v)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Ok(SqliteValueRef::Text(v)) => {
            serde_json::Value::String(String::from_utf8_lossy(v).to_string())
        }
        Ok(SqliteValueRef::Blob(v)) => serde_json::Value::String(format!("{:?}", v)),
        Err(_) => serde_json::Value::Null,
    }
}

fn with_mysql_conn<T>(
    cfg: MysqlConnConfig,
    f: impl FnOnce(&mut mysql::PooledConn) -> Result<T, deno_core::error::CoreError> + Send + 'static,
) -> Result<T, deno_core::error::CoreError>
where
    T: Send + 'static,
{
    std::thread::spawn(move || {
        let host = sanitize_conn_value(&cfg.host);
        let user = sanitize_conn_value(&cfg.user);
        let database = sanitize_conn_value(&cfg.database);
        let password = sanitize_conn_value(&cfg.password);

        let opts = OptsBuilder::new()
            .ip_or_hostname(Some(host.clone()))
            .tcp_port(cfg.port)
            .user(Some(user.clone()))
            .pass(Some(password.clone()))
            .db_name(Some(database.clone()))
            .tcp_connect_timeout(Some(Duration::from_secs(3)))
            .read_timeout(Some(Duration::from_secs(5)))
            .write_timeout(Some(Duration::from_secs(5)));

        let pool = MyPool::new(opts).map_err(|e| {
            deno_core::error::CoreError::from(std::io::Error::other(format!(
                "mysql pool failed: {} (host={}, port={}, database={}, user={})",
                e, host, cfg.port, database, user
            )))
        })?;
        let mut conn = pool.get_conn().map_err(|e| {
            deno_core::error::CoreError::from(std::io::Error::other(format!(
                "mysql connect failed: {}",
                e
            )))
        })?;
        f(&mut conn)
    })
    .join()
    .map_err(|_| {
        deno_core::error::CoreError::from(std::io::Error::other("db worker thread panicked"))
    })?
}

fn json_to_mysql_value(value: &serde_json::Value) -> MyValue {
    match value {
        serde_json::Value::Null => MyValue::NULL,
        serde_json::Value::Bool(v) => MyValue::Int(if *v { 1 } else { 0 }),
        serde_json::Value::Number(v) => {
            if let Some(i) = v.as_i64() {
                MyValue::Int(i)
            } else if let Some(u) = v.as_u64() {
                MyValue::UInt(u)
            } else if let Some(f) = v.as_f64() {
                MyValue::Double(f)
            } else {
                MyValue::NULL
            }
        }
        serde_json::Value::String(v) => MyValue::Bytes(v.clone().into_bytes()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            MyValue::Bytes(value.to_string().into_bytes())
        }
    }
}

fn mysql_value_to_json(value: &MyValue) -> serde_json::Value {
    match value {
        MyValue::NULL => serde_json::Value::Null,
        MyValue::Bytes(v) => serde_json::Value::String(String::from_utf8_lossy(v).to_string()),
        MyValue::Int(v) => serde_json::Value::Number(serde_json::Number::from(*v)),
        MyValue::UInt(v) => serde_json::Value::Number(serde_json::Number::from(*v)),
        MyValue::Float(v) => serde_json::Number::from_f64(*v as f64)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        MyValue::Double(v) => serde_json::Number::from_f64(*v)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        MyValue::Date(y, m, d, h, i, s, micros) => serde_json::Value::String(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}",
            y, m, d, h, i, s, micros
        )),
        MyValue::Time(neg, days, h, i, s, micros) => serde_json::Value::String(format!(
            "{}{} {:02}:{:02}:{:02}.{:06}",
            if *neg { "-" } else { "" },
            days,
            h,
            i,
            s,
            micros
        )),
    }
}

#[derive(Clone)]
struct TypeAliasInfo<'a> {
    params: Vec<String>,
    ty: &'a AstType<'a>,
}

struct TypeResolver<'a> {
    source: &'a [u8],
    aliases: HashMap<String, TypeAliasInfo<'a>>,
    structs: HashMap<String, Vec<BridgeField>>,
}

impl<'a> TypeResolver<'a> {
    fn new(source: &'a [u8], program: &'a Program<'a>) -> Self {
        let aliases = collect_aliases(program, source);
        let structs = collect_structs(program, source, &aliases);
        Self {
            source,
            aliases,
            structs,
        }
    }

    fn type_name(&self, ty: &'a AstType<'a>) -> Option<String> {
        match ty {
            AstType::Simple(token) => Some(token_text(self.source, token)),
            AstType::Name(name) => Some(name_to_string(self.source, name)),
            _ => None,
        }
    }

    fn base_name(name: &str) -> &str {
        name.rsplit('\\').next().unwrap_or(name)
    }

    fn convert_type(&mut self, ty: &'a AstType<'a>) -> BridgeType {
        let mut guard = HashSet::new();
        self.convert_type_internal(ty, &mut guard, None)
    }

    fn convert_type_internal(
        &mut self,
        ty: &'a AstType<'a>,
        alias_guard: &mut HashSet<String>,
        subs: Option<&HashMap<String, BridgeType>>,
    ) -> BridgeType {
        let resolve_param = |name: &str, subs: Option<&HashMap<String, BridgeType>>| {
            subs.and_then(|map| map.get(name).cloned())
        };
        match ty {
            AstType::Simple(token) => {
                let name = token_text(self.source, token);
                if let Some(bound) = resolve_param(&name, subs) {
                    return bound;
                }
                if let Some(resolved) = self.convert_named(&name, alias_guard) {
                    return resolved;
                }
                BridgeType::Unknown
            }
            AstType::Name(name) => {
                let name_str = name_to_string(self.source, name);
                if let Some(bound) = resolve_param(&name_str, subs) {
                    return bound;
                }
                if let Some(resolved) = self.convert_named(&name_str, alias_guard) {
                    return resolved;
                }
                BridgeType::Unknown
            }
            AstType::Nullable(inner) => {
                let inner = self.convert_type_internal(inner, alias_guard, subs);
                BridgeType::Option {
                    inner: Some(Box::new(inner)),
                }
            }
            AstType::Union(types) => {
                let mut parts = Vec::new();
                let mut saw_null = false;
                for part in *types {
                    let converted = self.convert_type_internal(part, alias_guard, subs);
                    if is_null_type(&converted) {
                        saw_null = true;
                    } else {
                        parts.push(converted);
                    }
                }
                if saw_null && parts.len() == 1 {
                    BridgeType::Option {
                        inner: Some(Box::new(parts.remove(0))),
                    }
                } else {
                    if saw_null {
                        parts.push(BridgeType::Primitive {
                            name: "null".to_string(),
                        });
                    }
                    BridgeType::Union { types: parts }
                }
            }
            AstType::Intersection(types) => {
                // Intersection types are not supported in the bridge yet; fall back to mixed.
                let _ = types;
                BridgeType::Mixed
            }
            AstType::ObjectShape(fields) => {
                let mut out = Vec::new();
                for field in *fields {
                    let name = token_text(self.source, field.name);
                    let ty = self.convert_type_internal(field.ty, alias_guard, subs);
                    out.push(BridgeField {
                        name,
                        ty,
                        optional: field.optional,
                    });
                }
                BridgeType::ObjectShape { fields: out }
            }
            AstType::Applied { base, args } => {
                let base_name = self
                    .type_name(base)
                    .unwrap_or_else(|| "unknown".to_string());
                if let Some(alias) = self.aliases.get(&base_name).cloned() {
                    if alias.params.len() == args.len() {
                        let mut param_map = HashMap::new();
                        for (idx, param) in alias.params.iter().enumerate() {
                            let arg_ty = self.convert_type_internal(&args[idx], alias_guard, subs);
                            param_map.insert(param.clone(), arg_ty);
                        }
                        if alias_guard.insert(base_name.clone()) {
                            let resolved =
                                self.convert_type_internal(alias.ty, alias_guard, Some(&param_map));
                            alias_guard.remove(&base_name);
                            return resolved;
                        }
                        return BridgeType::Mixed;
                    }
                }
                let base_id = Self::base_name(&base_name).to_ascii_lowercase();
                let mut converted_args = Vec::new();
                for arg in *args {
                    converted_args.push(self.convert_type_internal(arg, alias_guard, subs));
                }
                if base_id == "option" {
                    return BridgeType::Option {
                        inner: converted_args.get(0).cloned().map(Box::new),
                    };
                }
                if base_id == "result" {
                    let ok = converted_args.get(0).cloned().map(Box::new);
                    let err = converted_args.get(1).cloned().map(Box::new);
                    return BridgeType::Result { ok, err };
                }
                if base_id == "array" {
                    let element = converted_args.get(0).cloned().map(Box::new);
                    return BridgeType::Array { element };
                }
                BridgeType::Applied {
                    base: base_name,
                    args: converted_args,
                }
            }
        }
    }

    fn convert_named(
        &mut self,
        name: &str,
        alias_guard: &mut HashSet<String>,
    ) -> Option<BridgeType> {
        let base = Self::base_name(name).to_ascii_lowercase();
        match base.as_str() {
            "mixed" => return Some(BridgeType::Mixed),
            "int" | "float" | "bool" | "string" | "null" => {
                return Some(BridgeType::Primitive {
                    name: base.to_string(),
                });
            }
            "array" => return Some(BridgeType::Array { element: None }),
            "object" => return Some(BridgeType::Object),
            "option" => {
                return Some(BridgeType::Option { inner: None });
            }
            "result" => {
                return Some(BridgeType::Result {
                    ok: None,
                    err: None,
                });
            }
            _ => {}
        }

        if let Some(fields) = self.structs.get(name).cloned() {
            return Some(BridgeType::Struct {
                name: name.to_string(),
                fields,
            });
        }

        if let Some(alias_type) = self.aliases.get(name) {
            if !alias_type.params.is_empty() {
                return Some(BridgeType::Mixed);
            }
            if alias_guard.insert(name.to_string()) {
                let resolved = self.convert_type_internal(alias_type.ty, alias_guard, None);
                alias_guard.remove(name);
                return Some(resolved);
            }
            return Some(BridgeType::Mixed);
        }

        Some(BridgeType::Unknown)
    }
}

fn is_null_type(ty: &BridgeType) -> bool {
    matches!(ty, BridgeType::Primitive { name } if name == "null")
}

fn token_text(source: &[u8], token: &Token) -> String {
    String::from_utf8_lossy(token.text(source)).to_string()
}

fn name_to_string(source: &[u8], name: &php_rs::parser::ast::Name<'_>) -> String {
    let mut out = String::new();
    for (idx, part) in name.parts.iter().enumerate() {
        if idx > 0 {
            out.push('\\');
        }
        out.push_str(&token_text(source, part));
    }
    out
}

fn collect_aliases<'a>(
    program: &'a Program<'a>,
    source: &'a [u8],
) -> HashMap<String, TypeAliasInfo<'a>> {
    let mut out = HashMap::new();
    for stmt in program.statements.iter() {
        if let Stmt::TypeAlias {
            name,
            type_params,
            ty,
            ..
        } = stmt
        {
            let name_str = token_text(source, name);
            let params = type_params
                .iter()
                .map(|param| token_text(source, param.name))
                .collect::<Vec<_>>();
            out.insert(name_str, TypeAliasInfo { params, ty: *ty });
        }
    }
    out
}

fn collect_structs<'a>(
    program: &'a Program<'a>,
    source: &'a [u8],
    aliases: &HashMap<String, TypeAliasInfo<'a>>,
) -> HashMap<String, Vec<BridgeField>> {
    let mut out = HashMap::new();
    for stmt in program.statements.iter() {
        let Stmt::Class {
            kind,
            name,
            members,
            ..
        } = stmt
        else {
            continue;
        };
        if *kind != ClassKind::Struct {
            continue;
        }
        let struct_name = token_text(source, name);
        let mut fields = Vec::new();
        for member in members.iter() {
            match *member {
                ClassMember::Property { ty, entries, .. } => {
                    for entry in entries.iter() {
                        let field_name = token_text(source, entry.name);
                        let optional = entry.default.is_some();
                        let field_ty = ty
                            .map(|ty| {
                                let mut resolver = TypeResolver {
                                    source,
                                    aliases: aliases.clone(),
                                    structs: HashMap::new(),
                                };
                                resolver.convert_type(ty)
                            })
                            .unwrap_or(BridgeType::Mixed);
                        fields.push(BridgeField {
                            name: field_name,
                            ty: field_ty,
                            optional,
                        });
                    }
                }
                _ => {}
            }
        }
        out.insert(struct_name, fields);
    }
    out
}

#[op2]
#[serde]
fn op_php_parse_phpx_types(
    #[string] source: String,
    #[string] file_path: String,
) -> Result<BridgeModuleTypes, deno_core::error::CoreError> {
    let trimmed = source.trim_start();
    let source_holder: std::borrow::Cow<[u8]> = if trimmed.starts_with("<?php") {
        let prefix_len = source.len() - trimmed.len();
        let without_tag = source[prefix_len + 5..].to_string();
        std::borrow::Cow::Owned(without_tag.into_bytes())
    } else {
        std::borrow::Cow::Borrowed(source.as_bytes())
    };
    let source_bytes = source_holder.as_ref();
    let arena = Bump::new();
    let lexer = Lexer::new(source_bytes);
    let path = std::path::Path::new(&file_path);
    let mut mode = detect_parser_mode(source_bytes, Some(path));
    if path
        .to_string_lossy()
        .replace('\\', "/")
        .contains("/php_modules/")
    {
        mode = ParserMode::PhpxInternal;
    }
    let mut parser = Parser::new_with_mode(lexer, &arena, mode);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        return Err(deno_core::error::CoreError::from(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "Failed to parse PHPX types for '{}': {:?}",
                file_path, program.errors
            ),
        )));
    }
    let mut resolver = TypeResolver::new(source_bytes, &program);
    let mut functions = HashMap::new();
    for stmt in program.statements.iter() {
        if let Stmt::Function {
            name,
            params,
            return_type,
            ..
        } = stmt
        {
            let fn_name = token_text(source_bytes, name);
            let mut params_out = Vec::new();
            let mut has_variadic = false;
            for param in params.iter() {
                let ty = param.ty.map(|ty| resolver.convert_type(ty));
                let required = param.default.is_none() && !param.variadic;
                let variadic = param.variadic;
                if variadic {
                    has_variadic = true;
                }
                params_out.push(BridgeParam {
                    ty,
                    required,
                    variadic,
                });
            }
            let return_type = return_type.map(|ty| resolver.convert_type(ty));
            functions.insert(
                fn_name,
                BridgeFunction {
                    params: params_out,
                    return_type,
                    variadic: has_variadic,
                },
            );
        }
    }
    Ok(BridgeModuleTypes {
        functions,
        structs: resolver
            .structs
            .iter()
            .map(|(k, v)| (k.clone(), BridgeStruct { fields: v.clone() }))
            .collect(),
    })
}

#[op2]
#[buffer]
fn op_php_read_file_sync(#[string] path: String) -> Result<Vec<u8>, deno_core::error::CoreError> {
    enforce_read(Some(&path))?;
    std::fs::read(&path).map_err(|e| {
        deno_core::error::CoreError::from(std::io::Error::new(
            e.kind(),
            format!("Failed to read file '{}': {}", path, e),
        ))
    })
}

#[op2(fast)]
fn op_php_write_file_sync(
    #[string] path: String,
    #[buffer] data: &[u8],
) -> Result<(), deno_core::error::CoreError> {
    enforce_write(Some(&path))?;
    std::fs::write(&path, data).map_err(|e| {
        deno_core::error::CoreError::from(std::io::Error::new(
            e.kind(),
            format!("Failed to write file '{}': {}", path, e),
        ))
    })
}

#[op2(fast)]
fn op_php_mkdirs(#[string] path: String) -> Result<(), deno_core::error::CoreError> {
    enforce_write(Some(&path))?;
    std::fs::create_dir_all(&path).map_err(|e| {
        deno_core::error::CoreError::from(std::io::Error::new(
            e.kind(),
            format!("Failed to create dir '{}': {}", path, e),
        ))
    })
}

#[op2(fast)]
fn op_php_set_privileged(#[number] enabled: i64, #[string] label: String) {
    let label = if label.trim().is_empty() { None } else { Some(label) };
    set_security_privileged(enabled != 0, label);
}

#[op2]
#[string]
fn op_php_sha256(#[string] data: String) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let digest = hasher.finalize();
    format!("{:x}", digest)
}

#[op2]
#[buffer]
fn op_php_random_bytes(#[number] len: i64) -> Vec<u8> {
    if len <= 0 {
        return Vec::new();
    }
    if len > (1024 * 1024) {
        return Vec::new();
    }
    let mut out = vec![0u8; len as usize];
    if getrandom::getrandom(&mut out).is_err() {
        return Vec::new();
    }
    out
}

#[op2]
#[serde]
fn op_php_read_env() -> HashMap<String, String> {
    if enforce_env(None).is_err() {
        return HashMap::new();
    }
    let mut merged = HashMap::new();
    for (key, value) in std::env::vars() {
        merged.insert(key, value);
    }
    for (key, value) in read_dotenv_from_cwd() {
        merged.insert(key, value);
    }
    merged
}

fn read_dotenv_from_cwd() -> HashMap<String, String> {
    let out = HashMap::new();
    let Ok(cwd) = std::env::current_dir() else {
        return out;
    };
    let path = cwd.join(".env");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return out;
    };
    parse_dotenv(&raw)
}

fn parse_dotenv(raw: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let body = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let Some((key_raw, value_raw)) = body.split_once('=') else {
            continue;
        };
        let key = key_raw.trim();
        if key.is_empty() {
            continue;
        }
        let mut value = value_raw.trim().to_string();
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            value = decode_double_quoted(&value[1..value.len() - 1]);
        } else if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
            value = value[1..value.len() - 1].to_string();
        } else if let Some(idx) = value.find(" #") {
            value = value[..idx].trim_end().to_string();
        }
        out.insert(key.to_string(), value);
    }
    out
}

fn decode_double_quoted(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        match next {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

fn db_call_impl(
    action: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let err = |msg: String| {
        deno_core::error::CoreError::from(std::io::Error::new(std::io::ErrorKind::Other, msg))
    };

    let args_obj = args.as_object().cloned().unwrap_or_default();
    match action.as_str() {
        "open" => {
            let started = Instant::now();
            let raw_driver = args_obj
                .get("driver")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let driver = raw_driver
                .split('\0')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            let cfg = args_obj
                .get("config")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();

            let (key, driver_cfg) = match driver.as_str() {
                d if d.starts_with("postgres") => {
                    let host = cfg
                        .get("host")
                        .and_then(|v| v.as_str())
                        .unwrap_or("127.0.0.1")
                        .trim_matches('\0')
                        .to_string();
                    let port = cfg.get("port").and_then(|v| v.as_u64()).unwrap_or(5432);
                    let user = cfg
                        .get("user")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim_matches('\0')
                        .to_string();
                    let password = cfg
                        .get("password")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim_matches('\0')
                        .to_string();
                    let database = cfg
                        .get("database")
                        .and_then(|v| v.as_str())
                        .or_else(|| cfg.get("dbname").and_then(|v| v.as_str()))
                        .unwrap_or("")
                        .trim_matches('\0')
                        .to_string();
                    (
                        format!(
                            "postgres://{}:{}@{}:{}/{}",
                            user, password, host, port, database
                        ),
                        DbDriverConfig::Postgres(PgConnConfig {
                            host,
                            port: port as u16,
                            database,
                            user,
                            password,
                        }),
                    )
                }
                d if d.starts_with("sqlite") => {
                    let path = cfg
                        .get("path")
                        .and_then(|v| v.as_str())
                        .or_else(|| cfg.get("file").and_then(|v| v.as_str()))
                        .unwrap_or("")
                        .trim_matches('\0')
                        .to_string();
                    if path.is_empty() {
                        return Ok(serde_json::json!({
                            "ok": false,
                            "error": "sqlite open requires config.path"
                        }));
                    }
                    (
                        format!("sqlite://{}", path),
                        DbDriverConfig::Sqlite(SqliteConnConfig { path }),
                    )
                }
                d if d.starts_with("mysql") => {
                    let host = cfg
                        .get("host")
                        .and_then(|v| v.as_str())
                        .unwrap_or("127.0.0.1")
                        .trim_matches('\0')
                        .to_string();
                    let port = cfg.get("port").and_then(|v| v.as_u64()).unwrap_or(3306);
                    let user = cfg
                        .get("user")
                        .and_then(|v| v.as_str())
                        .unwrap_or("root")
                        .trim_matches('\0')
                        .to_string();
                    let password = cfg
                        .get("password")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim_matches('\0')
                        .to_string();
                    let database = cfg
                        .get("database")
                        .and_then(|v| v.as_str())
                        .or_else(|| cfg.get("dbname").and_then(|v| v.as_str()))
                        .unwrap_or("")
                        .trim_matches('\0')
                        .to_string();
                    (
                        format!(
                            "mysql://{}:{}@{}:{}/{}",
                            user, password, host, port, database
                        ),
                        DbDriverConfig::Mysql(MysqlConnConfig {
                            host,
                            port: port as u16,
                            database,
                            user,
                            password,
                        }),
                    )
                }
                _ => {
                    return Ok(serde_json::json!({
                        "ok": false,
                        "error": format!("unsupported driver '{}'", driver)
                    }));
                }
            };

            {
                let state = db_state()
                    .lock()
                    .map_err(|_| err("db lock poisoned".to_string()))?;
                if let Some(handle) = state.key_to_handle.get(&key).copied() {
                    drop(state);
                    if let Ok(mut state) = db_state().lock() {
                        state.record_metric(
                            "open",
                            &driver,
                            started.elapsed().as_millis() as u64,
                            false,
                        );
                    }
                    return Ok(serde_json::json!({
                        "ok": true,
                        "handle": handle,
                        "reused": true
                    }));
                }
            }

            let mut state = db_state()
                .lock()
                .map_err(|_| err("db lock poisoned".to_string()))?;
            if let Some(handle) = state.key_to_handle.get(&key).copied() {
                state.record_metric("open", &driver, started.elapsed().as_millis() as u64, false);
                return Ok(serde_json::json!({
                    "ok": true,
                    "handle": handle,
                    "reused": true
                }));
            }
            let handle = state.next_handle;
            state.next_handle += 1;
            state.handles.insert(
                handle,
                DbConn {
                    key: key.clone(),
                    config: driver_cfg,
                },
            );
            state.key_to_handle.insert(key, handle);
            state.record_metric("open", &driver, started.elapsed().as_millis() as u64, false);
            Ok(serde_json::json!({
                "ok": true,
                "handle": handle,
                "reused": false
            }))
        }
        "query" => {
            let started = Instant::now();
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("query: missing handle".to_string()))?;
            let sql = args_obj
                .get("sql")
                .and_then(|v| v.as_str())
                .ok_or_else(|| err("query: missing sql".to_string()))?;
            let params = args_obj
                .get("params")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let driver_cfg = {
                let mut state = db_state()
                    .lock()
                    .map_err(|_| err("db lock poisoned".to_string()))?;
                let driver_name = {
                    let conn = state
                        .handles
                        .get(&handle)
                        .ok_or_else(|| err(format!("query: unknown handle {}", handle)))?;
                    conn.config.clone()
                };
                state.touch_statement_cache(handle, sql);
                driver_name
            };
            let driver_name = driver_cfg.driver_name();
            let sql = sql.to_string();
            let out_rows_result = match driver_cfg {
                DbDriverConfig::Postgres(cfg) => with_pg_client(cfg, move |client| {
                    let boxed: Vec<Box<dyn ToSql + Sync>> =
                        params.iter().map(json_to_pg_param).collect();
                    let refs: Vec<&(dyn ToSql + Sync)> = boxed.iter().map(|v| v.as_ref()).collect();
                    let rows = client.query(&sql, &refs).map_err(|e| {
                        deno_core::error::CoreError::from(std::io::Error::other(format!(
                            "postgres query failed: {}",
                            e
                        )))
                    })?;

                    let mut out_rows = Vec::with_capacity(rows.len());
                    for row in &rows {
                        let mut obj = serde_json::Map::new();
                        for idx in 0..row.len() {
                            let name = row.columns()[idx].name().to_string();
                            obj.insert(name, pg_cell_to_json(row, idx));
                        }
                        out_rows.push(serde_json::Value::Object(obj));
                    }
                    Ok(out_rows)
                })?,
                DbDriverConfig::Sqlite(cfg) => with_sqlite_conn(cfg, move |conn| {
                    let mut stmt = conn.prepare(&sql).map_err(|e| {
                        deno_core::error::CoreError::from(std::io::Error::other(format!(
                            "sqlite prepare failed: {}",
                            e
                        )))
                    })?;
                    let sqlite_params: Vec<rusqlite::types::Value> =
                        params.iter().map(json_to_sqlite_value).collect();
                    let mut rows = stmt
                        .query(sqlite_params_from_iter(sqlite_params.iter()))
                        .map_err(|e| {
                            deno_core::error::CoreError::from(std::io::Error::other(format!(
                                "sqlite query failed: {}",
                                e
                            )))
                        })?;

                    let mut out_rows = Vec::new();
                    while let Some(row) = rows.next().map_err(|e| {
                        deno_core::error::CoreError::from(std::io::Error::other(format!(
                            "sqlite row fetch failed: {}",
                            e
                        )))
                    })? {
                        let mut obj = serde_json::Map::new();
                        let row_ref = row.as_ref();
                        for idx in 0..row_ref.column_count() {
                            let name = row_ref.column_name(idx).unwrap_or("").to_string();
                            obj.insert(name, sqlite_cell_to_json(row, idx));
                        }
                        out_rows.push(serde_json::Value::Object(obj));
                    }
                    Ok(out_rows)
                })?,
                DbDriverConfig::Mysql(cfg) => with_mysql_conn(cfg, move |conn| {
                    let mysql_params =
                        MyParams::Positional(params.iter().map(json_to_mysql_value).collect());
                    let rows: Vec<mysql::Row> = conn.exec(&sql, mysql_params).map_err(|e| {
                        deno_core::error::CoreError::from(std::io::Error::other(format!(
                            "mysql query failed: {}",
                            e
                        )))
                    })?;

                    let mut out_rows = Vec::with_capacity(rows.len());
                    for row in &rows {
                        let mut obj = serde_json::Map::new();
                        let cols = row.columns_ref();
                        for (idx, col) in cols.iter().enumerate() {
                            let name = col.name_str().to_string();
                            let value = row
                                .as_ref(idx)
                                .map(mysql_value_to_json)
                                .unwrap_or(serde_json::Value::Null);
                            obj.insert(name, value);
                        }
                        out_rows.push(serde_json::Value::Object(obj));
                    }
                    Ok(out_rows)
                })?,
            };
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let mut metric_state = db_state()
                .lock()
                .map_err(|_| err("db lock poisoned".to_string()))?;
            metric_state.record_metric("query", driver_name, elapsed_ms, false);
            drop(metric_state);
            let out_rows = out_rows_result;

            Ok(serde_json::json!({
                "ok": true,
                "rows": out_rows
            }))
        }
        "exec" => {
            let started = Instant::now();
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("exec: missing handle".to_string()))?;
            let sql = args_obj
                .get("sql")
                .and_then(|v| v.as_str())
                .ok_or_else(|| err("exec: missing sql".to_string()))?;
            let params = args_obj
                .get("params")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let driver_cfg = {
                let mut state = db_state()
                    .lock()
                    .map_err(|_| err("db lock poisoned".to_string()))?;
                let driver_name = {
                    let conn = state
                        .handles
                        .get(&handle)
                        .ok_or_else(|| err(format!("exec: unknown handle {}", handle)))?;
                    conn.config.clone()
                };
                state.touch_statement_cache(handle, sql);
                driver_name
            };
            let driver_name = driver_cfg.driver_name();
            let sql = sql.to_string();
            let affected_result = match driver_cfg {
                DbDriverConfig::Postgres(cfg) => with_pg_client(cfg, move |client| {
                    let boxed: Vec<Box<dyn ToSql + Sync>> =
                        params.iter().map(json_to_pg_param).collect();
                    let refs: Vec<&(dyn ToSql + Sync)> = boxed.iter().map(|v| v.as_ref()).collect();
                    client.execute(&sql, &refs).map_err(|e| {
                        deno_core::error::CoreError::from(std::io::Error::other(format!(
                            "postgres exec failed: {}",
                            e
                        )))
                    })
                })?,
                DbDriverConfig::Sqlite(cfg) => with_sqlite_conn(cfg, move |conn| {
                    let mut stmt = conn.prepare(&sql).map_err(|e| {
                        deno_core::error::CoreError::from(std::io::Error::other(format!(
                            "sqlite prepare failed: {}",
                            e
                        )))
                    })?;
                    let sqlite_params: Vec<rusqlite::types::Value> =
                        params.iter().map(json_to_sqlite_value).collect();
                    let changed = stmt
                        .execute(sqlite_params_from_iter(sqlite_params.iter()))
                        .map_err(|e| {
                            deno_core::error::CoreError::from(std::io::Error::other(format!(
                                "sqlite exec failed: {}",
                                e
                            )))
                        })?;
                    Ok(changed as u64)
                })?,
                DbDriverConfig::Mysql(cfg) => with_mysql_conn(cfg, move |conn| {
                    let mysql_params =
                        MyParams::Positional(params.iter().map(json_to_mysql_value).collect());
                    let result = conn.exec_iter(&sql, mysql_params).map_err(|e| {
                        deno_core::error::CoreError::from(std::io::Error::other(format!(
                            "mysql exec failed: {}",
                            e
                        )))
                    })?;
                    Ok(result.affected_rows())
                })?,
            };
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let mut metric_state = db_state()
                .lock()
                .map_err(|_| err("db lock poisoned".to_string()))?;
            metric_state.record_metric("exec", driver_name, elapsed_ms, false);
            drop(metric_state);
            let affected = affected_result;

            Ok(serde_json::json!({
                "ok": true,
                "affected_rows": affected
            }))
        }
        "begin" => {
            // Transaction scope across multiple calls is not supported in stateless mode.
            Ok(serde_json::json!({ "ok": true }))
        }
        "commit" => Ok(serde_json::json!({ "ok": true })),
        "rollback" => Ok(serde_json::json!({ "ok": true })),
        "close" => {
            let started = Instant::now();
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("close: missing handle".to_string()))?;
            let mut state = db_state()
                .lock()
                .map_err(|_| err("db lock poisoned".to_string()))?;
            if let Some(conn) = state.handles.remove(&handle) {
                state.record_metric(
                    "close",
                    conn.config.driver_name(),
                    started.elapsed().as_millis() as u64,
                    false,
                );
                state.key_to_handle.remove(&conn.key);
                state.statement_cache.remove(&handle);
            }
            Ok(serde_json::json!({ "ok": true }))
        }
        "stats" => {
            let state = db_state()
                .lock()
                .map_err(|_| err("db lock poisoned".to_string()))?;
            let mut handles_by_driver: HashMap<String, u64> = HashMap::new();
            for conn in state.handles.values() {
                let key = conn.config.driver_name().to_string();
                let prev = handles_by_driver.get(&key).copied().unwrap_or(0);
                handles_by_driver.insert(key, prev + 1);
            }

            let mut metrics = serde_json::Map::new();
            for (key, metric) in &state.metrics {
                let avg_ms = if metric.calls == 0 {
                    0
                } else {
                    metric.total_ms / metric.calls
                };
                metrics.insert(
                    key.clone(),
                    serde_json::json!({
                        "calls": metric.calls,
                        "errors": metric.errors,
                        "total_ms": metric.total_ms,
                        "avg_ms": avg_ms
                    }),
                );
            }

            Ok(serde_json::json!({
                "ok": true,
                "active_handles": state.handles.len() as u64,
                "handles_by_driver": handles_by_driver,
                "statement_cache_entries": state.statement_cache_entries(),
                "statement_cache_hits": state.statement_cache_hits,
                "statement_cache_misses": state.statement_cache_misses,
                "metrics": metrics
            }))
        }
        _ => Ok(serde_json::json!({
            "ok": false,
            "error": format!("unknown db action '{}'", action)
        })),
    }
}

#[derive(Clone, Copy)]
enum DbProtoActionKind {
    Open,
    Query,
    QueryOne,
    Exec,
    Begin,
    Commit,
    Rollback,
    Close,
    Stats,
}

fn core_err(msg: impl Into<String>) -> deno_core::error::CoreError {
    deno_core::error::CoreError::from(std::io::Error::other(msg.into()))
}

fn security_policy_from_env() -> SecurityPolicy {
    let raw = match std::env::var("DEKA_SECURITY_POLICY") {
        Ok(v) => v,
        Err(_) => return SecurityPolicy::default(),
    };
    let json = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(v) => v,
        Err(_) => return SecurityPolicy::default(),
    };
    let parsed = parse_deka_security_policy(&json);
    if parsed.has_errors() {
        SecurityPolicy::default()
    } else {
        parsed.policy
    }
}

fn rule_allows(capability: &str, rule: &RuleList, target: Option<&str>) -> bool {
    match rule {
        RuleList::None => false,
        RuleList::All => true,
        RuleList::List(items) => match target {
            Some(target) => items.iter().any(|item| match_rule_item(capability, item, target)),
            None => false,
        },
    }
}

fn rule_denies(capability: &str, rule: &RuleList, target: Option<&str>) -> bool {
    match rule {
        RuleList::None => false,
        RuleList::All => true,
        RuleList::List(items) => match target {
            Some(target) => items.iter().any(|item| match_rule_item(capability, item, target)),
            None => false,
        },
    }
}

fn match_rule_item(capability: &str, rule_item: &str, target: &str) -> bool {
    if rule_item == "*" {
        return true;
    }
    if matches!(capability, "read" | "write" | "wasm") {
        return path_matches(rule_item, target);
    }
    rule_item == target
}

fn path_matches(rule_item: &str, target: &str) -> bool {
    let target_path = normalize_path(target);
    let rule_path = normalize_path(rule_item);
    target_path.starts_with(&rule_path)
}

fn normalize_path(value: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(value);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    };
    std::fs::canonicalize(&resolved).unwrap_or(resolved)
}

thread_local! {
    static SECURITY_PRIVILEGED: Cell<bool> = Cell::new(false);
    static SECURITY_PRIVILEGED_LABEL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn set_security_privileged(enabled: bool, label: Option<String>) {
    SECURITY_PRIVILEGED.with(|flag| flag.set(enabled));
    SECURITY_PRIVILEGED_LABEL.with(|slot| {
        let mut guard = slot.borrow_mut();
        *guard = label;
    });
    let context = SECURITY_PRIVILEGED_LABEL.with(|slot| slot.borrow().clone());
    let context = context.as_deref().unwrap_or("unknown");
    if enabled {
        stdio::debug("security", &format!("privileged context enabled ({})", context));
    } else {
        stdio::debug("security", &format!("privileged context disabled ({})", context));
    }
}

fn security_privileged_enabled() -> bool {
    SECURITY_PRIVILEGED.with(|flag| flag.get())
}

fn is_internal_security_target(target: &str) -> bool {
    let mut normalized = target.replace('\\', "/");
    if normalized.ends_with('/') {
        normalized = normalized.trim_end_matches('/').to_string();
    }
    if normalized == "deka.lock" || normalized.ends_with("/deka.lock") {
        return true;
    }
    if normalized == "php_modules/.cache"
        || normalized.starts_with("php_modules/.cache/")
        || normalized.contains("/php_modules/.cache/")
        || normalized.ends_with("/php_modules/.cache")
    {
        return true;
    }
    if normalized == ".cache"
        || normalized.starts_with(".cache/")
        || normalized.contains("/.cache/")
        || normalized.ends_with("/.cache")
    {
        return true;
    }
    false
}

#[cfg(test)]
mod security_rule_tests {
    use super::{
        RuleList, classify_security_origin, is_internal_security_target, is_runtime_safe_env_key,
        match_rule_item, normalize_rel_like, prompt_scope_key, rule_allows, rule_denies,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        dir.push(format!("deka-security-test-{}", stamp));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_allows_prefix_path() {
        let root = temp_dir();
        let subdir = root.join("src");
        fs::create_dir_all(&subdir).unwrap();
        let file = subdir.join("main.phpx");
        fs::write(&file, "ok").unwrap();
        let rule = RuleList::List(vec![root.to_string_lossy().to_string()]);
        let target = file.to_string_lossy().to_string();
        assert!(rule_allows("read", &rule, Some(&target)));
    }

    #[test]
    fn write_denies_prefix_path() {
        let root = temp_dir();
        let subdir = root.join("php_modules/.cache");
        fs::create_dir_all(&subdir).unwrap();
        let file = subdir.join("out.php");
        fs::write(&file, "x").unwrap();
        let rule = RuleList::List(vec![subdir.to_string_lossy().to_string()]);
        let target = file.to_string_lossy().to_string();
        assert!(rule_denies("write", &rule, Some(&target)));
    }

    #[test]
    fn non_path_capability_requires_exact_match() {
        assert!(match_rule_item("env", "DATABASE_URL", "DATABASE_URL"));
        assert!(!match_rule_item("env", "DATABASE_URL", "PATH"));
    }

    #[test]
    fn internal_security_targets_match_expected_paths() {
        assert!(is_internal_security_target("deka.lock"));
        assert!(is_internal_security_target("/tmp/project/deka.lock"));
        assert!(is_internal_security_target("php_modules/.cache/phpx/foo.php"));
        assert!(is_internal_security_target("/tmp/project/php_modules/.cache"));
        assert!(is_internal_security_target(".cache/phpx/foo.php"));
        assert!(is_internal_security_target("/tmp/project/.cache"));
        assert!(!is_internal_security_target("/tmp/project/app/index.phpx"));
    }

    #[test]
    fn runtime_safe_env_keys_detected() {
        assert!(is_runtime_safe_env_key("PORT"));
        assert!(is_runtime_safe_env_key("deka_security_policy"));
        assert!(!is_runtime_safe_env_key("DATABASE_URL"));
    }

    #[test]
    fn prompt_scope_key_collapses_env_runtime_safe() {
        assert_eq!(prompt_scope_key("env", Some("PORT")), "env::runtime-safe");
        assert_eq!(
            prompt_scope_key("env", Some("DATABASE_URL")),
            "env::DATABASE_URL"
        );
    }

    #[test]
    fn relative_deps_classifies_as_third_party() {
        assert_eq!(
            classify_security_origin("read", Some("./deps/evil.txt")),
            "third-party"
        );
    }

    #[test]
    fn relative_app_classifies_as_project_owned() {
        assert_eq!(
            classify_security_origin("read", Some("./app/main.phpx")),
            "project-owned"
        );
    }

    #[test]
    fn normalize_rel_like_strips_dot_prefixes() {
        assert_eq!(normalize_rel_like("./deps/../deps/file.txt"), "deps/../deps/file.txt");
        assert_eq!(normalize_rel_like("././app/main.phpx"), "app/main.phpx");
    }

    #[test]
    fn prompt_scope_key_collapses_read_paths_to_directory_rule() {
        let key = prompt_scope_key("read", Some("./deps/evil.txt"));
        assert_eq!(key, "read::./deps");
    }
}

fn enforce_scope(
    capability: &str,
    allow_rule: &RuleList,
    deny_rule: &RuleList,
    target: Option<&str>,
) -> Result<(), deno_core::error::CoreError> {
    if !security_enforcement_enabled() {
        return Ok(());
    }
    if rule_denies(capability, deny_rule, target) {
        return Err(core_err(format!(
            "SECURITY_POLICY_DENY_PRECEDENCE: capability={} target={} denied by policy",
            capability,
            target.unwrap_or("*")
        )));
    }
    if !rule_allows(capability, allow_rule, target) {
        if prompt_enabled() && prompt_grant(capability, target)? {
            return Ok(());
        }
        let origin = classify_security_origin(capability, target);
        let mut message = format!(
            "SECURITY_CAPABILITY_DENIED: capability={} target={} origin={} not allowed (re-run with explicit allow flag or configure security)",
            capability,
            target.unwrap_or("*"),
            origin
        );
        if let Some(hint) = config_hint_for_request(capability, target) {
            message.push_str(" Hint: ");
            message.push_str(&hint);
        }
        return Err(core_err(message));
    }
    Ok(())
}

fn security_enforcement_enabled() -> bool {
    true
}

fn prompt_enabled() -> bool {
    if std::env::var("DEKA_SECURITY_NO_PROMPT")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return false;
    }
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

fn prompt_grants() -> &'static Mutex<HashSet<String>> {
    static PROMPT_GRANTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    PROMPT_GRANTS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn prompt_grant(
    capability: &str,
    target: Option<&str>,
) -> Result<bool, deno_core::error::CoreError> {
    let key = prompt_scope_key(capability, target);
    let origin = classify_security_origin(capability, target);
    let target_label = target.unwrap_or("*");
    let safe_env_scope = capability == "env"
        && target
            .map(|value| is_runtime_safe_env_key(value))
            .unwrap_or(false);
    {
        let grants = prompt_grants()
            .lock()
            .map_err(|_| core_err("security prompt lock poisoned"))?;
        if grants.contains(&key) {
            return Ok(true);
        }
    }

    if let Some(hint) = config_hint_for_request(capability, target) {
        eprintln!("[security] hint: {}", hint);
    }
    if safe_env_scope {
        eprintln!(
            "[security] note: '{}' is treated as runtime-safe env scope; one decision applies to this process",
            target_label
        );
    }
    let prompt = format!(
        "[security] allow {} on {} (origin={}, scope={}) for this process? [y/N]: ",
        capability,
        target_label,
        origin,
        key
    );
    eprint!("{}", prompt);
    let _ = std::io::stderr().flush();

    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(_) => {
            let accepted = matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
            if accepted {
                let mut grants = prompt_grants()
                    .lock()
                    .map_err(|_| core_err("security prompt lock poisoned"))?;
                grants.insert(key);
                if let Err(err) = update_deka_json_allow(capability, target) {
                    eprintln!("[security] note: failed to update deka.json: {}", err);
                } else {
                    eprintln!(
                        "[security] updated deka.json to allow {} on {}",
                        capability,
                        target.unwrap_or("*")
                    );
                }
            }
            Ok(accepted)
        }
        Err(err) => Err(core_err(format!("security prompt failed: {}", err))),
    }
}

fn prompt_scope_key(capability: &str, target: Option<&str>) -> String {
    let Some(target) = target else {
        return format!("{}::*", capability);
    };
    let target = target.trim();
    if target.is_empty() {
        return format!("{}::*", capability);
    }
    let project_kind = project_kind();
    match capability {
        "read" | "write" => {
            let items = rule_items_for_request(capability, target, project_kind);
            if let Some(item) = items.first() {
                return format!("{}::{}", capability, item);
            }
        }
        "env" => {
            if is_runtime_safe_env_key(target) {
                return "env::runtime-safe".to_string();
            }
        }
        _ => {}
    }
    format!("{}::{}", capability, target)
}

fn is_runtime_safe_env_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_uppercase();
    matches!(
        normalized.as_str(),
        "PORT"
            | "PWD"
            | "TMPDIR"
            | "TEMP"
            | "TMP"
            | "HOME"
            | "PATH"
            | "PHPX_MODULE_ROOT"
    ) || normalized.starts_with("DEKA_")
}

fn classify_security_origin(capability: &str, target: Option<&str>) -> &'static str {
    let Some(raw_target) = target else {
        return "unknown";
    };
    let target = raw_target.trim();
    if target.is_empty() {
        return "unknown";
    }

    if (capability == "read" || capability == "write") && is_internal_security_target(target) {
        return "runtime-internal";
    }

    let path = std::path::Path::new(target);
    let root = project_root();
    if let Some(root) = root {
        let rel = if path.is_absolute() {
            path.strip_prefix(&root).ok().map(|p| p.to_path_buf())
        } else {
            Some(path.to_path_buf())
        };
        if let Some(rel) = rel {
            let rel_str = normalize_rel_like(&rel.to_string_lossy().replace('\\', "/"));
            if rel_str.starts_with("deps/")
                || rel_str.starts_with("vendor/")
                || rel_str.starts_with("node_modules/")
            {
                return "third-party";
            }
            if rel_str.starts_with("app/")
                || rel_str.starts_with("src/")
                || rel_str.starts_with("public/")
                || rel_str.starts_with("php_modules/")
                || rel_str == "deka.json"
                || rel_str == "deka.lock"
            {
                return "project-owned";
            }
        }
    }

    if path.is_absolute() {
        return "third-party";
    }
    "unknown"
}

fn config_hint_for_request(capability: &str, target: Option<&str>) -> Option<String> {
    let target = target?.trim();
    if target.is_empty() {
        return None;
    }
    let project_kind = project_kind();
    let suggestion = match capability {
        "read" => suggest_read_rule(target, project_kind),
        "write" => suggest_write_rule(target, project_kind),
        "net" => Some(format!("security.allow.net = [\"{}\"]", target)),
        "env" => Some(format!("security.allow.env = [\"{}\"]", target)),
        "run" => Some(format!("security.allow.run = [\"{}\"]", target)),
        "db" => Some(format!("security.allow.db = [\"{}\"]", target)),
        "wasm" => Some(format!("security.allow.wasm = [\"{}\"]", target)),
        _ => None,
    }?;

    let note = if is_common_target(target, project_kind, capability) {
        " (common for this project type)"
    } else {
        ""
    };
    let patch = patch_for_suggestion(capability, &suggestion)?;
    let path = format!("$.security.allow.{}", capability);
    let risk = risk_note_for_request(capability, &suggestion);
    Some(format!(
        "deka.json path: {} rule: {}{} risk: {} patch:\n{}",
        path, suggestion, note, risk, patch
    ))
}

fn risk_note_for_request(capability: &str, suggestion: &str) -> &'static str {
    let broad = suggestion.contains("\"./\"")
        || suggestion.contains("\"*\"")
        || suggestion.contains("\"./app\"")
        || suggestion.contains("\"./php_modules\"");
    match capability {
        "read" => {
            if broad {
                "broader read grants expose more local files; prefer the narrowest folder."
            } else {
                "read grants reveal file contents at allowed paths only."
            }
        }
        "write" => {
            if broad {
                "broader write grants permit file mutation; keep writable paths scoped."
            } else {
                "write grants can modify files in allowed paths; avoid source roots when possible."
            }
        }
        "env" => "env grants expose process secrets; allow specific keys only.",
        "net" => "net grants allow outbound/inbound network access to allowed targets.",
        "run" => "run grants allow subprocess execution; treat binaries as privileged.",
        "db" => "db grants allow database side effects on allowed drivers.",
        "wasm" => "wasm grants permit loading executable wasm modules.",
        _ => "this grant expands runtime capabilities; scope tightly.",
    }
}

fn suggest_read_rule(target: &str, project_kind: ProjectKind) -> Option<String> {
    let root = project_root()?;
    let path = std::path::Path::new(target);
    let rel = path.strip_prefix(&root).ok().unwrap_or(path);
    let rel_str = normalize_rel_like(&rel.to_string_lossy());
    if rel_str.starts_with("deps/") {
        return Some("security.allow.read = [\"./deps\"]".to_string());
    }
    if rel_str.starts_with("node_modules/") {
        return Some("security.allow.read = [\"./node_modules\"]".to_string());
    }
    if rel_str.starts_with("php_modules/.cache") {
        return Some("security.allow.read = [\"./php_modules/.cache\"]".to_string());
    }
    if rel_str.starts_with("php_modules/") {
        return Some("security.allow.read = [\"./php_modules\"]".to_string());
    }
    if rel_str.starts_with("src/") {
        return Some("security.allow.read = [\"./src\"]".to_string());
    }
    if rel_str == "deka.lock" {
        return Some("security.allow.read = [\"./deka.lock\"]".to_string());
    }
    if let Some(prefix) = rel.components().next().and_then(|c| c.as_os_str().to_str()) {
        return Some(format!("security.allow.read = [\"./{}\"]", prefix));
    }
    default_example("read", project_kind)
}

fn suggest_write_rule(target: &str, project_kind: ProjectKind) -> Option<String> {
    let root = project_root()?;
    let path = std::path::Path::new(target);
    let rel = path.strip_prefix(&root).ok().unwrap_or(path);
    let rel_str = normalize_rel_like(&rel.to_string_lossy());
    if rel_str.starts_with("php_modules/.cache") {
        return Some("security.allow.write = [\"./php_modules/.cache\"]".to_string());
    }
    if rel_str.starts_with("dist/") {
        return Some("security.allow.write = [\"./dist\"]".to_string());
    }
    if rel_str.starts_with("build/") {
        return Some("security.allow.write = [\"./build\"]".to_string());
    }
    if rel_str.starts_with(".cache/") {
        return Some("security.allow.write = [\"./.cache\"]".to_string());
    }
    if rel_str == "deka.lock" {
        return Some("security.allow.write = [\"./deka.lock\"]".to_string());
    }
    if let Some(prefix) = rel.components().next().and_then(|c| c.as_os_str().to_str()) {
        return Some(format!("security.allow.write = [\"./{}\"]", prefix));
    }
    default_example("write", project_kind)
}

fn project_root() -> Option<std::path::PathBuf> {
    if let Ok(root) = std::env::var("PHPX_MODULE_ROOT") {
        if !root.trim().is_empty() {
            return Some(std::path::PathBuf::from(root));
        }
    }
    if let Ok(handler) = std::env::var("HANDLER_PATH") {
        let path = std::path::PathBuf::from(handler);
        if path.is_file() {
            if let Some(parent) = path.parent() {
                return Some(parent.to_path_buf());
            }
        } else if let Some(parent) = path.parent() {
            return Some(parent.to_path_buf());
        }
    }
    std::env::current_dir().ok()
}

#[derive(Copy, Clone)]
enum ProjectKind {
    Php,
    Js,
    Other,
}

fn project_kind() -> ProjectKind {
    if std::env::var("PHPX_MODULE_ROOT").is_ok() {
        return ProjectKind::Php;
    }
    if let Ok(handler) = std::env::var("HANDLER_PATH") {
        if handler.ends_with(".phpx") || handler.ends_with(".php") {
            return ProjectKind::Php;
        }
        if handler.ends_with(".ts")
            || handler.ends_with(".tsx")
            || handler.ends_with(".js")
            || handler.ends_with(".jsx")
        {
            return ProjectKind::Js;
        }
    }
    ProjectKind::Other
}

fn default_example(capability: &str, project_kind: ProjectKind) -> Option<String> {
    let example = match (project_kind, capability) {
        (ProjectKind::Php, "read") => "security.allow.read = [\"./php_modules\"]",
        (ProjectKind::Php, "write") => "security.allow.write = [\"./php_modules/.cache\"]",
        (ProjectKind::Js, "read") => "security.allow.read = [\"./src\"]",
        (ProjectKind::Js, "write") => "security.allow.write = [\"./.cache\"]",
        _ => return None,
    };
    Some(example.to_string())
}

fn is_common_target(target: &str, project_kind: ProjectKind, capability: &str) -> bool {
    let target = target.replace('\\', "/");
    match (project_kind, capability) {
        (ProjectKind::Php, "read") => target.contains("/php_modules/") || target.ends_with("/deka.lock"),
        (ProjectKind::Php, "write") => target.contains("/php_modules/.cache/") || target.ends_with("/deka.lock"),
        (ProjectKind::Js, "read") => {
            target.contains("/src/")
                || target.contains("/deps/")
                || target.contains("/node_modules/")
                || target.ends_with(".ts")
                || target.ends_with(".js")
        }
        (ProjectKind::Js, "write") => {
            target.contains("/.cache/")
                || target.ends_with(".cache")
                || target.contains("/dist/")
                || target.contains("/build/")
        }
        _ => false,
    }
}

fn patch_for_suggestion(capability: &str, suggestion: &str) -> Option<String> {
    let list_start = suggestion.find('[')?;
    let list_end = suggestion.rfind(']')?;
    let items = suggestion.get(list_start + 1..list_end)?.trim();
    let patch = format!(
        "{{\n  \"security\": {{\n    \"allow\": {{\n      \"{}\": [{}]\n    }}\n  }}\n}}",
        capability, items
    );
    Some(patch)
}

fn update_deka_json_allow(capability: &str, target: Option<&str>) -> Result<(), String> {
    let target = target
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| default_allow_target_for_capability(capability))
        .ok_or("missing target")?;
    if target.is_empty() {
        return Err("empty target".to_string());
    }
    let project_kind = project_kind();
    let root = project_root().ok_or("failed to determine project root")?;
    let path = root.join("deka.json");
    let mut doc = if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        serde_json::from_str::<Value>(&raw)
            .map_err(|err| format!("invalid JSON in {}: {}", path.display(), err))?
    } else {
        Value::Object(Map::new())
    };

    let root_obj = doc
        .as_object_mut()
        .ok_or("deka.json root must be an object")?;
    let security = root_obj
        .entry("security")
        .or_insert_with(|| Value::Object(Map::new()));
    let security_obj = security
        .as_object_mut()
        .ok_or("security must be an object")?;
    let allow = security_obj
        .entry("allow")
        .or_insert_with(|| Value::Object(Map::new()));
    let allow_obj = allow
        .as_object_mut()
        .ok_or("security.allow must be an object")?;

    let items = rule_items_for_request(capability, target, project_kind);
    if items.is_empty() {
        return Err("no allow items to apply".to_string());
    }

    let entry = allow_obj.entry(capability).or_insert(Value::Bool(false));
    if entry.as_bool() == Some(true) {
        return Ok(());
    }

    let mut list = Vec::new();
    match entry {
        Value::Bool(_) => {}
        Value::String(existing) => {
            list.push(existing.clone());
        }
        Value::Array(existing) => {
            for value in existing.iter() {
                if let Some(item) = value.as_str() {
                    if !item.trim().is_empty() {
                        list.push(item.to_string());
                    }
                }
            }
        }
        _ => {
            return Err(format!(
                "security.allow.{} must be boolean, string, or array",
                capability
            ));
        }
    }

    for item in items {
        if !list.iter().any(|existing| existing == &item) {
            list.push(item);
        }
    }

    *entry = Value::Array(list.into_iter().map(Value::String).collect());

    let payload = serde_json::to_string_pretty(&doc)
        .map_err(|err| format!("failed to serialize deka.json: {}", err))?;
    std::fs::write(&path, payload)
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    Ok(())
}

fn default_allow_target_for_capability(capability: &str) -> Option<&'static str> {
    match capability {
        // Scope-less capability requests should persist as wildcard allows.
        "env" | "net" | "run" | "db" | "wasm" => Some("*"),
        _ => None,
    }
}

fn rule_items_for_request(capability: &str, target: &str, project_kind: ProjectKind) -> Vec<String> {
    match capability {
        "read" => rule_items_for_path(target, project_kind, true),
        "write" => rule_items_for_path(target, project_kind, false),
        "net" | "env" | "run" | "db" | "wasm" => vec![target.trim().to_string()],
        _ => Vec::new(),
    }
}

fn rule_items_for_path(target: &str, _project_kind: ProjectKind, is_read: bool) -> Vec<String> {
    let root = project_root();
    let path = std::path::Path::new(target);
    let rel = root
        .as_ref()
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    let rel_str = normalize_rel_like(&rel.to_string_lossy().replace('\\', "/"));

    let item = if rel_str.starts_with("php_modules/.cache") {
        "./php_modules/.cache".to_string()
    } else if rel_str.starts_with("php_modules/") {
        "./php_modules".to_string()
    } else if rel_str.starts_with("src/") {
        "./src".to_string()
    } else if rel_str.starts_with("deps/") {
        "./deps".to_string()
    } else if rel_str.starts_with("node_modules/") {
        "./node_modules".to_string()
    } else if !is_read && rel_str.starts_with("dist/") {
        "./dist".to_string()
    } else if !is_read && rel_str.starts_with("build/") {
        "./build".to_string()
    } else if rel_str == "deka.lock" {
        "./deka.lock".to_string()
    } else if let Some(prefix) = rel.components().next().and_then(|c| c.as_os_str().to_str()) {
        format!("./{}", prefix)
    } else {
        target.to_string()
    };

    vec![item]
}

fn normalize_rel_like(input: &str) -> String {
    let normalized = input.replace('\\', "/");
    let mut out = normalized.trim().to_string();
    while out.starts_with("./") {
        out = out[2..].to_string();
    }
    out
}

fn enforce_read(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    if security_privileged_enabled() {
        match target {
            None => return Ok(()),
            Some(target) if is_internal_security_target(target) => return Ok(()),
            _ => {}
        }
    }
    enforce_scope("read", &policy.allow.read, &policy.deny.read, target)
}

fn enforce_write(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    if security_privileged_enabled() {
        match target {
            None => return Ok(()),
            Some(target) if is_internal_security_target(target) => return Ok(()),
            _ => {}
        }
    }
    enforce_scope("write", &policy.allow.write, &policy.deny.write, target)
}

fn enforce_net(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    enforce_scope("net", &policy.allow.net, &policy.deny.net, target)
}

fn enforce_env(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    enforce_scope("env", &policy.allow.env, &policy.deny.env, target)
}

fn enforce_db(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    enforce_scope("db", &policy.allow.db, &policy.deny.db, target)
}

fn enforce_wasm(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    enforce_scope("wasm", &policy.allow.wasm, &policy.deny.wasm, target)
}

fn db_json_to_proto_value(value: &serde_json::Value) -> proto::bridge_v1::Value {
    use proto::bridge_v1::value::Kind;
    let kind = match value {
        serde_json::Value::Null => Some(Kind::NullValue(proto::bridge_v1::NullValue {})),
        serde_json::Value::Bool(v) => Some(Kind::BoolValue(*v)),
        serde_json::Value::Number(v) => {
            if let Some(i) = v.as_i64() {
                Some(Kind::IntValue(i))
            } else if let Some(f) = v.as_f64() {
                Some(Kind::FloatValue(f))
            } else {
                Some(Kind::StringValue(v.to_string()))
            }
        }
        serde_json::Value::String(v) => Some(Kind::StringValue(v.clone())),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Some(Kind::StringValue(value.to_string()))
        }
    };
    proto::bridge_v1::Value { kind }
}

fn db_proto_to_json_value(value: &proto::bridge_v1::Value) -> serde_json::Value {
    use proto::bridge_v1::value::Kind;
    match value.kind.as_ref() {
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::BoolValue(v)) => serde_json::Value::Bool(*v),
        Some(Kind::IntValue(v)) => serde_json::Value::Number((*v).into()),
        Some(Kind::FloatValue(v)) => serde_json::Number::from_f64(*v)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Some(Kind::StringValue(v)) => serde_json::Value::String(v.clone()),
        Some(Kind::BytesValue(v)) => serde_json::Value::Array(
            v.iter()
                .map(|b| serde_json::Value::Number((*b as u64).into()))
                .collect(),
        ),
        None => serde_json::Value::Null,
    }
}

fn db_proto_request_to_action_payload(
    req: &proto::bridge_v1::DbRequest,
) -> Result<(String, serde_json::Value, DbProtoActionKind), deno_core::error::CoreError> {
    use proto::bridge_v1::db_request::Action;
    let Some(action) = req.action.as_ref() else {
        return Err(core_err("db proto request missing action"));
    };
    match action {
        Action::Open(open) => {
            let mut cfg = serde_json::Map::new();
            for item in &open.config {
                let value = item
                    .value
                    .as_ref()
                    .map(db_proto_to_json_value)
                    .unwrap_or(serde_json::Value::Null);
                cfg.insert(item.key.clone(), value);
            }
            Ok((
                "open".to_string(),
                serde_json::json!({
                    "driver": open.driver,
                    "config": cfg
                }),
                DbProtoActionKind::Open,
            ))
        }
        Action::Query(query) => Ok((
            "query".to_string(),
            serde_json::json!({
                "handle": query.handle,
                "sql": query.sql,
                "params": query.params.iter().map(db_proto_to_json_value).collect::<Vec<_>>()
            }),
            DbProtoActionKind::Query,
        )),
        Action::QueryOne(query) => Ok((
            "query".to_string(),
            serde_json::json!({
                "handle": query.handle,
                "sql": query.sql,
                "params": query.params.iter().map(db_proto_to_json_value).collect::<Vec<_>>()
            }),
            DbProtoActionKind::QueryOne,
        )),
        Action::Exec(exec) => Ok((
            "exec".to_string(),
            serde_json::json!({
                "handle": exec.handle,
                "sql": exec.sql,
                "params": exec.params.iter().map(db_proto_to_json_value).collect::<Vec<_>>()
            }),
            DbProtoActionKind::Exec,
        )),
        Action::Begin(h) => Ok((
            "begin".to_string(),
            serde_json::json!({ "handle": h.handle }),
            DbProtoActionKind::Begin,
        )),
        Action::Commit(h) => Ok((
            "commit".to_string(),
            serde_json::json!({ "handle": h.handle }),
            DbProtoActionKind::Commit,
        )),
        Action::Rollback(h) => Ok((
            "rollback".to_string(),
            serde_json::json!({ "handle": h.handle }),
            DbProtoActionKind::Rollback,
        )),
        Action::Close(h) => Ok((
            "close".to_string(),
            serde_json::json!({ "handle": h.handle }),
            DbProtoActionKind::Close,
        )),
        Action::Stats(_) => Ok((
            "stats".to_string(),
            serde_json::json!({}),
            DbProtoActionKind::Stats,
        )),
    }
}

fn db_action_payload_to_proto_request(
    action: &str,
    payload: &serde_json::Value,
) -> Result<proto::bridge_v1::DbRequest, deno_core::error::CoreError> {
    use proto::bridge_v1::db_request::Action;
    let args = payload.as_object().cloned().unwrap_or_default();
    let action = match action {
        "open" => {
            let driver = args
                .get("driver")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut config = Vec::new();
            if let Some(cfg) = args.get("config").and_then(|v| v.as_object()) {
                for (key, val) in cfg {
                    config.push(proto::bridge_v1::NamedValue {
                        key: key.clone(),
                        value: Some(db_json_to_proto_value(val)),
                    });
                }
            }
            Action::Open(proto::bridge_v1::DbOpenRequest { driver, config })
        }
        "query" => {
            let handle = args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0);
            let sql = args
                .get("sql")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let params = args
                .get("params")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(db_json_to_proto_value)
                .collect();
            Action::Query(proto::bridge_v1::DbQueryRequest {
                handle,
                sql,
                params,
            })
        }
        "query_one" => {
            let handle = args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0);
            let sql = args
                .get("sql")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let params = args
                .get("params")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(db_json_to_proto_value)
                .collect();
            Action::QueryOne(proto::bridge_v1::DbQueryRequest {
                handle,
                sql,
                params,
            })
        }
        "exec" => {
            let handle = args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0);
            let sql = args
                .get("sql")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let params = args
                .get("params")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(db_json_to_proto_value)
                .collect();
            Action::Exec(proto::bridge_v1::DbExecRequest {
                handle,
                sql,
                params,
            })
        }
        "begin" => Action::Begin(proto::bridge_v1::DbHandleRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        "commit" => Action::Commit(proto::bridge_v1::DbHandleRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        "rollback" => Action::Rollback(proto::bridge_v1::DbHandleRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        "close" => Action::Close(proto::bridge_v1::DbHandleRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        "stats" => Action::Stats(true),
        other => {
            return Err(core_err(format!("unsupported db proto action '{}'", other)));
        }
    };

    Ok(proto::bridge_v1::DbRequest {
        schema_version: 1,
        action: Some(action),
    })
}

fn db_json_rows_to_proto_rows(value: &serde_json::Value) -> Vec<proto::bridge_v1::Row> {
    let Some(rows) = value.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(obj) = row.as_object() {
            let fields = obj
                .iter()
                .map(|(key, val)| proto::bridge_v1::NamedValue {
                    key: key.clone(),
                    value: Some(db_json_to_proto_value(val)),
                })
                .collect();
            out.push(proto::bridge_v1::Row { fields });
        }
    }
    out
}

fn db_proto_rows_to_json(rows: &[proto::bridge_v1::Row]) -> serde_json::Value {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut obj = serde_json::Map::new();
        for field in &row.fields {
            let val = field
                .value
                .as_ref()
                .map(db_proto_to_json_value)
                .unwrap_or(serde_json::Value::Null);
            obj.insert(field.key.clone(), val);
        }
        out.push(serde_json::Value::Object(obj));
    }
    serde_json::Value::Array(out)
}

fn db_json_response_to_proto(
    resp: &serde_json::Value,
    kind: DbProtoActionKind,
) -> proto::bridge_v1::DbResponse {
    use proto::bridge_v1::db_response::Action;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let error = resp
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let action = match kind {
        DbProtoActionKind::Open => {
            let handle = resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0);
            let reused = resp
                .get("reused")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(Action::Open(proto::bridge_v1::DbOpenResponse {
                handle,
                reused,
            }))
        }
        DbProtoActionKind::Query => {
            let rows =
                db_json_rows_to_proto_rows(resp.get("rows").unwrap_or(&serde_json::Value::Null));
            Some(Action::Query(proto::bridge_v1::DbRowsResponse { rows }))
        }
        DbProtoActionKind::QueryOne => {
            let mut rows =
                db_json_rows_to_proto_rows(resp.get("rows").unwrap_or(&serde_json::Value::Null));
            if rows.len() > 1 {
                rows.truncate(1);
            }
            Some(Action::QueryOne(proto::bridge_v1::DbRowsResponse { rows }))
        }
        DbProtoActionKind::Exec => {
            let affected_rows = resp
                .get("affected_rows")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Some(Action::Exec(proto::bridge_v1::DbExecResponse {
                affected_rows,
            }))
        }
        DbProtoActionKind::Begin => Some(Action::Begin(proto::bridge_v1::DbUnitResponse { ok })),
        DbProtoActionKind::Commit => Some(Action::Commit(proto::bridge_v1::DbUnitResponse { ok })),
        DbProtoActionKind::Rollback => {
            Some(Action::Rollback(proto::bridge_v1::DbUnitResponse { ok }))
        }
        DbProtoActionKind::Close => Some(Action::Close(proto::bridge_v1::DbUnitResponse { ok })),
        DbProtoActionKind::Stats => {
            let active_handles = resp
                .get("active_handles")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let statement_cache_entries = resp
                .get("statement_cache_entries")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let statement_cache_hits = resp
                .get("statement_cache_hits")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let statement_cache_misses = resp
                .get("statement_cache_misses")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let mut handles_by_driver = Vec::new();
            if let Some(obj) = resp.get("handles_by_driver").and_then(|v| v.as_object()) {
                for (driver, count_val) in obj {
                    let count = count_val.as_u64().unwrap_or(0);
                    handles_by_driver.push(proto::bridge_v1::DriverCount {
                        driver: driver.clone(),
                        count,
                    });
                }
            }

            let mut metrics = Vec::new();
            if let Some(obj) = resp.get("metrics").and_then(|v| v.as_object()) {
                for (key, metric_val) in obj {
                    let calls = metric_val
                        .get("calls")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let errors = metric_val
                        .get("errors")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let total_ms = metric_val
                        .get("total_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let avg_ms = metric_val
                        .get("avg_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    metrics.push(proto::bridge_v1::NamedMetric {
                        key: key.clone(),
                        metric: Some(proto::bridge_v1::DbStatsMetric {
                            calls,
                            errors,
                            total_ms,
                            avg_ms,
                        }),
                    });
                }
            }

            Some(Action::Stats(proto::bridge_v1::DbStatsResponse {
                active_handles,
                handles_by_driver,
                metrics,
                statement_cache_entries,
                statement_cache_hits,
                statement_cache_misses,
            }))
        }
    };

    proto::bridge_v1::DbResponse {
        schema_version: 1,
        ok,
        error,
        action,
    }
}

fn db_proto_response_to_json(resp: &proto::bridge_v1::DbResponse) -> serde_json::Value {
    use proto::bridge_v1::db_response::Action;
    let mut out = serde_json::Map::new();
    out.insert("ok".to_string(), serde_json::Value::Bool(resp.ok));
    if !resp.error.is_empty() {
        out.insert(
            "error".to_string(),
            serde_json::Value::String(resp.error.clone()),
        );
    }
    if let Some(action) = resp.action.as_ref() {
        match action {
            Action::Open(open) => {
                out.insert(
                    "handle".to_string(),
                    serde_json::Value::Number(open.handle.into()),
                );
                out.insert("reused".to_string(), serde_json::Value::Bool(open.reused));
            }
            Action::Query(rows) | Action::QueryOne(rows) => {
                out.insert("rows".to_string(), db_proto_rows_to_json(&rows.rows));
            }
            Action::Exec(exec) => {
                out.insert(
                    "affected_rows".to_string(),
                    serde_json::Value::Number(exec.affected_rows.into()),
                );
            }
            Action::Begin(unit)
            | Action::Commit(unit)
            | Action::Rollback(unit)
            | Action::Close(unit) => {
                out.insert("ok".to_string(), serde_json::Value::Bool(unit.ok));
            }
            Action::Stats(stats) => {
                out.insert(
                    "active_handles".to_string(),
                    serde_json::Value::Number(stats.active_handles.into()),
                );
                let mut by_driver = serde_json::Map::new();
                for item in &stats.handles_by_driver {
                    by_driver.insert(
                        item.driver.clone(),
                        serde_json::Value::Number(item.count.into()),
                    );
                }
                out.insert(
                    "handles_by_driver".to_string(),
                    serde_json::Value::Object(by_driver),
                );
                out.insert(
                    "statement_cache_entries".to_string(),
                    serde_json::Value::Number(stats.statement_cache_entries.into()),
                );
                out.insert(
                    "statement_cache_hits".to_string(),
                    serde_json::Value::Number(stats.statement_cache_hits.into()),
                );
                out.insert(
                    "statement_cache_misses".to_string(),
                    serde_json::Value::Number(stats.statement_cache_misses.into()),
                );

                let mut metrics = serde_json::Map::new();
                for metric in &stats.metrics {
                    let m = metric.metric.as_ref();
                    metrics.insert(
                        metric.key.clone(),
                        serde_json::json!({
                            "calls": m.map(|x| x.calls).unwrap_or(0),
                            "errors": m.map(|x| x.errors).unwrap_or(0),
                            "total_ms": m.map(|x| x.total_ms).unwrap_or(0),
                            "avg_ms": m.map(|x| x.avg_ms).unwrap_or(0),
                        }),
                    );
                }
                out.insert("metrics".to_string(), serde_json::Value::Object(metrics));
            }
        }
    }
    serde_json::Value::Object(out)
}

fn db_call_proto_impl(request: &[u8]) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let started = Instant::now();
    let req = proto::bridge_v1::DbRequest::decode(request)
        .map_err(|e| core_err(format!("db proto decode failed: {}", e)))?;
    let (action, payload, kind) = db_proto_request_to_action_payload(&req)?;
    let db_target = db_target_from_payload(&action, &payload);
    let target = db_target.as_deref().unwrap_or("*");
    enforce_db(Some(target))?;
    let response_json = db_call_impl(action, payload)?;
    let response = db_json_response_to_proto(&response_json, kind);
    let out = response.encode_to_vec();
    record_bridge_proto_metric(
        "db",
        request.len(),
        out.len(),
        started.elapsed().as_micros() as u64,
    );
    Ok(out)
}

fn db_target_from_payload(action: &str, payload: &serde_json::Value) -> Option<String> {
    if action == "stats" {
        return Some("stats".to_string());
    }
    let obj = payload.as_object()?;
    if let Some(driver) = obj.get("driver").and_then(|v| v.as_str()) {
        let trimmed = driver.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let handle = obj.get("handle").and_then(|v| v.as_u64())?;
    let state = db_state().lock().ok()?;
    let conn = state.handles.get(&handle)?;
    Some(conn.config.driver_name().to_string())
}

#[op2]
#[buffer]
fn op_php_db_call_proto(#[buffer] request: &[u8]) -> Result<Vec<u8>, deno_core::error::CoreError> {
    db_call_proto_impl(request)
}

#[op2]
#[buffer]
fn op_php_db_proto_encode(
    #[string] action: String,
    #[serde] payload: serde_json::Value,
) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let request = db_action_payload_to_proto_request(&action, &payload)?;
    Ok(request.encode_to_vec())
}

#[op2]
#[serde]
fn op_php_db_proto_decode(
    #[buffer] response: &[u8],
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let decoded = proto::bridge_v1::DbResponse::decode(response)
        .map_err(|e| core_err(format!("db proto decode response failed: {}", e)))?;
    Ok(db_proto_response_to_json(&decoded))
}

fn net_call_impl(
    action: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let err = |msg: String| {
        deno_core::error::CoreError::from(std::io::Error::new(std::io::ErrorKind::Other, msg))
    };

    let args_obj = args.as_object().cloned().unwrap_or_default();
    match action.as_str() {
        "connect" => {
            let host = args_obj
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .trim_matches('\0')
                .to_string();
            let port = args_obj.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
            if port == 0 {
                return Ok(
                    serde_json::json!({ "ok": false, "error": "connect: missing or invalid port" }),
                );
            }
            let timeout_ms = args_obj
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(5000);
            let addr = format!("{}:{}", host, port);
            let mut addrs = addr
                .to_socket_addrs()
                .map_err(|e| err(format!("connect: resolve failed: {}", e)))?;
            let target = addrs
                .next()
                .ok_or_else(|| err("connect: no resolved address".to_string()))?;
            let stream = TcpStream::connect_timeout(&target, Duration::from_millis(timeout_ms))
                .map_err(|e| err(format!("connect: {}", e)))?;
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            let handle = state.next_handle;
            state.next_handle += 1;
            state.handles.insert(handle, NetConn::Tcp(stream));
            Ok(serde_json::json!({ "ok": true, "handle": handle }))
        }
        "set_deadline" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("set_deadline: missing handle".to_string()))?;
            let millis = args_obj.get("millis").and_then(|v| v.as_u64()).unwrap_or(0);
            let timeout = if millis == 0 {
                None
            } else {
                Some(Duration::from_millis(millis))
            };
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            let Some(conn) = state.handles.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("set_deadline: unknown handle {}", handle) }),
                );
            };
            let result = match conn {
                NetConn::Tcp(stream) => stream
                    .set_read_timeout(timeout)
                    .and_then(|_| stream.set_write_timeout(timeout)),
                NetConn::Tls(stream) => stream
                    .get_ref()
                    .set_read_timeout(timeout)
                    .and_then(|_| stream.get_ref().set_write_timeout(timeout)),
            };
            match result {
                Ok(()) => Ok(serde_json::json!({ "ok": true })),
                Err(e) => {
                    Ok(serde_json::json!({ "ok": false, "error": format!("set_deadline: {}", e) }))
                }
            }
        }
        "read" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("read: missing handle".to_string()))?;
            let max_bytes = args_obj
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(4096) as usize;
            let mut buf = vec![0_u8; max_bytes.max(1)];
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            let Some(conn) = state.handles.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("read: unknown handle {}", handle) }),
                );
            };
            let n = match conn {
                NetConn::Tcp(stream) => stream.read(&mut buf),
                NetConn::Tls(stream) => stream.read(&mut buf),
            };
            match n {
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    Ok(serde_json::json!({ "ok": true, "data": data, "eof": n == 0 }))
                }
                Err(e) => Ok(serde_json::json!({ "ok": false, "error": format!("read: {}", e) })),
            }
        }
        "write" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("write: missing handle".to_string()))?;
            let data = args_obj
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .as_bytes()
                .to_vec();
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            let Some(conn) = state.handles.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("write: unknown handle {}", handle) }),
                );
            };
            let result = match conn {
                NetConn::Tcp(stream) => stream.write_all(&data),
                NetConn::Tls(stream) => stream.write_all(&data),
            };
            match result {
                Ok(()) => Ok(serde_json::json!({ "ok": true, "written": data.len() })),
                Err(e) => Ok(serde_json::json!({ "ok": false, "error": format!("write: {}", e) })),
            }
        }
        "tls_upgrade" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("tls_upgrade: missing handle".to_string()))?;
            let server_name = args_obj
                .get("server_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if server_name.is_empty() {
                return Ok(
                    serde_json::json!({ "ok": false, "error": "tls_upgrade: missing server_name" }),
                );
            }
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            let Some(conn) = state.handles.remove(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("tls_upgrade: unknown handle {}", handle) }),
                );
            };
            let tcp = match conn {
                NetConn::Tcp(stream) => stream,
                NetConn::Tls(stream) => {
                    let new_handle = state.next_handle;
                    state.next_handle += 1;
                    state.handles.insert(new_handle, NetConn::Tls(stream));
                    return Ok(
                        serde_json::json!({ "ok": true, "handle": new_handle, "reused": true }),
                    );
                }
            };
            let connector = TlsConnector::new()
                .map_err(|e| err(format!("tls_upgrade: connector init failed: {}", e)))?;
            match connector.connect(&server_name, tcp) {
                Ok(stream) => {
                    let new_handle = state.next_handle;
                    state.next_handle += 1;
                    state.handles.insert(new_handle, NetConn::Tls(stream));
                    Ok(serde_json::json!({ "ok": true, "handle": new_handle }))
                }
                Err(e) => {
                    Ok(serde_json::json!({ "ok": false, "error": format!("tls_upgrade: {}", e) }))
                }
            }
        }
        "close" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("close: missing handle".to_string()))?;
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            state.handles.remove(&handle);
            Ok(serde_json::json!({ "ok": true }))
        }
        _ => Ok(serde_json::json!({
            "ok": false,
            "error": format!("unknown net action '{}'", action)
        })),
    }
}

#[derive(Clone, Copy)]
enum NetProtoActionKind {
    Connect,
    SetDeadline,
    Read,
    Write,
    TlsUpgrade,
    Close,
}

fn net_action_payload_to_proto_request(
    action: &str,
    payload: &serde_json::Value,
) -> Result<proto::bridge_v1::NetRequest, deno_core::error::CoreError> {
    use proto::bridge_v1::net_request::Action;
    let args = payload.as_object().cloned().unwrap_or_default();
    let action = match action {
        "connect" => Action::Connect(proto::bridge_v1::NetConnectRequest {
            host: args
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .to_string(),
            port: args.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            timeout_ms: args
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(5000),
        }),
        "set_deadline" => Action::SetDeadline(proto::bridge_v1::NetDeadlineRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            millis: args.get("millis").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        "read" => Action::Read(proto::bridge_v1::NetReadRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            max_bytes: args
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(4096),
        }),
        "write" => Action::Write(proto::bridge_v1::NetWriteRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            data: args
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "tls_upgrade" => Action::TlsUpgrade(proto::bridge_v1::NetTlsUpgradeRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            server_name: args
                .get("server_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "close" => Action::Close(proto::bridge_v1::NetHandleRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        other => {
            return Err(core_err(format!(
                "unsupported net proto action '{}'",
                other
            )));
        }
    };
    Ok(proto::bridge_v1::NetRequest {
        schema_version: 1,
        action: Some(action),
    })
}

fn net_proto_request_to_action_payload(
    req: &proto::bridge_v1::NetRequest,
) -> Result<(String, serde_json::Value, NetProtoActionKind), deno_core::error::CoreError> {
    use proto::bridge_v1::net_request::Action;
    let Some(action) = req.action.as_ref() else {
        return Err(core_err("net proto request missing action"));
    };
    match action {
        Action::Connect(connect) => Ok((
            "connect".to_string(),
            serde_json::json!({
                "host": connect.host,
                "port": connect.port,
                "timeout_ms": connect.timeout_ms,
            }),
            NetProtoActionKind::Connect,
        )),
        Action::SetDeadline(deadline) => Ok((
            "set_deadline".to_string(),
            serde_json::json!({
                "handle": deadline.handle,
                "millis": deadline.millis,
            }),
            NetProtoActionKind::SetDeadline,
        )),
        Action::Read(read) => Ok((
            "read".to_string(),
            serde_json::json!({
                "handle": read.handle,
                "max_bytes": read.max_bytes,
            }),
            NetProtoActionKind::Read,
        )),
        Action::Write(write) => Ok((
            "write".to_string(),
            serde_json::json!({
                "handle": write.handle,
                "data": write.data,
            }),
            NetProtoActionKind::Write,
        )),
        Action::TlsUpgrade(upgrade) => Ok((
            "tls_upgrade".to_string(),
            serde_json::json!({
                "handle": upgrade.handle,
                "server_name": upgrade.server_name,
            }),
            NetProtoActionKind::TlsUpgrade,
        )),
        Action::Close(close) => Ok((
            "close".to_string(),
            serde_json::json!({ "handle": close.handle }),
            NetProtoActionKind::Close,
        )),
    }
}

fn net_json_response_to_proto(
    resp: &serde_json::Value,
    kind: NetProtoActionKind,
) -> proto::bridge_v1::NetResponse {
    use proto::bridge_v1::net_response::Action;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let error = resp
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let action = match kind {
        NetProtoActionKind::Connect => Some(Action::Connect(proto::bridge_v1::NetHandleResponse {
            handle: resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            reused: resp
                .get("reused")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })),
        NetProtoActionKind::SetDeadline => {
            Some(Action::SetDeadline(proto::bridge_v1::NetUnitResponse {
                ok,
            }))
        }
        NetProtoActionKind::Read => Some(Action::Read(proto::bridge_v1::NetReadResponse {
            data: resp
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            eof: resp.get("eof").and_then(|v| v.as_bool()).unwrap_or(false),
        })),
        NetProtoActionKind::Write => Some(Action::Write(proto::bridge_v1::NetWriteResponse {
            written: resp.get("written").and_then(|v| v.as_u64()).unwrap_or(0),
        })),
        NetProtoActionKind::TlsUpgrade => {
            Some(Action::TlsUpgrade(proto::bridge_v1::NetHandleResponse {
                handle: resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
                reused: resp
                    .get("reused")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            }))
        }
        NetProtoActionKind::Close => Some(Action::Close(proto::bridge_v1::NetUnitResponse { ok })),
    };

    proto::bridge_v1::NetResponse {
        schema_version: 1,
        ok,
        error,
        action,
    }
}

fn net_proto_response_to_json(resp: &proto::bridge_v1::NetResponse) -> serde_json::Value {
    use proto::bridge_v1::net_response::Action;
    let mut out = serde_json::Map::new();
    out.insert("ok".to_string(), serde_json::Value::Bool(resp.ok));
    if !resp.error.is_empty() {
        out.insert(
            "error".to_string(),
            serde_json::Value::String(resp.error.clone()),
        );
    }

    if let Some(action) = resp.action.as_ref() {
        match action {
            Action::Connect(handle) | Action::TlsUpgrade(handle) => {
                out.insert(
                    "handle".to_string(),
                    serde_json::Value::Number(handle.handle.into()),
                );
                out.insert("reused".to_string(), serde_json::Value::Bool(handle.reused));
            }
            Action::SetDeadline(unit) | Action::Close(unit) => {
                out.insert("ok".to_string(), serde_json::Value::Bool(unit.ok));
            }
            Action::Read(read) => {
                out.insert(
                    "data".to_string(),
                    serde_json::Value::String(read.data.clone()),
                );
                out.insert("eof".to_string(), serde_json::Value::Bool(read.eof));
            }
            Action::Write(write) => {
                out.insert(
                    "written".to_string(),
                    serde_json::Value::Number(write.written.into()),
                );
            }
        }
    }

    serde_json::Value::Object(out)
}

fn net_call_proto_impl(request: &[u8]) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let started = Instant::now();
    let req = proto::bridge_v1::NetRequest::decode(request)
        .map_err(|e| core_err(format!("net proto decode failed: {}", e)))?;
    let (action, payload, kind) = net_proto_request_to_action_payload(&req)?;
    let net_target = payload.get("host").and_then(|v| v.as_str()).or(Some("*"));
    enforce_net(net_target)?;
    let response_json = net_call_impl(action, payload)?;
    let response = net_json_response_to_proto(&response_json, kind);
    let out = response.encode_to_vec();
    record_bridge_proto_metric(
        "net",
        request.len(),
        out.len(),
        started.elapsed().as_micros() as u64,
    );
    Ok(out)
}

#[op2]
#[buffer]
fn op_php_net_call_proto(#[buffer] request: &[u8]) -> Result<Vec<u8>, deno_core::error::CoreError> {
    net_call_proto_impl(request)
}

#[op2]
#[buffer]
fn op_php_net_proto_encode(
    #[string] action: String,
    #[serde] payload: serde_json::Value,
) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let request = net_action_payload_to_proto_request(&action, &payload)?;
    Ok(request.encode_to_vec())
}

#[op2]
#[serde]
fn op_php_net_proto_decode(
    #[buffer] response: &[u8],
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let decoded = proto::bridge_v1::NetResponse::decode(response)
        .map_err(|e| core_err(format!("net proto decode response failed: {}", e)))?;
    Ok(net_proto_response_to_json(&decoded))
}

fn fs_call_impl(
    action: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let err = |msg: String| {
        deno_core::error::CoreError::from(std::io::Error::new(std::io::ErrorKind::Other, msg))
    };

    let to_bytes = |value: Option<&serde_json::Value>| -> Vec<u8> {
        let Some(value) = value else {
            return Vec::new();
        };
        if let Some(arr) = value.as_array() {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                let byte = item.as_u64().unwrap_or(0).min(255) as u8;
                out.push(byte);
            }
            return out;
        }
        if let Some(s) = value.as_str() {
            return s.as_bytes().to_vec();
        }
        Vec::new()
    };

    let args_obj = args.as_object().cloned().unwrap_or_default();
    match action.as_str() {
        "open" => {
            let path = args_obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if path.is_empty() {
                return Ok(serde_json::json!({ "ok": false, "error": "open: missing path" }));
            }

            let mode = args_obj.get("mode").and_then(|v| v.as_str()).unwrap_or("r");
            let mut opts = OpenOptions::new();
            let read = mode.contains('r') || mode.contains('+');
            let write = mode.contains('w')
                || mode.contains('a')
                || mode.contains('x')
                || mode.contains('c')
                || mode.contains('+');
            let append = mode.contains('a');
            let truncate = mode.contains('w');
            let create = mode.contains('w')
                || mode.contains('a')
                || mode.contains('x')
                || mode.contains('c');
            let create_new = mode.contains('x');

            opts.read(read)
                .write(write)
                .append(append)
                .truncate(truncate)
                .create(create)
                .create_new(create_new);

            let file = match opts.open(&path) {
                Ok(file) => file,
                Err(e) => {
                    return Ok(serde_json::json!({
                        "ok": false,
                        "error": format!("open: {}", e)
                    }));
                }
            };

            let mut state = fs_state()
                .lock()
                .map_err(|_| err("fs lock poisoned".to_string()))?;
            let handle = state.next_handle;
            state.next_handle += 1;
            state.handles.insert(handle, file);
            Ok(serde_json::json!({ "ok": true, "handle": handle }))
        }
        "read" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("read: missing handle".to_string()))?;
            let max_bytes = args_obj
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(65536) as usize;
            let mut buf = vec![0_u8; max_bytes.max(1)];

            let mut state = fs_state()
                .lock()
                .map_err(|_| err("fs lock poisoned".to_string()))?;
            let Some(file) = state.handles.get_mut(&handle) else {
                return Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("read: unknown handle {}", handle)
                }));
            };

            match file.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    Ok(serde_json::json!({
                        "ok": true,
                        "data": buf,
                        "eof": n == 0
                    }))
                }
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("read: {}", e)
                })),
            }
        }
        "write" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("write: missing handle".to_string()))?;
            let data = to_bytes(args_obj.get("data"));

            let mut state = fs_state()
                .lock()
                .map_err(|_| err("fs lock poisoned".to_string()))?;
            let Some(file) = state.handles.get_mut(&handle) else {
                return Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("write: unknown handle {}", handle)
                }));
            };

            match file.write_all(&data) {
                Ok(()) => Ok(serde_json::json!({ "ok": true, "written": data.len() })),
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("write: {}", e)
                })),
            }
        }
        "close" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("close: missing handle".to_string()))?;
            let mut state = fs_state()
                .lock()
                .map_err(|_| err("fs lock poisoned".to_string()))?;
            state.handles.remove(&handle);
            Ok(serde_json::json!({ "ok": true }))
        }
        "read_file" => {
            let path = args_obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if path.is_empty() {
                return Ok(serde_json::json!({ "ok": false, "error": "read_file: missing path" }));
            }
            match std::fs::read(&path) {
                Ok(data) => Ok(serde_json::json!({ "ok": true, "data": data })),
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("read_file: {}", e)
                })),
            }
        }
        "write_file" => {
            let path = args_obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if path.is_empty() {
                return Ok(serde_json::json!({ "ok": false, "error": "write_file: missing path" }));
            }
            let data = to_bytes(args_obj.get("data"));
            match std::fs::write(&path, &data) {
                Ok(()) => Ok(serde_json::json!({ "ok": true, "written": data.len() })),
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("write_file: {}", e)
                })),
            }
        }
        "read_dir" => {
            let path = args_obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if path.is_empty() {
                return Ok(serde_json::json!({ "ok": false, "error": "read_dir: missing path" }));
            }
            let entries = std::fs::read_dir(&path).map_err(|e| {
                deno_core::error::CoreError::from(std::io::Error::new(
                    e.kind(),
                    format!("read_dir: {}", e),
                ))
            })?;
            let mut out = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|e| {
                    deno_core::error::CoreError::from(std::io::Error::new(
                        e.kind(),
                        format!("read_dir: {}", e),
                    ))
                })?;
                let file_type = entry.file_type().map_err(|e| {
                    deno_core::error::CoreError::from(std::io::Error::new(
                        e.kind(),
                        format!("read_dir: {}", e),
                    ))
                })?;
                out.push(serde_json::json!({
                    "name": entry.file_name().to_string_lossy().to_string(),
                    "is_dir": file_type.is_dir(),
                    "is_file": file_type.is_file(),
                }));
            }
            Ok(serde_json::json!({ "ok": true, "entries": out }))
        }
        "mkdirs" => {
            let path = args_obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if path.is_empty() {
                return Ok(serde_json::json!({ "ok": false, "error": "mkdirs: missing path" }));
            }
            match std::fs::create_dir_all(&path) {
                Ok(()) => Ok(serde_json::json!({ "ok": true })),
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("mkdirs: {}", e)
                })),
            }
        }
        _ => Ok(serde_json::json!({
            "ok": false,
            "error": format!("unknown fs action '{}'", action)
        })),
    }
}

#[derive(Clone, Copy)]
enum FsProtoActionKind {
    Open,
    Read,
    Write,
    Close,
    ReadFile,
    WriteFile,
    ReadDir,
    Mkdirs,
}

fn fs_action_payload_to_proto_request(
    action: &str,
    payload: &serde_json::Value,
) -> Result<proto::bridge_v1::FsRequest, deno_core::error::CoreError> {
    use proto::bridge_v1::fs_request::Action;
    let args = payload.as_object().cloned().unwrap_or_default();
    let action = match action {
        "open" => Action::Open(proto::bridge_v1::FsOpenRequest {
            path: args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            mode: args
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("r")
                .to_string(),
        }),
        "read" => Action::Read(proto::bridge_v1::FsReadRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            max_bytes: args
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(65536),
        }),
        "write" => {
            let data = args
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|x| x.as_u64().unwrap_or(0).min(255) as u8)
                        .collect::<Vec<u8>>()
                })
                .or_else(|| {
                    args.get("data")
                        .and_then(|v| v.as_str())
                        .map(|s| s.as_bytes().to_vec())
                })
                .unwrap_or_default();
            Action::Write(proto::bridge_v1::FsWriteRequest {
                handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
                data,
            })
        }
        "close" => Action::Close(proto::bridge_v1::FsCloseRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        "read_file" => Action::ReadFile(proto::bridge_v1::FsReadFileRequest {
            path: args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "write_file" => {
            let data = args
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|x| x.as_u64().unwrap_or(0).min(255) as u8)
                        .collect::<Vec<u8>>()
                })
                .or_else(|| {
                    args.get("data")
                        .and_then(|v| v.as_str())
                        .map(|s| s.as_bytes().to_vec())
                })
                .unwrap_or_default();
            Action::WriteFile(proto::bridge_v1::FsWriteFileRequest {
                path: args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                data,
            })
        }
        "read_dir" => Action::ReadDir(proto::bridge_v1::FsReadDirRequest {
            path: args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            with_types: args
                .get("with_types")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        }),
        "mkdirs" => Action::Mkdirs(proto::bridge_v1::FsMkdirsRequest {
            path: args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        other => return Err(core_err(format!("unsupported fs proto action '{}'", other))),
    };
    Ok(proto::bridge_v1::FsRequest {
        schema_version: 1,
        action: Some(action),
    })
}

fn fs_proto_request_to_action_payload(
    req: &proto::bridge_v1::FsRequest,
) -> Result<(String, serde_json::Value, FsProtoActionKind), deno_core::error::CoreError> {
    use proto::bridge_v1::fs_request::Action;
    let Some(action) = req.action.as_ref() else {
        return Err(core_err("fs proto request missing action"));
    };
    match action {
        Action::Open(open) => Ok((
            "open".to_string(),
            serde_json::json!({ "path": open.path, "mode": open.mode }),
            FsProtoActionKind::Open,
        )),
        Action::Read(read) => Ok((
            "read".to_string(),
            serde_json::json!({ "handle": read.handle, "max_bytes": read.max_bytes }),
            FsProtoActionKind::Read,
        )),
        Action::Write(write) => Ok((
            "write".to_string(),
            serde_json::json!({
                "handle": write.handle,
                "data": write.data.iter().map(|b| serde_json::Value::Number((*b as u64).into())).collect::<Vec<_>>()
            }),
            FsProtoActionKind::Write,
        )),
        Action::Close(close) => Ok((
            "close".to_string(),
            serde_json::json!({ "handle": close.handle }),
            FsProtoActionKind::Close,
        )),
        Action::ReadFile(read_file) => Ok((
            "read_file".to_string(),
            serde_json::json!({ "path": read_file.path }),
            FsProtoActionKind::ReadFile,
        )),
        Action::WriteFile(write_file) => Ok((
            "write_file".to_string(),
            serde_json::json!({
                "path": write_file.path,
                "data": write_file.data.iter().map(|b| serde_json::Value::Number((*b as u64).into())).collect::<Vec<_>>()
            }),
            FsProtoActionKind::WriteFile,
        )),
        Action::ReadDir(read_dir) => Ok((
            "read_dir".to_string(),
            serde_json::json!({ "path": read_dir.path, "with_types": read_dir.with_types }),
            FsProtoActionKind::ReadDir,
        )),
        Action::Mkdirs(mkdirs) => Ok((
            "mkdirs".to_string(),
            serde_json::json!({ "path": mkdirs.path }),
            FsProtoActionKind::Mkdirs,
        )),
    }
}

fn fs_json_response_to_proto(
    resp: &serde_json::Value,
    kind: FsProtoActionKind,
) -> proto::bridge_v1::FsResponse {
    use proto::bridge_v1::fs_response::Action;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let error = resp
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let action = match kind {
        FsProtoActionKind::Open => Some(Action::Open(proto::bridge_v1::FsOpenResponse {
            handle: resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        })),
        FsProtoActionKind::Read => {
            let data = resp
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|x| x.as_u64().unwrap_or(0).min(255) as u8)
                        .collect::<Vec<u8>>()
                })
                .unwrap_or_default();
            Some(Action::Read(proto::bridge_v1::FsReadResponse {
                data,
                eof: resp.get("eof").and_then(|v| v.as_bool()).unwrap_or(false),
            }))
        }
        FsProtoActionKind::Write => Some(Action::Write(proto::bridge_v1::FsWriteResponse {
            written: resp.get("written").and_then(|v| v.as_u64()).unwrap_or(0),
        })),
        FsProtoActionKind::Close => Some(Action::Close(proto::bridge_v1::FsUnitResponse { ok })),
        FsProtoActionKind::ReadFile => {
            let data = resp
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|x| x.as_u64().unwrap_or(0).min(255) as u8)
                        .collect::<Vec<u8>>()
                })
                .unwrap_or_default();
            Some(Action::ReadFile(proto::bridge_v1::FsReadResponse {
                data,
                eof: true,
            }))
        }
        FsProtoActionKind::WriteFile => {
            Some(Action::WriteFile(proto::bridge_v1::FsWriteResponse {
                written: resp.get("written").and_then(|v| v.as_u64()).unwrap_or(0),
            }))
        }
        FsProtoActionKind::ReadDir => {
            let entries = resp
                .get("entries")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|entry| entry.as_object())
                        .map(|entry| proto::bridge_v1::FsDirEntry {
                            name: entry
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            is_dir: entry.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false),
                            is_file: entry.get("is_file").and_then(|v| v.as_bool()).unwrap_or(false),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some(Action::ReadDir(proto::bridge_v1::FsReadDirResponse { entries }))
        }
        FsProtoActionKind::Mkdirs => Some(Action::Mkdirs(proto::bridge_v1::FsUnitResponse { ok })),
    };
    proto::bridge_v1::FsResponse {
        schema_version: 1,
        ok,
        error,
        action,
    }
}

fn fs_proto_response_to_json(resp: &proto::bridge_v1::FsResponse) -> serde_json::Value {
    use proto::bridge_v1::fs_response::Action;
    let mut out = serde_json::Map::new();
    out.insert("ok".to_string(), serde_json::Value::Bool(resp.ok));
    if !resp.error.is_empty() {
        out.insert(
            "error".to_string(),
            serde_json::Value::String(resp.error.clone()),
        );
    }
    if let Some(action) = resp.action.as_ref() {
        match action {
            Action::Open(open) => {
                out.insert(
                    "handle".to_string(),
                    serde_json::Value::Number(open.handle.into()),
                );
            }
            Action::Read(read) | Action::ReadFile(read) => {
                out.insert(
                    "data".to_string(),
                    serde_json::Value::Array(
                        read.data
                            .iter()
                            .map(|b| serde_json::Value::Number((*b as u64).into()))
                            .collect(),
                    ),
                );
                out.insert("eof".to_string(), serde_json::Value::Bool(read.eof));
            }
            Action::Write(write) | Action::WriteFile(write) => {
                out.insert(
                    "written".to_string(),
                    serde_json::Value::Number(write.written.into()),
                );
            }
            Action::Close(unit) => {
                out.insert("ok".to_string(), serde_json::Value::Bool(unit.ok));
            }
            Action::ReadDir(read_dir) => {
                out.insert(
                    "entries".to_string(),
                    serde_json::Value::Array(
                        read_dir
                            .entries
                            .iter()
                            .map(|entry| {
                                serde_json::json!({
                                    "name": entry.name,
                                    "is_dir": entry.is_dir,
                                    "is_file": entry.is_file,
                                })
                            })
                            .collect(),
                    ),
                );
            }
            Action::Mkdirs(unit) => {
                out.insert("ok".to_string(), serde_json::Value::Bool(unit.ok));
            }
        }
    }
    serde_json::Value::Object(out)
}

fn fs_call_proto_impl(request: &[u8]) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let started = Instant::now();
    let req = proto::bridge_v1::FsRequest::decode(request)
        .map_err(|e| core_err(format!("fs proto decode failed: {}", e)))?;
    let (action, payload, kind) = fs_proto_request_to_action_payload(&req)?;
    let fs_target = payload.get("path").and_then(|v| v.as_str()).or(Some("*"));
    match action.as_str() {
        "read" | "read_file" => enforce_read(fs_target)?,
        "write" | "write_file" | "mkdirs" => enforce_write(fs_target)?,
        "read_dir" => enforce_read(fs_target)?,
        "open" => {
            let mode = payload.get("mode").and_then(|v| v.as_str()).unwrap_or("r");
            if mode.contains('w')
                || mode.contains('a')
                || mode.contains('x')
                || mode.contains('c')
                || mode.contains('+')
            {
                enforce_write(fs_target)?;
            } else {
                enforce_read(fs_target)?;
            }
        }
        _ => {}
    }
    let response_json = fs_call_impl(action, payload)?;
    let response = fs_json_response_to_proto(&response_json, kind);
    let out = response.encode_to_vec();
    record_bridge_proto_metric(
        "fs",
        request.len(),
        out.len(),
        started.elapsed().as_micros() as u64,
    );
    Ok(out)
}

#[op2]
#[buffer]
fn op_php_fs_call_proto(#[buffer] request: &[u8]) -> Result<Vec<u8>, deno_core::error::CoreError> {
    fs_call_proto_impl(request)
}

#[op2]
#[buffer]
fn op_php_fs_proto_encode(
    #[string] action: String,
    #[serde] payload: serde_json::Value,
) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let request = fs_action_payload_to_proto_request(&action, &payload)?;
    Ok(request.encode_to_vec())
}

#[op2]
#[serde]
fn op_php_fs_proto_decode(
    #[buffer] response: &[u8],
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let decoded = proto::bridge_v1::FsResponse::decode(response)
        .map_err(|e| core_err(format!("fs proto decode response failed: {}", e)))?;
    Ok(fs_proto_response_to_json(&decoded))
}

#[op2]
#[serde]
fn op_php_bridge_proto_stats() -> Result<serde_json::Value, deno_core::error::CoreError> {
    let metrics = bridge_proto_metrics()
        .lock()
        .map_err(|_| core_err("bridge proto metrics lock poisoned"))?;
    let mut out = serde_json::Map::new();
    for (key, metric) in metrics.iter() {
        out.insert(
            key.clone(),
            serde_json::json!({
                "calls": metric.calls,
                "total_req_bytes": metric.total_req_bytes,
                "total_resp_bytes": metric.total_resp_bytes,
                "total_us": metric.total_us,
                "avg_us": metric.avg_us,
            }),
        );
    }
    Ok(serde_json::Value::Object(out))
}

#[op2]
#[string]
fn op_php_cwd() -> Result<String, deno_core::error::CoreError> {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| deno_core::error::CoreError::from(e))
}

#[op2(fast)]
fn op_php_file_exists(#[string] path: String) -> bool {
    if enforce_read(Some(&path)).is_err() {
        return false;
    }
    std::path::Path::new(&path).exists()
}

#[op2]
#[string]
fn op_php_path_resolve(#[string] base: String, #[string] path: String) -> String {
    let _ = enforce_read(Some(&base));
    let _ = enforce_read(Some(&path));
    if let Some(stripped) = path.strip_prefix("@/") {
        let root = std::env::var("PHPX_MODULE_ROOT")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
        if !root.is_empty() {
            return std::path::Path::new(&root)
                .join(stripped)
                .to_string_lossy()
                .to_string();
        }
    }

    let base_path = std::path::Path::new(&base);
    let target_path = std::path::Path::new(&path);

    let resolved = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        base_path.join(target_path)
    };

    resolved.to_string_lossy().to_string()
}

#[op2]
#[serde]
fn op_php_read_dir(
    #[string] path: String,
) -> Result<Vec<PhpDirEntry>, deno_core::error::CoreError> {
    enforce_read(Some(&path))?;
    let entries = std::fs::read_dir(&path).map_err(|e| {
        deno_core::error::CoreError::from(std::io::Error::new(
            e.kind(),
            format!("Failed to read dir '{}': {}", path, e),
        ))
    })?;

    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            deno_core::error::CoreError::from(std::io::Error::new(
                e.kind(),
                format!("Failed to read dir entry in '{}': {}", path, e),
            ))
        })?;
        let file_type = entry.file_type().map_err(|e| {
            deno_core::error::CoreError::from(std::io::Error::new(
                e.kind(),
                format!("Failed to read dir entry type in '{}': {}", path, e),
            ))
        })?;
        out.push(PhpDirEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir: file_type.is_dir(),
            is_file: file_type.is_file(),
        });
    }
    Ok(out)
}

#[op2]
#[serde]
fn op_php_parse_wit(
    #[string] path: String,
    #[string] world: String,
) -> Result<WitSchema, deno_core::error::CoreError> {
    enforce_wasm(Some(&path))?;
    enforce_read(Some(&path))?;
    let mut resolve = Resolve::default();
    let (package_id, _) = resolve.push_path(&path).map_err(|err| {
        deno_core::error::CoreError::from(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to parse WIT '{}': {}", path, err),
        ))
    })?;

    let world_id = if world.trim().is_empty() {
        let package = &resolve.packages[package_id];
        if package.worlds.len() != 1 {
            return Err(deno_core::error::CoreError::from(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "WIT package has {} worlds; set deka.json.world",
                    package.worlds.len()
                ),
            )));
        }
        *package.worlds.values().next().expect("worlds len checked")
    } else {
        resolve
            .select_world(package_id, Some(world.trim()))
            .map_err(|err| {
                deno_core::error::CoreError::from(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to select world '{}': {}", world, err),
                ))
            })?
    };

    let world = &resolve.worlds[world_id];
    let mut functions = Vec::new();
    let mut interfaces = Vec::new();

    for (key, item) in world.exports.iter() {
        match item {
            WorldItem::Function(func) => {
                let sig = build_function(&resolve, func);
                let name = world_key_name(key);
                functions.push(WitFunction { name, ..sig });
            }
            WorldItem::Interface { id, .. } => {
                let iface = &resolve.interfaces[*id];
                let iface_name = world_key_name(key);
                let mut iface_functions = Vec::new();
                for func in iface.functions.values() {
                    let sig = build_function(&resolve, func);
                    iface_functions.push(sig);
                }
                interfaces.push(WitInterface {
                    name: iface_name,
                    functions: iface_functions,
                });
            }
            WorldItem::Type(_) => {}
        }
    }

    Ok(WitSchema {
        world: world.name.clone(),
        functions,
        interfaces,
    })
}

#[op2]
#[serde]
fn op_redis_call(
    #[string] action: String,
    #[serde] args: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    Ok(super::redis_mod::redis_call(&action, &args))
}

#[op2]
#[serde]
fn op_neo4j_call(
    #[string] action: String,
    #[serde] args: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    Ok(super::neo4j::neo4j_call(&action, &args))
}

deno_core::extension!(
    php_core,
    ops = [
        op_php_parse_phpx_types,
        op_php_read_file_sync,
        op_php_write_file_sync,
        op_php_mkdirs,
        op_php_set_privileged,
        op_php_sha256,
        op_php_random_bytes,
        op_php_read_env,
        op_php_db_call_proto,
        op_php_db_proto_encode,
        op_php_db_proto_decode,
        op_php_net_call_proto,
        op_php_net_proto_encode,
        op_php_net_proto_decode,
        op_php_fs_call_proto,
        op_php_fs_proto_encode,
        op_php_fs_proto_decode,
        op_php_bridge_proto_stats,
        op_php_cwd,
        op_php_file_exists,
        op_php_path_resolve,
        op_php_read_dir,
        op_php_parse_wit,
        op_neo4j_call,
        op_redis_call,
    ],
    esm_entry_point = "ext:php_core/php.js",
    esm = [dir "src/modules/php", "php.js"],
);

pub fn init() -> deno_core::Extension {
    php_core::init()
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_suffix() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{}_{}", std::process::id(), nanos)
    }

    fn assert_ok(value: &serde_json::Value) {
        assert_eq!(
            value.get("ok").and_then(|v| v.as_bool()),
            Some(true),
            "expected ok response, got: {}",
            value
        );
    }

    #[test]
    fn default_allow_target_uses_wildcard_for_scope_less_capabilities() {
        assert_eq!(default_allow_target_for_capability("env"), Some("*"));
        assert_eq!(default_allow_target_for_capability("net"), Some("*"));
        assert_eq!(default_allow_target_for_capability("run"), Some("*"));
        assert_eq!(default_allow_target_for_capability("db"), Some("*"));
        assert_eq!(default_allow_target_for_capability("wasm"), Some("*"));
        assert_eq!(default_allow_target_for_capability("read"), None);
        assert_eq!(default_allow_target_for_capability("write"), None);
    }

    #[test]
    #[ignore = "requires security capabilities (db grant) not available in unit tests"]
    fn db_proto_open_parity_postgres_mysql_sqlite() {
        let suffix = unique_suffix();
        let cases = vec![
            (
                "postgres",
                serde_json::json!({
                    "host": "127.0.0.1",
                    "port": 5432,
                    "database": format!("db_proto_pg_{}", suffix),
                    "user": "u",
                    "password": "p",
                }),
            ),
            (
                "mysql",
                serde_json::json!({
                    "host": "127.0.0.1",
                    "port": 3306,
                    "database": format!("db_proto_my_{}", suffix),
                    "user": "u",
                    "password": "p",
                }),
            ),
            (
                "sqlite",
                serde_json::json!({
                    "path": format!("/tmp/db_proto_open_{}.sqlite", suffix),
                }),
            ),
        ];

        for (driver, config) in cases {
            let payload = serde_json::json!({
                "driver": driver,
                "config": config
            });

            let json_res =
                db_call_impl("open".to_string(), payload.clone()).expect("json open failed");
            assert_ok(&json_res);
            let json_handle = json_res
                .get("handle")
                .and_then(|v| v.as_u64())
                .expect("json open missing handle");

            let proto_req = db_action_payload_to_proto_request("open", &payload)
                .expect("proto request build failed");
            let proto_bytes = proto_req.encode_to_vec();
            let proto_resp_bytes =
                db_call_proto_impl(&proto_bytes).expect("proto open dispatch failed");
            let proto_resp = proto::bridge_v1::DbResponse::decode(proto_resp_bytes.as_slice())
                .expect("decode failed");
            let proto_json = db_proto_response_to_json(&proto_resp);
            assert_ok(&proto_json);
            let proto_handle = proto_json
                .get("handle")
                .and_then(|v| v.as_u64())
                .expect("proto open missing handle");

            let close_json = db_call_impl(
                "close".to_string(),
                serde_json::json!({ "handle": json_handle }),
            )
            .expect("json close failed");
            assert_ok(&close_json);
            if proto_handle != json_handle {
                let close_proto = db_call_impl(
                    "close".to_string(),
                    serde_json::json!({ "handle": proto_handle }),
                )
                .expect("proto handle close failed");
                assert_ok(&close_proto);
            }
        }
    }

    #[test]
    #[ignore = "requires security capabilities (db grant) not available in unit tests"]
    fn db_proto_sqlite_exec_query_parity() {
        let suffix = unique_suffix();
        let path = format!("/tmp/db_proto_query_{}.sqlite", suffix);
        let open_payload = serde_json::json!({
            "driver": "sqlite",
            "config": { "path": path }
        });
        let open_res = db_call_impl("open".to_string(), open_payload).expect("open failed");
        assert_ok(&open_res);
        let handle = open_res
            .get("handle")
            .and_then(|v| v.as_u64())
            .expect("missing handle");

        let setup_sql = vec![
            "create table if not exists packages (name text, downloads integer)",
            "delete from packages",
            "insert into packages(name, downloads) values ('db', 10), ('component', 20)",
        ];
        for sql in setup_sql {
            let exec_res = db_call_impl(
                "exec".to_string(),
                serde_json::json!({
                    "handle": handle,
                    "sql": sql,
                    "params": []
                }),
            )
            .expect("exec failed");
            assert_ok(&exec_res);
        }

        let json_query = db_call_impl(
            "query".to_string(),
            serde_json::json!({
                "handle": handle,
                "sql": "select name, downloads from packages order by downloads asc",
                "params": []
            }),
        )
        .expect("json query failed");
        assert_ok(&json_query);

        let json_query_again = db_call_impl(
            "query".to_string(),
            serde_json::json!({
                "handle": handle,
                "sql": "select name, downloads from packages order by downloads asc",
                "params": []
            }),
        )
        .expect("json query again failed");
        assert_ok(&json_query_again);

        let proto_req = db_action_payload_to_proto_request(
            "query",
            &serde_json::json!({
                "handle": handle,
                "sql": "select name, downloads from packages order by downloads asc",
                "params": []
            }),
        )
        .expect("proto query request build failed");
        let proto_resp =
            db_call_proto_impl(&proto_req.encode_to_vec()).expect("proto query failed");
        let proto_decoded =
            proto::bridge_v1::DbResponse::decode(proto_resp.as_slice()).expect("decode failed");
        let proto_json = db_proto_response_to_json(&proto_decoded);
        assert_ok(&proto_json);

        assert_eq!(json_query.get("rows"), proto_json.get("rows"));

        let json_stats =
            db_call_impl("stats".to_string(), serde_json::json!({})).expect("json stats failed");
        assert_ok(&json_stats);
        assert!(
            json_stats
                .get("statement_cache_entries")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                >= 1
        );
        assert!(
            json_stats
                .get("statement_cache_hits")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                >= 1
        );
        assert!(
            json_stats
                .get("statement_cache_misses")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                >= 1
        );

        let proto_stats_req = db_action_payload_to_proto_request("stats", &serde_json::json!({}))
            .expect("proto stats request build failed");
        let proto_stats_resp =
            db_call_proto_impl(&proto_stats_req.encode_to_vec()).expect("proto stats failed");
        let proto_stats_decoded = proto::bridge_v1::DbResponse::decode(proto_stats_resp.as_slice())
            .expect("decode stats failed");
        let proto_stats_json = db_proto_response_to_json(&proto_stats_decoded);
        assert_ok(&proto_stats_json);
        assert_eq!(
            json_stats.get("statement_cache_entries"),
            proto_stats_json.get("statement_cache_entries")
        );
        assert_eq!(
            json_stats.get("statement_cache_hits"),
            proto_stats_json.get("statement_cache_hits")
        );
        assert_eq!(
            json_stats.get("statement_cache_misses"),
            proto_stats_json.get("statement_cache_misses")
        );

        let close_res = db_call_impl("close".to_string(), serde_json::json!({ "handle": handle }))
            .expect("close failed");
        assert_ok(&close_res);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore = "requires security capabilities (write grant) not available in unit tests"]
    fn fs_proto_binary_roundtrip_integrity() {
        let suffix = unique_suffix();
        let path = format!("/tmp/fs_proto_roundtrip_{}.bin", suffix);
        let payload = serde_json::json!({
            "path": path,
            "data": [0, 1, 2, 10, 127, 128, 200, 255]
        });

        let json_write = fs_call_impl("write_file".to_string(), payload.clone())
            .expect("json write_file failed");
        assert_ok(&json_write);

        let proto_write_req = fs_action_payload_to_proto_request("write_file", &payload)
            .expect("fs proto write request build failed");
        let proto_write_resp = fs_call_proto_impl(&proto_write_req.encode_to_vec())
            .expect("fs proto write_file dispatch failed");
        let proto_write_decoded = proto::bridge_v1::FsResponse::decode(proto_write_resp.as_slice())
            .expect("fs decode write response failed");
        let proto_write_json = fs_proto_response_to_json(&proto_write_decoded);
        assert_ok(&proto_write_json);

        let json_read = fs_call_impl("read_file".to_string(), serde_json::json!({ "path": path }))
            .expect("json read_file failed");
        assert_ok(&json_read);

        let proto_read_req =
            fs_action_payload_to_proto_request("read_file", &serde_json::json!({ "path": path }))
                .expect("fs proto read request build failed");
        let proto_read_resp = fs_call_proto_impl(&proto_read_req.encode_to_vec())
            .expect("fs proto read_file dispatch failed");
        let proto_read_decoded = proto::bridge_v1::FsResponse::decode(proto_read_resp.as_slice())
            .expect("fs decode read response failed");
        let proto_read_json = fs_proto_response_to_json(&proto_read_decoded);
        assert_ok(&proto_read_json);

        assert_eq!(json_read.get("data"), proto_read_json.get("data"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore = "requires security capabilities (net grant) not available in unit tests"]
    fn net_proto_tcp_parity_sanity() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut buf = [0_u8; 64];
                let n = std::io::Read::read(&mut stream, &mut buf).expect("read");
                std::io::Write::write_all(&mut stream, &buf[..n]).expect("write");
            }
        });

        let json_connect = net_call_impl(
            "connect".to_string(),
            serde_json::json!({
                "host": "127.0.0.1",
                "port": addr.port(),
                "timeout_ms": 3000
            }),
        )
        .expect("json connect failed");
        assert_ok(&json_connect);
        let json_handle = json_connect
            .get("handle")
            .and_then(|v| v.as_u64())
            .expect("json handle missing");

        let json_write = net_call_impl(
            "write".to_string(),
            serde_json::json!({
                "handle": json_handle,
                "data": "ping"
            }),
        )
        .expect("json write failed");
        assert_ok(&json_write);

        let json_read = net_call_impl(
            "read".to_string(),
            serde_json::json!({
                "handle": json_handle,
                "max_bytes": 4
            }),
        )
        .expect("json read failed");
        assert_ok(&json_read);
        assert_eq!(json_read.get("data").and_then(|v| v.as_str()), Some("ping"));

        let json_close = net_call_impl(
            "close".to_string(),
            serde_json::json!({ "handle": json_handle }),
        )
        .expect("json close failed");
        assert_ok(&json_close);

        let proto_connect_req = net_action_payload_to_proto_request(
            "connect",
            &serde_json::json!({
                "host": "127.0.0.1",
                "port": addr.port(),
                "timeout_ms": 3000
            }),
        )
        .expect("proto connect build failed");
        let proto_connect_resp =
            net_call_proto_impl(&proto_connect_req.encode_to_vec()).expect("proto connect failed");
        let proto_connect_json = net_proto_response_to_json(
            &proto::bridge_v1::NetResponse::decode(proto_connect_resp.as_slice())
                .expect("decode connect"),
        );
        assert_ok(&proto_connect_json);
        let proto_handle = proto_connect_json
            .get("handle")
            .and_then(|v| v.as_u64())
            .expect("proto handle missing");

        let proto_write_req = net_action_payload_to_proto_request(
            "write",
            &serde_json::json!({
                "handle": proto_handle,
                "data": "pong"
            }),
        )
        .expect("proto write build failed");
        let proto_write_resp =
            net_call_proto_impl(&proto_write_req.encode_to_vec()).expect("proto write failed");
        let proto_write_json = net_proto_response_to_json(
            &proto::bridge_v1::NetResponse::decode(proto_write_resp.as_slice())
                .expect("decode write"),
        );
        assert_ok(&proto_write_json);

        let proto_read_req = net_action_payload_to_proto_request(
            "read",
            &serde_json::json!({
                "handle": proto_handle,
                "max_bytes": 4
            }),
        )
        .expect("proto read build failed");
        let proto_read_resp =
            net_call_proto_impl(&proto_read_req.encode_to_vec()).expect("proto read failed");
        let proto_read_json = net_proto_response_to_json(
            &proto::bridge_v1::NetResponse::decode(proto_read_resp.as_slice())
                .expect("decode read"),
        );
        assert_ok(&proto_read_json);
        assert_eq!(
            proto_read_json.get("data").and_then(|v| v.as_str()),
            Some("pong")
        );

        let proto_close_req = net_action_payload_to_proto_request(
            "close",
            &serde_json::json!({ "handle": proto_handle }),
        )
        .expect("proto close build failed");
        let proto_close_resp =
            net_call_proto_impl(&proto_close_req.encode_to_vec()).expect("proto close failed");
        let proto_close_json = net_proto_response_to_json(
            &proto::bridge_v1::NetResponse::decode(proto_close_resp.as_slice())
                .expect("decode close"),
        );
        assert_ok(&proto_close_json);

        server.join().expect("server join");
    }

    #[test]
    fn proto_bridge_rejects_malformed_payloads() {
        assert!(db_call_proto_impl(&[0xff, 0x00, 0x01]).is_err());
        assert!(fs_call_proto_impl(&[0xff, 0x00, 0x01]).is_err());
        assert!(net_call_proto_impl(&[0xff, 0x00, 0x01]).is_err());
    }
}

fn world_key_name(key: &WorldKey) -> String {
    match key {
        WorldKey::Name(name) => name.clone(),
        WorldKey::Interface(id) => format!("interface_{}", id.index()),
    }
}

fn build_function(resolve: &Resolve, func: &wit_parser::Function) -> WitFunction {
    let params = func
        .params
        .iter()
        .map(|(name, ty)| WitParam {
            name: name.clone(),
            ty: resolve_type(resolve, ty, &mut HashSet::new()),
        })
        .collect::<Vec<_>>();

    let result = match &func.results {
        Results::Anon(ty) => Some(resolve_type(resolve, ty, &mut HashSet::new())),
        Results::Named(named) => {
            if named.is_empty() {
                None
            } else if named.len() == 1 {
                Some(resolve_type(resolve, &named[0].1, &mut HashSet::new()))
            } else {
                let fields = named
                    .iter()
                    .map(|(name, ty)| WitField {
                        name: name.clone(),
                        ty: resolve_type(resolve, ty, &mut HashSet::new()),
                    })
                    .collect();
                Some(WitType::Record { fields })
            }
        }
    };

    WitFunction {
        name: func.name.clone(),
        params,
        result,
    }
}

fn resolve_type(resolve: &Resolve, ty: &Type, visiting: &mut HashSet<TypeId>) -> WitType {
    match ty {
        Type::Bool => WitType::Bool,
        Type::U8 => WitType::U8,
        Type::U16 => WitType::U16,
        Type::U32 => WitType::U32,
        Type::U64 => WitType::U64,
        Type::S8 => WitType::S8,
        Type::S16 => WitType::S16,
        Type::S32 => WitType::S32,
        Type::S64 => WitType::S64,
        Type::F32 => WitType::F32,
        Type::F64 => WitType::F64,
        Type::Char => WitType::Char,
        Type::String => WitType::String,
        Type::Id(id) => resolve_type_id(resolve, *id, visiting),
    }
}

fn resolve_type_id(resolve: &Resolve, id: TypeId, visiting: &mut HashSet<TypeId>) -> WitType {
    if !visiting.insert(id) {
        return WitType::Unsupported {
            detail: "recursive type".to_string(),
        };
    }
    let ty = &resolve.types[id];
    let out = match &ty.kind {
        TypeDefKind::Record(record) => WitType::Record {
            fields: record
                .fields
                .iter()
                .map(|field| WitField {
                    name: field.name.clone(),
                    ty: resolve_type(resolve, &field.ty, visiting),
                })
                .collect(),
        },
        TypeDefKind::Tuple(tuple) => WitType::Tuple {
            items: tuple
                .types
                .iter()
                .map(|ty| resolve_type(resolve, ty, visiting))
                .collect(),
        },
        TypeDefKind::Option(inner) => WitType::Option {
            some: Box::new(resolve_type(resolve, inner, visiting)),
        },
        TypeDefKind::Result(res) => WitType::Result {
            ok: res
                .ok
                .as_ref()
                .map(|ty| Box::new(resolve_type(resolve, ty, visiting))),
            err: res
                .err
                .as_ref()
                .map(|ty| Box::new(resolve_type(resolve, ty, visiting))),
        },
        TypeDefKind::List(inner) => WitType::List {
            element: Box::new(resolve_type(resolve, inner, visiting)),
        },
        TypeDefKind::Enum(enm) => WitType::Enum {
            cases: enm.cases.iter().map(|c| c.name.clone()).collect(),
        },
        TypeDefKind::Flags(flags) => WitType::Flags {
            flags: flags.flags.iter().map(|f| f.name.clone()).collect(),
        },
        TypeDefKind::Variant(variant) => WitType::Variant {
            cases: variant
                .cases
                .iter()
                .map(|case| WitVariantCase {
                    name: case.name.clone(),
                    ty: case
                        .ty
                        .as_ref()
                        .map(|ty| resolve_type(resolve, ty, visiting)),
                })
                .collect(),
        },
        TypeDefKind::Type(inner) => resolve_type(resolve, inner, visiting),
        TypeDefKind::Resource => WitType::Resource,
        TypeDefKind::Handle(_)
        | TypeDefKind::Future(_)
        | TypeDefKind::Stream(_)
        | TypeDefKind::Unknown => WitType::Unsupported {
            detail: ty.kind.as_str().to_string(),
        },
    };
    visiting.remove(&id);
    out
}
