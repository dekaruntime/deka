use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

pub type SecretsMap = HashMap<String, String>;

pub trait ShopSecretsSource: Send + Sync {
    fn fetch_shop_secrets<'a>(
        &'a self,
        shop_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SecretsMap, String>> + Send + 'a>>;
}

/// Placeholder secrets source used while the harar vault system is phased out.
/// Returns an empty map for every shop. A replacement backend (environment
/// variables, cloud secret manager, etc.) can be plugged in here later without
/// changing the cache API.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyShopSecrets;

impl ShopSecretsSource for EmptyShopSecrets {
    fn fetch_shop_secrets<'a>(
        &'a self,
        _shop_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SecretsMap, String>> + Send + 'a>> {
        Box::pin(async move { Ok(HashMap::new()) })
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
            Arc::new(EmptyShopSecrets::default()),
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
    async fn empty_source_returns_no_secrets() {
        let cache = SecretsCache::from_env();
        let secrets = cache.get_secrets_for_shop("shop_a").await.unwrap();
        assert!(secrets.is_empty());
    }
}
