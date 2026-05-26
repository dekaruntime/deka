use std::{
    collections::HashMap,
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Deserialize;
use simple_dns::{
    CLASS, Name, Packet, PacketFlag, QTYPE, RCODE, ResourceRecord, TYPE,
    rdata::{A, AAAA, MX, NS, RData, SOA, TXT},
};

pub mod doh;
pub mod udp;

const DEFAULT_ZONE: &str = "tana.gg";
const DEFAULT_TTL: u32 = 300;
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(5);
const DEV_UDP_PORT: u16 = 8053;
const PRODUCTION_UDP_PORT: u16 = 53;

#[derive(Debug, Clone)]
pub struct Config {
    pub udp_addr: SocketAddr,
    pub doh_addr: SocketAddr,
    pub redis_url: String,
    pub zone: String,
    pub ns1_host: String,
    pub ns2_host: String,
    pub ns1_ip: Option<IpAddr>,
    pub ns2_ip: Option<IpAddr>,
    pub soa_rname: String,
}

impl Config {
    pub fn from_env() -> Self {
        let udp_addr = env_addr(
            "DEKA_DNS_UDP_ADDR",
            "DEKA_DNS_UDP_PORT",
            "0.0.0.0",
            default_udp_port(),
        );
        let doh_addr = env_addr("DEKA_DNS_DOH_ADDR", "DEKA_DNS_DOH_PORT", "127.0.0.1", 8080);
        let zone = std::env::var("DEKA_DNS_ZONE").unwrap_or_else(|_| DEFAULT_ZONE.to_string());
        let ns1_host = std::env::var("NS1_HOST").unwrap_or_else(|_| format!("ns1.{zone}"));
        let ns2_host = std::env::var("NS2_HOST").unwrap_or_else(|_| format!("ns2.{zone}"));

        Self {
            udp_addr,
            doh_addr,
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
            zone,
            ns1_host,
            ns2_host,
            ns1_ip: std::env::var("NS1_IP").ok().and_then(|ip| ip.parse().ok()),
            ns2_ip: std::env::var("NS2_IP").ok().and_then(|ip| ip.parse().ok()),
            soa_rname: std::env::var("DEKA_DNS_SOA_RNAME")
                .unwrap_or_else(|_| "hostmaster.tana.gg".to_string()),
        }
    }
}

fn default_udp_port() -> u16 {
    match std::env::var("DEKA_DNS_MODE") {
        Ok(mode) if mode.eq_ignore_ascii_case("production") => PRODUCTION_UDP_PORT,
        _ => DEV_UDP_PORT,
    }
}

fn env_addr(addr_key: &str, port_key: &str, host: &str, port: u16) -> SocketAddr {
    if let Ok(addr) = std::env::var(addr_key)
        && let Ok(parsed) = addr.parse()
    {
        return parsed;
    }

    let port = std::env::var(port_key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(port);
    format!("{host}:{port}")
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], port)))
}

#[derive(Debug, Clone)]
pub struct StoreValue {
    pub value: String,
    pub ttl: Option<Duration>,
}

pub trait Store: Clone + Send + Sync + 'static {
    fn get_string(
        &self,
        key: String,
    ) -> impl Future<Output = Result<Option<StoreValue>, StoreError>> + Send;
}

#[derive(Debug, Clone)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

#[derive(Clone)]
pub struct RedisStore {
    client: redis::Client,
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
}

#[derive(Clone)]
struct CacheEntry {
    value: Option<String>,
    expires_at: Instant,
}

impl RedisStore {
    pub fn new(url: &str) -> Result<Self, redis::RedisError> {
        Ok(Self {
            client: redis::Client::open(url)?,
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}

impl Store for RedisStore {
    async fn get_string(&self, key: String) -> Result<Option<StoreValue>, StoreError> {
        if let Some(entry) = self
            .cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).cloned())
            && entry.expires_at > Instant::now()
        {
            return Ok(entry.value.map(|value| StoreValue { value, ttl: None }));
        }

        let client = self.client.clone();
        let lookup_key = key.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut conn = client.get_connection()?;
            let value: Option<String> = redis::cmd("GET").arg(&lookup_key).query(&mut conn)?;
            let ttl_secs: i64 = redis::cmd("TTL").arg(&lookup_key).query(&mut conn)?;
            Ok::<_, redis::RedisError>((value, ttl_secs))
        })
        .await
        .map_err(|err| StoreError(format!("redis task failed: {err}")))?
        .map_err(|err| StoreError(format!("redis read failed: {err}")))?;

        let ttl = result
            .1
            .try_into()
            .ok()
            .map(Duration::from_secs)
            .filter(|ttl| *ttl > Duration::ZERO);
        let cache_ttl = ttl
            .unwrap_or(NEGATIVE_CACHE_TTL)
            .min(Duration::from_secs(60));

        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(
                key,
                CacheEntry {
                    value: result.0.clone(),
                    expires_at: Instant::now() + cache_ttl,
                },
            );
        }

        Ok(result.0.map(|value| StoreValue { value, ttl }))
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    values: Arc<Mutex<HashMap<String, StoreValue>>>,
}

impl MemoryStore {
    pub fn insert(&self, key: impl Into<String>, value: impl Into<String>) {
        if let Ok(mut values) = self.values.lock() {
            values.insert(
                key.into(),
                StoreValue {
                    value: value.into(),
                    ttl: None,
                },
            );
        }
    }
}

impl Store for MemoryStore {
    async fn get_string(&self, key: String) -> Result<Option<StoreValue>, StoreError> {
        Ok(self
            .values
            .lock()
            .ok()
            .and_then(|values| values.get(&key).cloned()))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DnsRecord {
    #[serde(rename = "type")]
    record_type: String,
    name: String,
    value: String,
    #[serde(default = "default_ttl")]
    ttl: u32,
    #[serde(default)]
    priority: Option<u16>,
}

fn default_ttl() -> u32 {
    DEFAULT_TTL
}

#[derive(Clone)]
pub struct Resolver<S> {
    config: Config,
    store: S,
}

impl<S: Store> Resolver<S> {
    pub fn new(config: Config, store: S) -> Self {
        Self { config, store }
    }

    pub async fn resolve_bytes(&self, bytes: &[u8]) -> Result<Vec<u8>, ResolveError> {
        let query = Packet::parse(bytes).map_err(|err| ResolveError::BadQuery(err.to_string()))?;
        let response = self.resolve_packet(query).await?;
        response
            .build_bytes_vec_compressed()
            .map_err(|err| ResolveError::Encode(err.to_string()))
    }

    pub async fn resolve_packet<'a>(
        &self,
        query: Packet<'a>,
    ) -> Result<Packet<'static>, ResolveError> {
        let mut response: Packet<'static> = Packet::new_reply(query.id());
        response.set_flags(PacketFlag::AUTHORITATIVE_ANSWER);
        response.questions = query
            .questions
            .into_iter()
            .map(simple_dns::Question::into_owned)
            .collect();

        let questions = response.questions.clone();
        let mut found_any = false;
        for question in questions {
            let qname = normalize_name(&question.qname.to_string());
            let answers = self.resolve_question(&qname, question.qtype).await?;
            if !answers.is_empty() {
                found_any = true;
                response.answers.extend(answers);
            }
        }

        if !found_any {
            *response.rcode_mut() = RCODE::NameError;
        }

        response.additional_records.extend(self.glue_records());
        Ok(response)
    }

    async fn resolve_question(
        &self,
        qname: &str,
        qtype: QTYPE,
    ) -> Result<Vec<ResourceRecord<'static>>, ResolveError> {
        if qname == self.config.zone {
            return Ok(self.apex_records(qtype));
        }

        if let Some(domain) = qname.strip_prefix("_tana-verify.") {
            if matches_qtype(qtype, TYPE::TXT)
                && let Some(token) = self.read_key(format!("verify:{domain}")).await?
            {
                return txt_record(qname, token.value, DEFAULT_TTL).map(|rr| vec![rr]);
            }
            return Ok(Vec::new());
        }

        let domain = self.domain_for_name(qname).await?;
        let Some(domain) = domain else {
            return Ok(Vec::new());
        };

        let Some(records_json) = self.read_key(format!("domain:{domain}:records")).await? else {
            return Ok(Vec::new());
        };

        let records: Vec<DnsRecord> = serde_json::from_str(&records_json.value)
            .map_err(|err| ResolveError::Store(format!("invalid records for {domain}: {err}")))?;

        records
            .into_iter()
            .filter(|record| record_matches_name(record, &domain, qname))
            .filter(|record| record_matches_type(record, qtype))
            .map(|record| record_to_rr(record, &domain))
            .collect()
    }

    async fn domain_for_name(&self, qname: &str) -> Result<Option<String>, ResolveError> {
        let labels: Vec<&str> = qname.split('.').collect();
        for index in 0..labels.len().saturating_sub(1) {
            let candidate = labels[index..].join(".");
            if self
                .read_key(format!("domain:{candidate}:records"))
                .await?
                .is_some()
            {
                return Ok(Some(candidate));
            }
        }

        if let Some(sub) = qname.strip_suffix(&format!(".{}", self.config.zone))
            && !sub.contains('.')
            && self.read_key(format!("subdomain:{sub}")).await?.is_some()
        {
            return Ok(Some(qname.to_string()));
        }

        Ok(None)
    }

    async fn read_key(&self, key: String) -> Result<Option<StoreValue>, ResolveError> {
        self.store
            .get_string(key)
            .await
            .map_err(|err| ResolveError::Store(err.to_string()))
    }

    fn apex_records(&self, qtype: QTYPE) -> Vec<ResourceRecord<'static>> {
        let mut records = Vec::new();
        if matches_qtype(qtype, TYPE::NS) {
            if let Ok(rr) = ns_record(&self.config.zone, &self.config.ns1_host) {
                records.push(rr);
            }
            if let Ok(rr) = ns_record(&self.config.zone, &self.config.ns2_host) {
                records.push(rr);
            }
        }
        if (matches_qtype(qtype, TYPE::SOA) || matches!(qtype, QTYPE::ANY))
            && let Ok(rr) = soa_record(&self.config)
        {
            records.push(rr);
        }
        records
    }

    fn glue_records(&self) -> Vec<ResourceRecord<'static>> {
        [
            (&self.config.ns1_host, self.config.ns1_ip),
            (&self.config.ns2_host, self.config.ns2_ip),
        ]
        .into_iter()
        .filter_map(|(host, ip)| match ip {
            Some(IpAddr::V4(ip)) => a_record(host, ip, DEFAULT_TTL).ok(),
            Some(IpAddr::V6(ip)) => aaaa_record(host, ip, DEFAULT_TTL).ok(),
            None => None,
        })
        .collect()
    }
}

fn normalize_name(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

fn matches_qtype(qtype: QTYPE, record_type: TYPE) -> bool {
    matches!(qtype, QTYPE::ANY) || matches!(qtype, QTYPE::TYPE(ty) if ty == record_type)
}

fn record_matches_type(record: &DnsRecord, qtype: QTYPE) -> bool {
    let record_type = match record.record_type.to_ascii_uppercase().as_str() {
        "A" => TYPE::A,
        "AAAA" => TYPE::AAAA,
        "NS" => TYPE::NS,
        "MX" => TYPE::MX,
        "TXT" => TYPE::TXT,
        _ => return false,
    };
    matches_qtype(qtype, record_type)
}

fn record_matches_name(record: &DnsRecord, domain: &str, qname: &str) -> bool {
    record_owner(&record.name, domain)
        .map(|owner| owner == qname)
        .unwrap_or(false)
}

fn record_owner(name: &str, domain: &str) -> Option<String> {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name == "@" {
        Some(domain.to_string())
    } else if name.ends_with(domain) {
        Some(name)
    } else if valid_relative_name(&name) {
        Some(format!("{name}.{domain}"))
    } else {
        None
    }
}

fn valid_relative_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(' ')
}

fn record_to_rr(record: DnsRecord, domain: &str) -> Result<ResourceRecord<'static>, ResolveError> {
    let owner = record_owner(&record.name, domain)
        .ok_or_else(|| ResolveError::Store(format!("invalid owner name {}", record.name)))?;
    match record.record_type.to_ascii_uppercase().as_str() {
        "A" => {
            let ip = record.value.parse::<Ipv4Addr>().map_err(|err| {
                ResolveError::Store(format!("invalid A record value {}: {err}", record.value))
            })?;
            a_record(&owner, ip, record.ttl)
        }
        "AAAA" => {
            let ip = record.value.parse::<Ipv6Addr>().map_err(|err| {
                ResolveError::Store(format!("invalid AAAA record value {}: {err}", record.value))
            })?;
            aaaa_record(&owner, ip, record.ttl)
        }
        "NS" => ns_record(&owner, &record.value).map(|mut rr| {
            rr.ttl = record.ttl;
            rr
        }),
        "MX" => mx_record(
            &owner,
            &record.value,
            record.priority.unwrap_or(10),
            record.ttl,
        ),
        "TXT" => txt_record(&owner, record.value, record.ttl),
        _ => Ok(ResourceRecord::new(
            Name::new_unchecked("").into_owned(),
            CLASS::IN,
            0,
            RData::Empty(TYPE::A),
        )),
    }
}

fn a_record(owner: &str, ip: Ipv4Addr, ttl: u32) -> Result<ResourceRecord<'static>, ResolveError> {
    Ok(ResourceRecord::new(
        Name::new(owner).map_err(name_err)?.into_owned(),
        CLASS::IN,
        ttl,
        RData::A(A::from(ip)),
    ))
}

fn aaaa_record(
    owner: &str,
    ip: Ipv6Addr,
    ttl: u32,
) -> Result<ResourceRecord<'static>, ResolveError> {
    Ok(ResourceRecord::new(
        Name::new(owner).map_err(name_err)?.into_owned(),
        CLASS::IN,
        ttl,
        RData::AAAA(AAAA::from(ip)),
    ))
}

fn ns_record(owner: &str, host: &str) -> Result<ResourceRecord<'static>, ResolveError> {
    Ok(ResourceRecord::new(
        Name::new(owner).map_err(name_err)?.into_owned(),
        CLASS::IN,
        DEFAULT_TTL,
        RData::NS(NS(Name::new(host).map_err(name_err)?.into_owned())),
    ))
}

fn mx_record(
    owner: &str,
    exchange: &str,
    priority: u16,
    ttl: u32,
) -> Result<ResourceRecord<'static>, ResolveError> {
    Ok(ResourceRecord::new(
        Name::new(owner).map_err(name_err)?.into_owned(),
        CLASS::IN,
        ttl,
        RData::MX(MX {
            preference: priority,
            exchange: Name::new(exchange).map_err(name_err)?.into_owned(),
        }),
    ))
}

fn txt_record(
    owner: &str,
    value: String,
    ttl: u32,
) -> Result<ResourceRecord<'static>, ResolveError> {
    Ok(ResourceRecord::new(
        Name::new(owner).map_err(name_err)?.into_owned(),
        CLASS::IN,
        ttl,
        RData::TXT(
            TXT::try_from(value.as_str())
                .map_err(name_err)?
                .into_owned(),
        ),
    ))
}

fn soa_record(config: &Config) -> Result<ResourceRecord<'static>, ResolveError> {
    Ok(ResourceRecord::new(
        Name::new(&config.zone).map_err(name_err)?.into_owned(),
        CLASS::IN,
        DEFAULT_TTL,
        RData::SOA(SOA {
            mname: Name::new(&config.ns1_host).map_err(name_err)?.into_owned(),
            rname: Name::new(&config.soa_rname).map_err(name_err)?.into_owned(),
            serial: 1,
            refresh: 3600,
            retry: 600,
            expire: 604800,
            minimum: 60,
        }),
    ))
}

fn name_err(err: simple_dns::SimpleDnsError) -> ResolveError {
    ResolveError::Encode(err.to_string())
}

#[derive(Debug)]
pub enum ResolveError {
    BadQuery(String),
    Store(String),
    Encode(String),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::BadQuery(err) => write!(f, "bad DNS query: {err}"),
            ResolveError::Store(err) => write!(f, "store error: {err}"),
            ResolveError::Encode(err) => write!(f, "DNS encode error: {err}"),
        }
    }
}

impl std::error::Error for ResolveError {}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_dns::{Question, rdata::RData};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn with_dns_env<R>(values: &[(&str, Option<&str>)], run: impl FnOnce() -> R) -> R {
        let _guard = env_lock();
        let keys = ["DEKA_DNS_MODE", "DEKA_DNS_UDP_ADDR", "DEKA_DNS_UDP_PORT"];
        let saved = keys.map(|key| (key, std::env::var_os(key)));

        for key in keys {
            unsafe { std::env::remove_var(key) };
        }
        for (key, value) in values {
            if let Some(value) = value {
                unsafe { std::env::set_var(key, value) };
            }
        }

        let result = run();

        for (key, value) in saved {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }

        result
    }

    fn test_config() -> Config {
        Config {
            udp_addr: SocketAddr::from(([127, 0, 0, 1], 8053)),
            doh_addr: SocketAddr::from(([127, 0, 0, 1], 8080)),
            redis_url: "redis://127.0.0.1:6379".to_string(),
            zone: "tana.gg".to_string(),
            ns1_host: "ns1.tana.gg".to_string(),
            ns2_host: "ns2.tana.gg".to_string(),
            ns1_ip: Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))),
            ns2_ip: Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))),
            soa_rname: "hostmaster.tana.gg".to_string(),
        }
    }

    fn query(name: &str, ty: TYPE) -> Packet<'static> {
        let mut packet = Packet::new_query(42);
        packet.questions.push(Question::new(
            Name::new(name).unwrap().into_owned(),
            ty.into(),
            CLASS::IN.into(),
            false,
        ));
        packet
    }

    #[test]
    fn config_defaults_udp_to_dev_port_without_mode() {
        let config = with_dns_env(&[], Config::from_env);

        assert_eq!(8053, config.udp_addr.port());
    }

    #[test]
    fn config_defaults_udp_to_port_53_in_production_mode() {
        let config = with_dns_env(&[("DEKA_DNS_MODE", Some("production"))], Config::from_env);

        assert_eq!(53, config.udp_addr.port());
    }

    #[test]
    fn explicit_udp_port_overrides_production_default() {
        let config = with_dns_env(
            &[
                ("DEKA_DNS_MODE", Some("production")),
                ("DEKA_DNS_UDP_PORT", Some("8054")),
            ],
            Config::from_env,
        );

        assert_eq!(8054, config.udp_addr.port());
    }

    async fn resolve(store: MemoryStore, name: &str, ty: TYPE) -> Packet<'static> {
        Resolver::new(test_config(), store)
            .resolve_packet(query(name, ty))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_lookup_hit() {
        let store = MemoryStore::default();
        store.insert("subdomain:test", "shop_test");
        store.insert(
            "domain:test.tana.gg:records",
            r#"[{"type":"A","name":"@","value":"1.2.3.4","ttl":300}]"#,
        );

        let response = resolve(store, "test.tana.gg", TYPE::A).await;

        assert_eq!(RCODE::NoError, response.rcode());
        assert_eq!(1, response.answers.len());
        match &response.answers[0].rdata {
            RData::A(a) => assert_eq!(u32::from(Ipv4Addr::new(1, 2, 3, 4)), a.address),
            other => panic!("expected A record, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn aaaa_lookup_hit() {
        let store = MemoryStore::default();
        store.insert(
            "domain:ipv6.example.com:records",
            r#"[{"type":"AAAA","name":"@","value":"2001:db8::1","ttl":300}]"#,
        );

        let response = resolve(store, "ipv6.example.com", TYPE::AAAA).await;

        assert_eq!(RCODE::NoError, response.rcode());
        match &response.answers[0].rdata {
            RData::AAAA(aaaa) => assert_eq!(
                u128::from(Ipv6Addr::from(0x20010db8000000000000000000000001_u128)),
                aaaa.address
            ),
            other => panic!("expected AAAA record, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ns_apex() {
        let response = resolve(MemoryStore::default(), "tana.gg", TYPE::NS).await;

        assert_eq!(RCODE::NoError, response.rcode());
        assert_eq!(2, response.answers.len());
        assert!(
            response
                .answers
                .iter()
                .any(|rr| matches!(&rr.rdata, RData::NS(ns) if ns.to_string() == "ns1.tana.gg"))
        );
    }

    #[tokio::test]
    async fn nxdomain_miss() {
        let response = resolve(MemoryStore::default(), "missing.tana.gg", TYPE::A).await;

        assert_eq!(RCODE::NameError, response.rcode());
        assert!(response.answers.is_empty());
    }

    #[tokio::test]
    async fn mx_with_priority() {
        let store = MemoryStore::default();
        store.insert(
            "domain:example.com:records",
            r#"[{"type":"MX","name":"@","value":"mail.example.com","ttl":300,"priority":5}]"#,
        );

        let response = resolve(store, "example.com", TYPE::MX).await;

        assert_eq!(RCODE::NoError, response.rcode());
        match &response.answers[0].rdata {
            RData::MX(mx) => {
                assert_eq!(5, mx.preference);
                assert_eq!("mail.example.com", mx.exchange.to_string());
            }
            other => panic!("expected MX record, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn txt_verification_token() {
        let store = MemoryStore::default();
        store.insert("verify:example.com", "tana-verify=abc123");

        let response = resolve(store, "_tana-verify.example.com", TYPE::TXT).await;

        assert_eq!(RCODE::NoError, response.rcode());
        match response.answers[0].rdata.clone() {
            RData::TXT(txt) => {
                let value = String::try_from(txt).unwrap();
                assert_eq!("tana-verify=abc123", value);
            }
            other => panic!("expected TXT record, got {other:?}"),
        }
    }
}
