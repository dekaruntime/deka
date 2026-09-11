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
    /// RFD 27 host grant table for DekaScript-from-disk modules. `None` lets
    /// the ESM loader use the project-installed `deka.grants.json` written by
    /// `deka add` / `deka install` (deka#797).
    pub host_grants: Option<runtime_core::host_bridge::GrantTable>,
    /// Arguments exposed to a handler. The command dispatcher supplies these;
    /// worker threads never consult process-global state.
    pub deka_args: serde_json::Value,
    /// Explicit execution posture selected by the dispatcher/configuration.
    pub use_esm: bool,
    pub debug: bool,
    pub perf_profile: bool,
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
            deka_args: serde_json::json!([]),
            use_esm: true,
            debug: false,
            perf_profile: false,
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
