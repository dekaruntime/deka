// ========== Configuration ==========

/// Configuration for the isolate pool
#[derive(Clone)]
pub struct PoolConfig {
    /// Number of worker threads (default: num_cpus)
    pub num_workers: usize,
    /// Max isolates per worker (0 = unlimited)
    pub max_isolates_per_worker: usize,
    /// Idle timeout before evicting an isolate (seconds, 0 = never)
    pub idle_timeout_secs: u64,
    /// Enable detailed timing logs
    pub enable_metrics: bool,
    /// Enable V8 code cache for handler compilation
    pub enable_code_cache: bool,
    /// Request execution timeout in milliseconds (0 = no timeout)
    pub request_timeout_ms: u64,
    /// Max time a request can sit in the queue in milliseconds (0 = no timeout)
    pub queue_timeout_ms: u64,
    /// Scheduler strategy for routing requests to workers
    pub scheduler_strategy: SchedulerStrategy,
    /// Enable per-request profiling data (op timings)
    pub introspect_profiling: bool,
    /// RFD 27 host grant table for DekaScript-from-disk modules. `None` falls
    /// back to the `DEKA_HOST_GRANTS` env var in the ESM loader until the
    /// registry/index plumbing lands.
    pub host_grants: Option<runtime_core::host_bridge::GrantTable>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        let default_workers = default_num_workers();
        Self {
            num_workers: default_workers,
            max_isolates_per_worker: 100, // Reasonable default for deka
            idle_timeout_secs: 300,       // 5 minutes
            enable_metrics: true,
            enable_code_cache: true,
            request_timeout_ms: 30_000,
            queue_timeout_ms: 10_000,
            scheduler_strategy: SchedulerStrategy::LeastLoaded,
            introspect_profiling: false,
            host_grants: None,
        }
    }
}

impl PoolConfig {
    /// Create config from environment variables
    ///
    /// Environment variables:
    /// - ISOLATE_WORKERS: Number of worker threads (default: num_cpus)
    /// - ISOLATES_PER_WORKER: Max isolates per worker (0 = unlimited)
    /// - ISOLATE_IDLE_TIMEOUT: Idle timeout in seconds (0 = never evict)
    /// - ISOLATE_METRICS: Enable metrics (default: true)
    /// - ISOLATE_CODE_CACHE: Enable V8 code cache (default: true)
    /// - ISOLATE_REQUEST_TIMEOUT_MS: Request timeout in ms (0 = no timeout)
    /// - ISOLATE_QUEUE_TIMEOUT_MS: Queue timeout in ms (0 = no timeout)
    /// - ISOLATE_SCHEDULER: "consistent" or "least_loaded"
    pub fn from_env() -> Self {
        let default_workers = default_num_workers();
        Self {
            num_workers: std::env::var("ISOLATE_WORKERS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(default_workers),
            max_isolates_per_worker: std::env::var("ISOLATES_PER_WORKER")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(100),
            idle_timeout_secs: std::env::var("ISOLATE_IDLE_TIMEOUT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300),
            enable_metrics: std::env::var("ISOLATE_METRICS")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
            enable_code_cache: std::env::var("ISOLATE_CODE_CACHE")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
            request_timeout_ms: std::env::var("ISOLATE_REQUEST_TIMEOUT_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30_000),
            queue_timeout_ms: std::env::var("ISOLATE_QUEUE_TIMEOUT_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10_000),
            scheduler_strategy: std::env::var("ISOLATE_SCHEDULER")
                .ok()
                .and_then(|value| SchedulerStrategy::from_env(&value))
                .unwrap_or(SchedulerStrategy::LeastLoaded),
            introspect_profiling: std::env::var("INTROSPECT_PROFILING")
                .map(|value| value != "false" && value != "0")
                .unwrap_or(false),
            // Grant tables are not env-scalar config; the ESM loader already
            // falls back to DEKA_HOST_GRANTS when this is None.
            host_grants: None,
        }
    }
}

fn default_num_workers() -> usize {
    num_cpus::get().max(1)
}

/// Scheduler strategy for routing requests to workers
#[derive(Debug, Clone, Copy)]
pub enum SchedulerStrategy {
    ConsistentHash,
    LeastLoaded,
}

impl SchedulerStrategy {
    fn from_env(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "consistent" | "hash" => Some(Self::ConsistentHash),
            "least_loaded" | "least" => Some(Self::LeastLoaded),
            _ => None,
        }
    }
}
