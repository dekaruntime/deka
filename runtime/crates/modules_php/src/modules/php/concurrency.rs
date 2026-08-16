use super::*;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use tokio::sync::{oneshot, Mutex};
use tokio::time::{timeout, Duration};

/// Runtime-scoped named lock.
///
/// Isolates do not share memory, so this is not a thread mutex. It is a
/// runtime-backed queue used to serialize access to external shared resources
/// (a DB row, a KV key, an external API rate limit, etc.).

#[derive(Debug)]
struct LockState {
    holder: Option<u64>,
    waiters: VecDeque<oneshot::Sender<u64>>,
}

#[derive(Debug)]
struct LockRegistry {
    locks: HashMap<String, LockState>,
    token_to_name: HashMap<u64, String>,
    next_token: AtomicU64,
}

impl LockRegistry {
    fn new() -> Self {
        Self {
            locks: HashMap::new(),
            token_to_name: HashMap::new(),
            next_token: AtomicU64::new(1),
        }
    }

    fn alloc_token(&self) -> u64 {
        self.next_token.fetch_add(1, Ordering::Relaxed)
    }
}

static LOCK_REGISTRY: OnceLock<Mutex<LockRegistry>> = OnceLock::new();

fn lock_registry() -> &'static Mutex<LockRegistry> {
    LOCK_REGISTRY.get_or_init(|| Mutex::new(LockRegistry::new()))
}

fn lock_ok(value: serde_json::Value) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    out.insert("ok".to_string(), serde_json::Value::Bool(true));
    if let serde_json::Value::Object(map) = value {
        for (k, v) in map {
            out.insert(k, v);
        }
    }
    serde_json::Value::Object(out)
}

fn lock_err(error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "error": error.into(),
    })
}

async fn lock_acquire_impl(name: String, timeout_ms: u64) -> serde_json::Value {
    let deadline = Duration::from_millis(timeout_ms.max(1));
    let registry = lock_registry();
    let mut remaining = deadline;

    loop {
        let maybe_rx = {
            let mut reg = registry.lock().await;
            let token = reg.alloc_token();
            let state = reg.locks.entry(name.clone()).or_insert_with(|| LockState {
                holder: None,
                waiters: VecDeque::new(),
            });

            if state.holder.is_none() {
                state.holder = Some(token);
                reg.token_to_name.insert(token, name.clone());
                return lock_ok(serde_json::json!({ "token": token }));
            }

            let (tx, rx) = oneshot::channel::<u64>();
            state.waiters.push_back(tx);
            rx
        };

        let wait_start = tokio::time::Instant::now();
        match timeout(remaining, maybe_rx).await {
            Ok(Ok(acquired_token)) => {
                // We were granted the lock by the previous holder.
                let mut reg = registry.lock().await;
                reg.token_to_name.insert(acquired_token, name.clone());
                return lock_ok(serde_json::json!({ "token": acquired_token }));
            }
            Ok(Err(_)) => {
                // Sender dropped without granting; this should not happen,
                // but treat it as a cancellation and retry.
            }
            Err(_) => {
                // Timeout. Remove our waiter if it is still queued.
                let mut reg = registry.lock().await;
                if let Some(state) = reg.locks.get_mut(&name) {
                    state.waiters.retain(|tx| !tx.is_closed());
                    if state.waiters.is_empty() && state.holder.is_none() {
                        reg.locks.remove(&name);
                    }
                }
                return lock_err("lock_acquire_timeout");
            }
        }

        let elapsed = wait_start.elapsed();
        if elapsed >= remaining {
            return lock_err("lock_acquire_timeout");
        }
        remaining -= elapsed;

        // The cancelled/closed waiter was already removed above, so retry.
    }
}

async fn lock_release_impl(token: u64) -> serde_json::Value {
    let registry = lock_registry();
    let mut reg = registry.lock().await;

    let Some(name) = reg.token_to_name.remove(&token) else {
        return lock_err("lock_token_not_found");
    };

    let Some(state) = reg.locks.get_mut(&name) else {
        return lock_err("lock_state_missing");
    };

    if state.holder != Some(token) {
        return lock_err("lock_token_not_holder");
    }

    // Pass ownership to the next waiter, or free the lock.
    let mut granted = false;
    while let Some(tx) = state.waiters.pop_front() {
        if tx.is_closed() {
            continue;
        }
        // Reuse this token for the next holder to keep the registry simple.
        state.holder = Some(token);
        let _ = tx.send(token);
        granted = true;
        break;
    }

    if !granted {
        state.holder = None;
        if state.waiters.is_empty() {
            reg.locks.remove(&name);
        }
    }

    lock_ok(serde_json::Value::Object(serde_json::Map::new()))
}

#[op2]
#[serde]
pub(super) async fn op_php_concurrency_lock_acquire(
    #[string] name: String,
    #[number] timeout_ms: u64,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    Ok(lock_acquire_impl(name, timeout_ms).await)
}

#[op2]
#[serde]
pub(super) async fn op_php_concurrency_lock_release(
    #[number] token: u64,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    Ok(lock_release_impl(token).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_name(prefix: &str) -> String {
        format!(
            "{}_{}_{}",
            prefix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
    }

    #[tokio::test]
    async fn lock_acquire_grants_token_when_free() {
        let name = unique_name("free");
        let res = lock_acquire_impl(name.clone(), 1000).await;
        assert_eq!(res.get("ok").and_then(|v| v.as_bool()), Some(true));
        let token = res.get("token").and_then(|v| v.as_u64()).unwrap();
        let _ = lock_release_impl(token).await;
    }

    #[tokio::test]
    async fn lock_second_acquirer_waits_until_release() {
        let name = unique_name("queue");
        let first = lock_acquire_impl(name.clone(), 1000).await;
        let token1 = first.get("token").and_then(|v| v.as_u64()).unwrap();

        let name2 = name.clone();
        let pending = tokio::spawn(async move {
            lock_acquire_impl(name2, 2000).await
        });

        // Give the pending task time to enter the queue.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let release = lock_release_impl(token1).await;
        assert_eq!(release.get("ok").and_then(|v| v.as_bool()), Some(true));

        let second = pending.await.expect("spawn join");
        assert_eq!(second.get("ok").and_then(|v| v.as_bool()), Some(true));
        let token2 = second.get("token").and_then(|v| v.as_u64()).unwrap();
        // The same runtime token is reused when passing ownership to the next waiter.
        assert_eq!(token1, token2);

        // Clean up.
        let _ = lock_release_impl(token2).await;
    }

    #[tokio::test]
    async fn lock_acquire_times_out_when_held() {
        let name = unique_name("timeout");
        let first = lock_acquire_impl(name.clone(), 1000).await;
        let token1 = first.get("token").and_then(|v| v.as_u64()).unwrap();

        let second = lock_acquire_impl(name.clone(), 50).await;
        assert_eq!(second.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            second.get("error").and_then(|v| v.as_str()),
            Some("lock_acquire_timeout")
        );

        let _ = lock_release_impl(token1).await;
    }

    #[tokio::test]
    async fn lock_release_rejects_unknown_token() {
        let res = lock_release_impl(u64::MAX).await;
        assert_eq!(res.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            res.get("error").and_then(|v| v.as_str()),
            Some("lock_token_not_found")
        );
    }

    #[tokio::test]
    async fn lock_release_rejects_double_release() {
        let name = unique_name("double_release");

        let first = lock_acquire_impl(name, 1000).await;
        let token = first.get("token").and_then(|v| v.as_u64()).unwrap();

        let release1 = lock_release_impl(token).await;
        assert_eq!(release1.get("ok").and_then(|v| v.as_bool()), Some(true));

        // Releasing the same token again should fail because the token is no
        // longer registered once the lock has been released.
        let release2 = lock_release_impl(token).await;
        assert_eq!(release2.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            release2.get("error").and_then(|v| v.as_str()),
            Some("lock_token_not_found")
        );
    }
}
