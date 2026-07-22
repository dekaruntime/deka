use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use harar_client::VaultClient;
use tokio::sync::Mutex;

pub type SecretsMap = HashMap<String, String>;

pub trait ShopSecretsSource: Send + Sync {
    fn fetch_shop_secrets<'a>(
        &'a self,
        shop_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SecretsMap, String>> + Send + 'a>>;
}

#[derive(Clone)]
pub struct HararShopSecrets {
    client: VaultClient,
}

impl HararShopSecrets {
    pub fn from_env() -> Self {
        Self {
            client: VaultClient::from_socket(),
        }
    }

    pub fn from_client(client: VaultClient) -> Self {
        Self { client }
    }
}

impl ShopSecretsSource for HararShopSecrets {
    fn fetch_shop_secrets<'a>(
        &'a self,
        shop_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SecretsMap, String>> + Send + 'a>> {
        Box::pin(async move {
            if shop_id.is_empty() {
                return Ok(HashMap::new());
            }

            let prefix = format!("shops/{shop_id}/");
            let keys = self
                .client
                .list_for_shop(shop_id)
                .await
                .map_err(|err| err.to_string())?;
            let mut out = HashMap::new();
            for key in keys {
                let Some(name) = key.strip_prefix(&prefix) else {
                    continue;
                };
                if name.is_empty() || name.contains('/') {
                    continue;
                }
                let value = self
                    .client
                    .get_for_shop(&key, shop_id)
                    .await
                    .map_err(|err| err.to_string())?;
                out.insert(name.to_string(), value);
            }
            Ok(out)
        })
    }
}

struct CacheEntry {
    loaded_at: Instant,
    secrets: SecretsMap,
}

#[derive(Clone)]
pub struct SecretsCache {
    ttl: Duration,
    source: Arc<dyn ShopSecretsSource>,
    entries: Arc<Mutex<HashMap<String, CacheEntry>>>,
}

impl SecretsCache {
    pub fn from_env() -> Self {
        Self::new(
            Duration::from_secs(300),
            Arc::new(HararShopSecrets::from_env()),
        )
    }

    pub fn new(ttl: Duration, source: Arc<dyn ShopSecretsSource>) -> Self {
        Self {
            ttl,
            source,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_secrets_for_shop(&self, shop_id: &str) -> Result<SecretsMap, String> {
        if shop_id.is_empty() {
            return Ok(HashMap::new());
        }

        let now = Instant::now();
        {
            let entries = self.entries.lock().await;
            if let Some(entry) = entries.get(shop_id) {
                if now.duration_since(entry.loaded_at) < self.ttl {
                    return Ok(entry.secrets.clone());
                }
            }
        }

        let secrets = self.source.fetch_shop_secrets(shop_id).await?;
        self.entries.lock().await.insert(
            shop_id.to_string(),
            CacheEntry {
                loaded_at: now,
                secrets: secrets.clone(),
            },
        );
        Ok(secrets)
    }

    pub async fn invalidate_shop(&self, shop_id: &str) {
        self.entries.lock().await.remove(shop_id);
    }

    pub async fn clear(&self) {
        self.entries.lock().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::time::sleep;

    struct MockSource {
        calls: AtomicUsize,
    }

    impl MockSource {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl ShopSecretsSource for MockSource {
        fn fetch_shop_secrets<'a>(
            &'a self,
            shop_id: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<SecretsMap, String>> + Send + 'a>> {
            Box::pin(async move {
                let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
                Ok(HashMap::from([(
                    "STRIPE_SECRET_KEY".to_string(),
                    format!("{shop_id}-secret-{call}"),
                )]))
            })
        }
    }

    #[tokio::test]
    async fn cache_hit_reuses_shop_snapshot_until_ttl_expires() {
        let source = Arc::new(MockSource::new());
        let cache = SecretsCache::new(Duration::from_millis(50), source.clone());

        let first = cache.get_secrets_for_shop("shop_a").await.unwrap();
        let second = cache.get_secrets_for_shop("shop_a").await.unwrap();

        assert_eq!(first, second);
        assert_eq!(source.calls(), 1);

        sleep(Duration::from_millis(60)).await;
        let third = cache.get_secrets_for_shop("shop_a").await.unwrap();

        assert_eq!(third.get("STRIPE_SECRET_KEY").unwrap(), "shop_a-secret-2");
        assert_eq!(source.calls(), 2);
    }

    #[tokio::test]
    async fn shops_have_independent_cache_entries() {
        let source = Arc::new(MockSource::new());
        let cache = SecretsCache::new(Duration::from_secs(60), source.clone());

        let shop_a = cache.get_secrets_for_shop("shop_a").await.unwrap();
        let shop_b = cache.get_secrets_for_shop("shop_b").await.unwrap();

        assert_eq!(shop_a.get("STRIPE_SECRET_KEY").unwrap(), "shop_a-secret-1");
        assert_eq!(shop_b.get("STRIPE_SECRET_KEY").unwrap(), "shop_b-secret-2");
        assert_eq!(source.calls(), 2);
    }

    #[tokio::test]
    async fn invalidation_forces_refetch() {
        let source = Arc::new(MockSource::new());
        let cache = SecretsCache::new(Duration::from_secs(60), source.clone());

        let first = cache.get_secrets_for_shop("shop_a").await.unwrap();
        cache.invalidate_shop("shop_a").await;
        let second = cache.get_secrets_for_shop("shop_a").await.unwrap();

        assert_ne!(first, second);
        assert_eq!(source.calls(), 2);
    }

    #[tokio::test]
    async fn harar_source_filters_to_requested_shop_and_key_names() {
        let server = MockVault::start(&[
            ("shops/shop_a/STRIPE_SECRET_KEY", "sk-a"),
            ("shops/shop_a/WEBHOOK_SECRET", "wh-a"),
            ("shops/shop_a/nested/IGNORED", "nested"),
            ("shops/shop_b/STRIPE_SECRET_KEY", "sk-b"),
        ])
        .await;
        let source =
            HararShopSecrets::from_client(VaultClient::from_socket_path(&server.socket_path));

        let secrets = source.fetch_shop_secrets("shop_a").await.unwrap();

        assert_eq!(secrets.len(), 2);
        assert_eq!(secrets.get("STRIPE_SECRET_KEY").unwrap(), "sk-a");
        assert_eq!(secrets.get("WEBHOOK_SECRET").unwrap(), "wh-a");
        assert!(!secrets.contains_key("nested/IGNORED"));
        assert!(!secrets.values().any(|value| value == "sk-b"));
    }

    struct MockVault {
        socket_path: std::path::PathBuf,
        _temp: TempDir,
    }

    impl MockVault {
        async fn start(secrets: &[(&str, &str)]) -> Self {
            let temp = TempDir::new().unwrap();
            let socket_path = temp.path().join("vault.sock");
            let listener = UnixListener::bind(&socket_path).unwrap();
            let secrets = Arc::new(
                secrets
                    .iter()
                    .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                    .collect::<HashMap<_, _>>(),
            );

            tokio::spawn({
                let secrets = Arc::clone(&secrets);
                async move {
                    loop {
                        let Ok((mut stream, _)) = listener.accept().await else {
                            return;
                        };
                        let secrets = Arc::clone(&secrets);
                        tokio::spawn(async move {
                            let mut request = Vec::new();
                            let _ = stream.read_to_end(&mut request).await;
                            let parsed: serde_json::Value =
                                serde_json::from_slice(&request).unwrap();
                            let requested_shop_id =
                                parsed.get("shop_id").and_then(|shop_id| shop_id.as_str());
                            let response = match parsed.get("op").and_then(|op| op.as_str()) {
                                Some("list") => {
                                    if requested_shop_id != Some("shop_a") {
                                        return write_mock_response(
                                            stream,
                                            serde_json::json!({
                                                "ok": false,
                                                "error": "missing_shop_scope"
                                            }),
                                        )
                                        .await;
                                    }
                                    let mut keys = secrets.keys().cloned().collect::<Vec<_>>();
                                    keys.sort();
                                    serde_json::json!({ "ok": true, "keys": keys })
                                }
                                Some("get") => {
                                    let key = parsed
                                        .get("key")
                                        .and_then(|key| key.as_str())
                                        .unwrap_or_default();
                                    if requested_shop_id != Some("shop_a")
                                        || !key.starts_with("shops/shop_a/")
                                    {
                                        return write_mock_response(
                                            stream,
                                            serde_json::json!({
                                                "ok": false,
                                                "error": "missing_shop_scope"
                                            }),
                                        )
                                        .await;
                                    }
                                    match secrets.get(key) {
                                        Some(value) => {
                                            serde_json::json!({ "ok": true, "value": value })
                                        }
                                        None => {
                                            serde_json::json!({ "ok": false, "error": "not_found" })
                                        }
                                    }
                                }
                                _ => serde_json::json!({ "ok": false, "error": "bad_op" }),
                            };
                            write_mock_response(stream, response).await;
                        });
                    }
                }
            });

            Self {
                socket_path,
                _temp: temp,
            }
        }
    }

    async fn write_mock_response(mut stream: UnixStream, response: serde_json::Value) {
        let _ = stream.write_all(response.to_string().as_bytes()).await;
        let _ = stream.shutdown().await;
    }
}
