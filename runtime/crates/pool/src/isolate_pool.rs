//! Warm Isolate Pool for Deka Runtime
//!
//! This module implements Cloudflare-style warm isolate pooling to reduce cold start
//! latency. Instead of creating a fresh JsRuntime for every request (~200ms), we keep
//! isolates "warm" with code pre-compiled, reducing subsequent request latency to ~5ms.
//!
//! Architecture:
//! - N worker threads, each owning many JsRuntime instances locally
//! - Consistent hashing routes handlers to specific workers
//! - LRU eviction when worker reaches max isolate capacity
//! - Thread-local design because JsRuntime is !Send

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use deno_core::{
    Extension, JsRuntime, ModuleCodeString, ModuleSpecifier, OpMetricsEvent, OpMetricsFactoryFn,
    OpMetricsFn, RuntimeOptions, serde_v8,
};
use nanoid::nanoid;

const ID_ALPHABET: [char; 62] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I',
    'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b',
    'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u',
    'v', 'w', 'x', 'y', 'z',
];
use deno_core::v8;
use tokio::sync::{mpsc, oneshot};

static POOL_IDS: AtomicU64 = AtomicU64::new(1);
const REQUEST_BATCH_MAX: usize = 8;
static PERF_PROFILE_ENABLED: OnceLock<bool> = OnceLock::new();
static PERF_COUNT: AtomicU64 = AtomicU64::new(0);
static PERF_QUEUE_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
static PERF_WARM_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
static PERF_EXEC_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
static PERF_EVENT_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
static PERF_RESULT_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
static PERF_TOTAL_MS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static CURRENT_WORKER_ID: Cell<Option<usize>> = Cell::new(None);
    static CURRENT_POOL_ID: Cell<Option<u64>> = Cell::new(None);
}

use crate::validation;
use crate::esm_loader::{
    PhpxEsmLoader, entry_wrapper_path, hash_module_graph, resolve_project_root,
};

// ========== OS-level Thread CPU Time ==========

/// Get CPU time consumed by current thread
#[cfg(target_os = "linux")]
fn get_thread_cpu_time() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe {
        libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts);
    }
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// Get CPU time consumed by current thread (macOS)
#[cfg(target_os = "macos")]
fn get_thread_cpu_time() -> Duration {
    use libc::{THREAD_BASIC_INFO, thread_basic_info, thread_info};
    use mach2::mach_init::mach_thread_self;

    unsafe {
        let mut info = std::mem::zeroed::<thread_basic_info>();
        let mut count =
            (std::mem::size_of::<thread_basic_info>() / std::mem::size_of::<libc::c_int>()) as u32;

        let kr = thread_info(
            mach_thread_self(),
            THREAD_BASIC_INFO as u32,
            &mut info as *mut _ as *mut _,
            &mut count,
        );

        if kr == 0 {
            let user_secs = info.user_time.seconds as u64;
            let user_usecs = info.user_time.microseconds as u32;
            let sys_secs = info.system_time.seconds as u64;
            let sys_usecs = info.system_time.microseconds as u32;

            let user = Duration::new(user_secs, user_usecs * 1000);
            let sys = Duration::new(sys_secs, sys_usecs * 1000);

            user + sys
        } else {
            Duration::ZERO
        }
    }
}

/// Fallback for unsupported platforms
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn get_thread_cpu_time() -> Duration {
    Duration::ZERO
}

fn perf_profile_enabled() -> bool {
    *PERF_PROFILE_ENABLED.get_or_init(|| {
        std::env::var("DEKA_PERF_MODE")
            .map(|value| value != "false" && value != "0")
            .unwrap_or(false)
    })
}

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

// ========== Request/Response Types ==========

/// Unique identifier for a handler (routes to consistent worker)
#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub struct HandlerKey {
    pub name: String,
}

impl HandlerKey {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// Data needed to execute a request
#[derive(Clone)]
pub struct RequestData {
    pub handler_code: String,
    pub handler_entry: Option<String>,
    pub request_value: serde_json::Value,
    pub request_parts: Option<RequestParts>,
    pub mode: ExecutionMode,
}

#[derive(Clone)]
pub struct RequestParts {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Request,
    Module,
}

fn parse_exit_code(message: &str) -> Option<i64> {
    let marker = "DekaExit:";
    let idx = message.find(marker)?;
    let tail = &message[idx + marker.len()..];
    let digits: String = tail
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// Response from isolate execution
pub struct IsolateResponse {
    pub success: bool,
    pub error: Option<String>,
    pub result: Option<serde_json::Value>,
    /// Time spent getting/creating the isolate
    pub warm_time_us: u64,
    /// Total execution time
    pub total_time_us: u64,
    /// Whether this was a cache hit
    pub cache_hit: bool,
}

/// Internal request sent to worker thread
struct WorkerRequest {
    handler_key: HandlerKey,
    request_data: RequestData,
    request_id: String,
    enqueued_at: Instant,
    /// Response channel
    response_tx: oneshot::Sender<IsolateResponse>,
}

/// Control commands sent to workers
enum WorkerControl {
    /// Clear all cached isolates
    EvictAll { response_tx: oneshot::Sender<usize> },

    /// Kill a specific isolate
    KillIsolate {
        key: HandlerKey,
        response_tx: oneshot::Sender<Result<(), String>>,
    },

    /// Get metrics for a specific isolate
    GetIsolateMetrics {
        key: HandlerKey,
        response_tx: oneshot::Sender<Option<IsolateMetrics>>,
    },

    /// Get metrics for all isolates on this worker
    GetAllMetrics {
        response_tx: oneshot::Sender<Vec<(HandlerKey, IsolateMetrics)>>,
    },
    /// Get recent request traces for this worker
    GetRecentRequests {
        response_tx: oneshot::Sender<Vec<RequestTrace>>,
    },
    /// Drain request history entries at or before a cutoff timestamp
    DrainRequestHistory {
        cutoff_ms: u64,
        response_tx: oneshot::Sender<Vec<RequestTrace>>,
    },
}

// ========== Pool Metrics ==========

/// Metrics for monitoring pool health
pub struct PoolMetrics {
    pub total_requests: AtomicU64,
    pub cache_hits: AtomicU64,
    pub cache_misses: AtomicU64,
    pub evictions: AtomicU64,
}

impl Default for PoolMetrics {
    fn default() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }
}

impl PoolMetrics {
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.total_requests.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let hits = self.cache_hits.load(Ordering::Relaxed);
        hits as f64 / total as f64
    }

    /// Get metrics as a JSON-serializable snapshot
    pub fn to_json(&self) -> serde_json::Value {
        let total = self.total_requests.load(Ordering::Relaxed);
        let hits = self.cache_hits.load(Ordering::Relaxed);
        let misses = self.cache_misses.load(Ordering::Relaxed);
        let evictions = self.evictions.load(Ordering::Relaxed);

        serde_json::json!({
            "total_requests": total,
            "cache_hits": hits,
            "cache_misses": misses,
            "cache_hit_rate": self.cache_hit_rate(),
            "evictions": evictions
        })
    }
}

// ========== Isolate Metrics ==========

/// Load stats for a worker (used by scheduler)
struct WorkerLoad {
    queued_requests: AtomicUsize,
    active_requests: AtomicUsize,
}

impl Default for WorkerLoad {
    fn default() -> Self {
        Self {
            queued_requests: AtomicUsize::new(0),
            active_requests: AtomicUsize::new(0),
        }
    }
}

/// State of an isolate
#[derive(Debug, Clone, serde::Serialize)]
pub enum IsolateState {
    Idle,
    Executing {
        request_id: String,
        #[serde(skip)]
        started_at: Instant,
    },
    Stuck {
        request_id: String,
        #[serde(skip)]
        started_at: Instant,
        timeout_triggered: bool,
    },
}

impl std::fmt::Display for IsolateState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IsolateState::Idle => write!(f, "Idle"),
            IsolateState::Executing { started_at, .. } => {
                write!(f, "Executing ({}ms)", started_at.elapsed().as_millis())
            }
            IsolateState::Stuck {
                started_at,
                timeout_triggered,
                ..
            } => {
                if *timeout_triggered {
                    write!(f, "Stuck (timeout, {}ms)", started_at.elapsed().as_millis())
                } else {
                    write!(f, "Stuck ({}ms)", started_at.elapsed().as_millis())
                }
            }
        }
    }
}

/// Per-isolate metrics for observability
#[derive(Debug, Clone, serde::Serialize)]
pub struct IsolateMetrics {
    pub isolate_id: String,
    pub handler_name: String,
    pub worker_id: usize,

    // Request stats
    pub total_requests: u64,
    pub active_requests: u64,

    // V8 heap
    pub heap_used_bytes: usize,
    pub heap_limit_bytes: usize,

    // Derived metrics (computed on demand)
    pub cpu_percent: f64,
    pub avg_latency_ms: f64,

    // State
    pub state: IsolateState,

    // Op-level timing summary
    pub op_timings: Vec<OpTimingSummary>,
}

impl IsolateMetrics {
    /// Create metrics from WarmIsolate
    fn from_isolate(
        key: &HandlerKey,
        worker_id: usize,
        isolate: &WarmIsolate,
        cpu_time: Duration,
        wall_time: Duration,
    ) -> Self {
        let cpu_percent = if wall_time.as_secs_f64() > 0.0 {
            (cpu_time.as_secs_f64() / wall_time.as_secs_f64()) * 100.0
        } else {
            0.0
        };

        let avg_latency_ms = if isolate.request_count > 0 {
            wall_time.as_secs_f64() * 1000.0 / isolate.request_count as f64
        } else {
            0.0
        };

        Self {
            isolate_id: isolate.isolate_id.clone(),
            handler_name: key.name.clone(),
            worker_id,
            total_requests: isolate.request_count,
            active_requests: isolate.active_requests,
            heap_used_bytes: isolate.heap_used_bytes,
            heap_limit_bytes: isolate.heap_limit_bytes,
            cpu_percent,
            avg_latency_ms,
            state: isolate.state.clone(),
            op_timings: isolate
                .op_metrics
                .as_ref()
                .map(|metrics| metrics.top_ops(10))
                .unwrap_or_default(),
        }
    }
}

/// Summary for a single op's timing
#[derive(Debug, Clone, serde::Serialize)]
pub struct OpTimingSummary {
    pub name: String,
    pub count: u64,
    pub total_ms: f64,
    pub avg_ms: f64,
    pub in_flight: usize,
}

#[derive(Default, Clone)]
struct OpTimingAccum {
    count: u64,
    total: Duration,
}

#[derive(Default)]
struct OpTimingTracker {
    names: RefCell<Vec<String>>,
    totals: RefCell<Vec<OpTimingAccum>>,
    inflight: RefCell<Vec<VecDeque<Instant>>>,
}

#[derive(Clone)]
struct OpTimingSnapshot {
    names: Vec<String>,
    totals: Vec<OpTimingAccum>,
}

struct ExecutionProfile {
    heap_before_bytes: usize,
    heap_after_bytes: usize,
    exec_script_ms: u64,
    event_loop_ms: u64,
    result_decode_ms: u64,
}

impl ExecutionProfile {
    fn empty() -> Self {
        Self {
            heap_before_bytes: 0,
            heap_after_bytes: 0,
            exec_script_ms: 0,
            event_loop_ms: 0,
            result_decode_ms: 0,
        }
    }
}

impl OpTimingTracker {
    fn op_metrics_factory_fn(self: Rc<Self>) -> OpMetricsFactoryFn {
        Box::new(move |op_id, total, op_decl| {
            self.ensure_capacity(total);
            self.names.borrow_mut()[op_id as usize] = op_decl.name.to_string();
            Some(self.clone().op_metrics_fn())
        })
    }

    fn op_metrics_fn(self: Rc<Self>) -> OpMetricsFn {
        Rc::new(move |ctx, event, _source| {
            let op_id = ctx.id as usize;
            match event {
                OpMetricsEvent::Dispatched => {
                    if let Some(queue) = self.inflight.borrow_mut().get_mut(op_id) {
                        queue.push_back(Instant::now());
                    }
                }
                OpMetricsEvent::Completed
                | OpMetricsEvent::Error
                | OpMetricsEvent::CompletedAsync
                | OpMetricsEvent::ErrorAsync => {
                    let mut inflight = self.inflight.borrow_mut();
                    let mut totals = self.totals.borrow_mut();
                    if let (Some(queue), Some(accum)) =
                        (inflight.get_mut(op_id), totals.get_mut(op_id))
                    {
                        if let Some(start) = queue.pop_front() {
                            accum.count += 1;
                            accum.total += start.elapsed();
                        }
                    }
                }
            }
        })
    }

    fn top_ops(&self, limit: usize) -> Vec<OpTimingSummary> {
        let names = self.names.borrow();
        let totals = self.totals.borrow();
        let inflight = self.inflight.borrow();

        let mut summaries: Vec<OpTimingSummary> = totals
            .iter()
            .enumerate()
            .filter_map(|(idx, accum)| {
                let in_flight = inflight.get(idx).map(|q| q.len()).unwrap_or(0);
                if accum.count == 0 && in_flight == 0 {
                    return None;
                }
                let total_ms = accum.total.as_secs_f64() * 1000.0;
                let avg_ms = if accum.count > 0 {
                    total_ms / accum.count as f64
                } else {
                    0.0
                };
                Some(OpTimingSummary {
                    name: names.get(idx).cloned().unwrap_or_default(),
                    count: accum.count,
                    total_ms,
                    avg_ms,
                    in_flight,
                })
            })
            .collect();

        summaries.sort_by(|a, b| {
            b.total_ms
                .partial_cmp(&a.total_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        summaries.into_iter().take(limit).collect()
    }

    fn snapshot(&self) -> OpTimingSnapshot {
        OpTimingSnapshot {
            names: self.names.borrow().clone(),
            totals: self.totals.borrow().clone(),
        }
    }

    fn diff(&self, before: &OpTimingSnapshot, limit: usize) -> Vec<RequestOpTiming> {
        let after = self.snapshot();
        let max_len = after.totals.len().max(before.totals.len());
        let mut summaries = Vec::new();

        for idx in 0..max_len {
            let after_accum = after.totals.get(idx);
            let before_accum = before.totals.get(idx);
            let after_count = after_accum.map(|a| a.count).unwrap_or(0);
            let before_count = before_accum.map(|a| a.count).unwrap_or(0);
            let count = after_count.saturating_sub(before_count);
            if count == 0 {
                continue;
            }

            let after_total = after_accum.map(|a| a.total).unwrap_or(Duration::ZERO);
            let before_total = before_accum.map(|a| a.total).unwrap_or(Duration::ZERO);
            let total = after_total.saturating_sub(before_total);
            let total_ms = total.as_secs_f64() * 1000.0;
            let avg_ms = total_ms / count as f64;
            let name = after
                .names
                .get(idx)
                .cloned()
                .or_else(|| before.names.get(idx).cloned())
                .unwrap_or_default();

            summaries.push(RequestOpTiming {
                name,
                count,
                total_ms,
                avg_ms,
            });
        }

        summaries.sort_by(|a, b| {
            b.total_ms
                .partial_cmp(&a.total_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        summaries.into_iter().take(limit).collect()
    }

    fn ensure_capacity(&self, total: usize) {
        let mut names = self.names.borrow_mut();
        let mut totals = self.totals.borrow_mut();
        let mut inflight = self.inflight.borrow_mut();

        if names.len() < total {
            names.resize(total, String::new());
            totals.resize_with(total, OpTimingAccum::default);
            inflight.resize_with(total, VecDeque::new);
        }
    }
}

// ========== Worker Handle ==========

/// Handle to communicate with a worker thread
struct WorkerHandle {
    request_tx: mpsc::UnboundedSender<WorkerRequest>,
    control_tx: mpsc::UnboundedSender<WorkerControl>,
    load: Arc<WorkerLoad>,
    #[allow(dead_code)]
    thread: JoinHandle<()>,
}

// ========== Main Pool ==========

/// The isolate pool manager
pub struct IsolatePool {
    workers: Vec<WorkerHandle>,
    config: PoolConfig,
    metrics: Arc<PoolMetrics>,
    request_seq: AtomicU64,
    introspect_profiling: Arc<AtomicBool>,
    pool_id: u64,
}

impl IsolatePool {
    /// Create a new isolate pool with the given configuration
    pub fn new(
        config: PoolConfig,
        extensions_provider: Arc<dyn Fn() -> Vec<Extension> + Send + Sync>,
    ) -> Self {
        let metrics = Arc::new(PoolMetrics::default());
        let introspect_profiling = Arc::new(AtomicBool::new(config.introspect_profiling));
        let mut workers = Vec::with_capacity(config.num_workers);
        let pool_id = POOL_IDS.fetch_add(1, Ordering::Relaxed);
        let core_ids = core_affinity::get_core_ids();

        tracing::info!(
            "Initializing isolate pool: {} workers, {} max isolates/worker",
            config.num_workers,
            config.max_isolates_per_worker
        );

        for worker_id in 0..config.num_workers {
            let (tx, rx) = mpsc::unbounded_channel();
            let (ctrl_tx, ctrl_rx) = mpsc::unbounded_channel();
            let worker_config = config.clone();
            let worker_metrics = Arc::clone(&metrics);
            let ext_provider = Arc::clone(&extensions_provider);
            let load = Arc::new(WorkerLoad::default());
            let worker_load = Arc::clone(&load);
            let profiling = Arc::clone(&introspect_profiling);
            let core_id = core_ids
                .as_ref()
                .and_then(|ids| ids.get(worker_id % ids.len()).cloned());

            let thread = thread::spawn(move || {
                if let Some(core_id) = core_id {
                    core_affinity::set_for_current(core_id);
                }
                let mut worker = WorkerThread::new(
                    worker_id,
                    pool_id,
                    worker_config,
                    worker_metrics,
                    worker_load,
                    ext_provider,
                    profiling,
                );
                worker.run(rx, ctrl_rx);
            });

            workers.push(WorkerHandle {
                request_tx: tx,
                control_tx: ctrl_tx,
                load,
                thread,
            });
        }

        Self {
            workers,
            config,
            metrics,
            request_seq: AtomicU64::new(0),
            introspect_profiling,
            pool_id,
        }
    }

    /// Execute a handler request through the pool
    pub async fn execute(
        &self,
        handler_key: HandlerKey,
        request_data: RequestData,
    ) -> Result<IsolateResponse, String> {
        let current_worker = CURRENT_WORKER_ID.with(|cell| cell.get());
        let current_pool = CURRENT_POOL_ID.with(|cell| cell.get());
        if current_pool == Some(self.pool_id) && self.workers.len() == 1 {
            return Err(
                "IsolatePool has a single worker; Isolate.run requires at least 2 workers"
                    .to_string(),
            );
        }
        let worker_index = self.select_worker_with_exclude(&handler_key, current_worker);
        let request_id = self.next_request_id();
        let enqueued_at = Instant::now();

        let (response_tx, response_rx) = oneshot::channel();

        let request = WorkerRequest {
            handler_key,
            request_data,
            request_id,
            enqueued_at,
            response_tx,
        };

        self.workers[worker_index]
            .load
            .queued_requests
            .fetch_add(1, Ordering::Relaxed);

        self.workers[worker_index]
            .request_tx
            .send(request)
            .map_err(|_| "Worker thread dead".to_string())?;

        response_rx
            .await
            .map_err(|_| "Worker dropped response channel".to_string())
    }

    /// Hash handler key to worker index
    fn hash_to_worker(&self, key: &HandlerKey) -> usize {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.workers.len()
    }

    fn select_worker_with_exclude(&self, key: &HandlerKey, exclude: Option<usize>) -> usize {
        if self.workers.len() <= 1 {
            return 0;
        }
        match self.config.scheduler_strategy {
            SchedulerStrategy::ConsistentHash => {
                let mut index = self.hash_to_worker(key);
                if Some(index) == exclude {
                    index = (index + 1) % self.workers.len();
                }
                index
            }
            SchedulerStrategy::LeastLoaded => {
                let mut best: Option<(usize, usize)> = None;
                for (index, worker) in self.workers.iter().enumerate() {
                    if Some(index) == exclude {
                        continue;
                    }
                    let queued = worker.load.queued_requests.load(Ordering::Relaxed);
                    let active = worker.load.active_requests.load(Ordering::Relaxed);
                    let load = queued + active;
                    match best {
                        None => best = Some((index, load)),
                        Some((_, best_load)) if load < best_load => best = Some((index, load)),
                        _ => {}
                    }
                }
                best.map(|(index, _)| index).unwrap_or(0)
            }
        }
    }

    fn next_request_id(&self) -> String {
        let id = self.request_seq.fetch_add(1, Ordering::Relaxed);
        format!("req_{}", id)
    }

    /// Get full pool stats as JSON (for /stats endpoint)
    pub fn stats(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": true,
            "config": {
                "num_workers": self.config.num_workers,
                "max_isolates_per_worker": self.config.max_isolates_per_worker,
                "idle_timeout_secs": self.config.idle_timeout_secs,
                "metrics_enabled": self.config.enable_metrics,
                "code_cache_enabled": self.config.enable_code_cache,
                "request_timeout_ms": self.config.request_timeout_ms,
                "queue_timeout_ms": self.config.queue_timeout_ms,
                "scheduler": match self.config.scheduler_strategy {
                    SchedulerStrategy::ConsistentHash => "consistent_hash",
                    SchedulerStrategy::LeastLoaded => "least_loaded",
                },
                "introspect_profiling": self.introspect_profiling.load(Ordering::Relaxed)
            },
            "metrics": self.metrics.to_json()
        })
    }

    pub async fn set_introspect_profiling(&self, enabled: bool) -> usize {
        self.introspect_profiling.store(enabled, Ordering::Relaxed);
        self.evict_all().await
    }

    /// Evict all cached isolates across all workers
    /// Returns the total number of isolates evicted
    pub async fn evict_all(&self) -> usize {
        let mut total_evicted = 0;
        let mut receivers = Vec::new();

        // Send evict command to all workers
        for worker in &self.workers {
            let (tx, rx) = oneshot::channel();
            if worker
                .control_tx
                .send(WorkerControl::EvictAll { response_tx: tx })
                .is_ok()
            {
                receivers.push(rx);
            }
        }

        // Collect responses
        for rx in receivers {
            if let Ok(count) = rx.await {
                total_evicted += count;
            }
        }

        // Update metrics
        self.metrics
            .evictions
            .fetch_add(total_evicted as u64, Ordering::Relaxed);

        tracing::info!(
            "Cache eviction complete: {} isolates evicted across {} workers",
            total_evicted,
            self.workers.len()
        );

        total_evicted
    }

    /// Kill a specific isolate by handler name
    pub async fn kill_isolate(&self, handler_name: String) -> Result<(), String> {
        let key = HandlerKey::new(handler_name);
        let worker_index = self.hash_to_worker(&key);

        let (tx, rx) = oneshot::channel();

        self.workers[worker_index]
            .control_tx
            .send(WorkerControl::KillIsolate {
                key,
                response_tx: tx,
            })
            .map_err(|_| "Worker dead".to_string())?;

        rx.await
            .map_err(|_| "Worker dropped response".to_string())?
    }

    /// Get metrics for a specific isolate
    pub async fn get_isolate_metrics(&self, handler_name: String) -> Option<IsolateMetrics> {
        let key = HandlerKey::new(handler_name);
        let worker_index = self.hash_to_worker(&key);

        let (tx, rx) = oneshot::channel();

        if self.workers[worker_index]
            .control_tx
            .send(WorkerControl::GetIsolateMetrics {
                key,
                response_tx: tx,
            })
            .is_err()
        {
            return None;
        }

        rx.await.ok().flatten()
    }

    /// Get top isolates sorted by a metric
    pub async fn get_top_isolates(&self, sort_by: SortBy, limit: usize) -> Vec<IsolateMetrics> {
        let mut all_metrics = Vec::new();

        // Collect metrics from all workers
        for worker in &self.workers {
            let (tx, rx) = oneshot::channel();

            if worker
                .control_tx
                .send(WorkerControl::GetAllMetrics { response_tx: tx })
                .is_ok()
            {
                if let Ok(worker_metrics) = rx.await {
                    all_metrics.extend(worker_metrics.into_iter().map(|(_, m)| m));
                }
            }
        }

        // Sort by requested metric
        all_metrics.sort_by(|a, b| match sort_by {
            SortBy::Cpu => b
                .cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal),
            SortBy::Memory => b.heap_used_bytes.cmp(&a.heap_used_bytes),
            SortBy::Requests => b.total_requests.cmp(&a.total_requests),
        });

        // Take top N
        all_metrics.into_iter().take(limit).collect()
    }

    /// Get worker statistics
    pub async fn get_worker_stats(&self) -> Vec<WorkerStats> {
        let mut stats = Vec::new();

        for (worker_id, worker) in self.workers.iter().enumerate() {
            let (tx, rx) = oneshot::channel();

            if worker
                .control_tx
                .send(WorkerControl::GetAllMetrics { response_tx: tx })
                .is_ok()
            {
                if let Ok(metrics) = rx.await {
                    let active_isolates = metrics.len();
                    let total_requests: u64 = metrics.iter().map(|(_, m)| m.total_requests).sum();
                    let avg_latency = if total_requests > 0 {
                        metrics.iter().map(|(_, m)| m.avg_latency_ms).sum::<f64>()
                            / metrics.len() as f64
                    } else {
                        0.0
                    };

                    stats.push(WorkerStats {
                        worker_id,
                        active_isolates,
                        queued_requests: worker.load.queued_requests.load(Ordering::Relaxed),
                        total_requests,
                        avg_latency_ms: avg_latency,
                    });
                }
            }
        }

        stats
    }

    /// Get recent request traces across all workers
    pub async fn get_recent_requests(&self, limit: usize) -> Vec<RequestTrace> {
        let mut traces = Vec::new();

        for worker in &self.workers {
            let (tx, rx) = oneshot::channel();
            if worker
                .control_tx
                .send(WorkerControl::GetRecentRequests { response_tx: tx })
                .is_ok()
            {
                if let Ok(mut worker_traces) = rx.await {
                    traces.append(&mut worker_traces);
                }
            }
        }

        traces.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
        traces.into_iter().take(limit).collect()
    }

    pub async fn drain_request_history_before(&self, cutoff_ms: u64) -> Vec<RequestTrace> {
        let mut traces = Vec::new();

        for worker in &self.workers {
            let (tx, rx) = oneshot::channel();
            if worker
                .control_tx
                .send(WorkerControl::DrainRequestHistory {
                    cutoff_ms,
                    response_tx: tx,
                })
                .is_ok()
            {
                if let Ok(mut worker_traces) = rx.await {
                    traces.append(&mut worker_traces);
                }
            }
        }

        traces
    }
}

/// Sort criteria for isolate listing
#[derive(Debug, Clone, Copy)]
pub enum SortBy {
    Cpu,
    Memory,
    Requests,
}

/// Worker statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkerStats {
    pub worker_id: usize,
    pub active_isolates: usize,
    pub queued_requests: usize,
    pub total_requests: u64,
    pub avg_latency_ms: f64,
}

/// State of a request for observability
#[derive(Debug, Clone, serde::Serialize)]
pub enum RequestState {
    Executing,
    Completed { duration_ms: u64 },
    Failed { error: String, duration_ms: u64 },
    QueueTimeout { waited_ms: u64 },
}

/// Per-request op timing summary
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RequestOpTiming {
    pub name: String,
    pub count: u64,
    pub total_ms: f64,
    pub avg_ms: f64,
}

/// Recent request trace entry
#[derive(Debug, Clone, serde::Serialize)]
pub struct RequestTrace {
    pub id: String,
    pub handler_name: String,
    pub isolate_id: String,
    pub worker_id: usize,
    pub started_at_ms: u64,
    pub state: RequestState,
    pub op_timings: Vec<RequestOpTiming>,
    pub queue_wait_ms: u64,
    pub warm_time_us: u64,
    pub total_time_us: u64,
    pub heap_before_bytes: usize,
    pub heap_after_bytes: usize,
    pub heap_delta_bytes: i64,
    pub response_status: Option<u16>,
    pub response_body: Option<String>,
}

const REQUEST_HISTORY_LIMIT: usize = 200;

// ========== Warm Isolate ==========

/// A warm isolate with metadata
struct WarmIsolate {
    isolate_id: String,
    runtime: JsRuntime,
    last_used: Instant,
    request_count: u64,
    active_requests: u64,
    /// Hash of handler source - for cache invalidation on redeploy
    source_hash: u64,
    /// Whether this isolate has been bootstrapped
    bootstrapped: bool,
    /// Total CPU time consumed
    total_cpu_time: Duration,
    /// Total wall time (created_at to now)
    created_at: Instant,
    /// V8 heap usage
    heap_used_bytes: usize,
    heap_limit_bytes: usize,
    state: IsolateState,
    op_metrics: Option<Rc<OpTimingTracker>>,
    handler_loaded: bool,
    entry_specifier: Option<ModuleSpecifier>,
}

// ========== Worker Thread ==========

/// Worker thread that owns isolates locally
struct WorkerThread {
    worker_id: usize,
    pool_id: u64,
    config: PoolConfig,
    introspect_profiling: Arc<AtomicBool>,
    metrics: Arc<PoolMetrics>,
    load: Arc<WorkerLoad>,
    isolates: HashMap<HandlerKey, WarmIsolate>,
    lru_order: Vec<HandlerKey>, // Front = oldest, back = newest
    code_cache: HashMap<u64, Vec<u8>>,
    extensions_provider: Arc<dyn Fn() -> Vec<Extension> + Send + Sync>,
    request_history: VecDeque<RequestTrace>,
    deka_args: serde_json::Value,
}

enum ExecutionOutcome {
    Ok(serde_json::Value),
    Err(String),
    TimedOut,
}

impl WorkerThread {
    fn new(
        worker_id: usize,
        pool_id: u64,
        config: PoolConfig,
        metrics: Arc<PoolMetrics>,
        load: Arc<WorkerLoad>,
        extensions_provider: Arc<dyn Fn() -> Vec<Extension> + Send + Sync>,
        introspect_profiling: Arc<AtomicBool>,
    ) -> Self {
        let deka_args = std::env::var("DEKA_ARGS").unwrap_or_else(|_| "[]".to_string());
        let deka_args = serde_json::from_str(&deka_args).unwrap_or_else(|_| serde_json::json!([]));
        Self {
            worker_id,
            pool_id,
            config,
            introspect_profiling,
            metrics,
            load,
            isolates: HashMap::new(),
            lru_order: Vec::new(),
            code_cache: HashMap::new(),
            extensions_provider,
            request_history: VecDeque::new(),
            deka_args,
        }
    }

    /// Main event loop - runs on dedicated thread
    fn run(
        &mut self,
        mut rx: mpsc::UnboundedReceiver<WorkerRequest>,
        mut ctrl_rx: mpsc::UnboundedReceiver<WorkerControl>,
    ) {
        CURRENT_WORKER_ID.with(|cell| cell.set(Some(self.worker_id)));
        CURRENT_POOL_ID.with(|cell| cell.set(Some(self.pool_id)));

        // Create a tokio runtime for this thread (needed for async ops in V8)
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for worker");

        tracing::debug!("Worker {} started", self.worker_id);

        rt.block_on(async {
            loop {
                tokio::select! {
                    // Handle regular requests
                    Some(request) = rx.recv() => {
                        let mut batch = Vec::with_capacity(REQUEST_BATCH_MAX);
                        batch.push(request);
                        while batch.len() < REQUEST_BATCH_MAX {
                            match rx.try_recv() {
                                Ok(request) => batch.push(request),
                                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                            }
                        }
                        for request in batch {
                            let response = self.process_request(&request).await;
                            let _ = request.response_tx.send(response);
                        }
                    }
                    // Handle control commands
                    Some(cmd) = ctrl_rx.recv() => {
                        self.handle_control(cmd);
                    }
                    // Both channels closed - shutdown
                    else => break,
                }
            }
        });

        tracing::debug!("Worker {} shutting down", self.worker_id);
    }

    /// Handle control commands
    fn handle_control(&mut self, cmd: WorkerControl) {
        match cmd {
            WorkerControl::EvictAll { response_tx } => {
                let count = self.isolates.len();
                self.isolates.clear();
                self.lru_order.clear();
                self.code_cache.clear();
                tracing::debug!("Worker {} evicted {} isolates", self.worker_id, count);
                let _ = response_tx.send(count);
            }

            WorkerControl::KillIsolate { key, response_tx } => {
                if self.isolates.remove(&key).is_some() {
                    self.lru_order.retain(|k| k != &key);
                    tracing::info!("Worker {} killed isolate: {}", self.worker_id, key.name);
                    let _ = response_tx.send(Ok(()));
                } else {
                    let _ = response_tx.send(Err("Isolate not found".to_string()));
                }
            }

            WorkerControl::GetIsolateMetrics { key, response_tx } => {
                let metrics = self.isolates.get(&key).map(|isolate| {
                    let wall_time = isolate.created_at.elapsed();
                    IsolateMetrics::from_isolate(
                        &key,
                        self.worker_id,
                        isolate,
                        isolate.total_cpu_time,
                        wall_time,
                    )
                });
                let _ = response_tx.send(metrics);
            }

            WorkerControl::GetAllMetrics { response_tx } => {
                let all_metrics: Vec<(HandlerKey, IsolateMetrics)> = self
                    .isolates
                    .iter()
                    .map(|(key, isolate)| {
                        let wall_time = isolate.created_at.elapsed();
                        let metrics = IsolateMetrics::from_isolate(
                            key,
                            self.worker_id,
                            isolate,
                            isolate.total_cpu_time,
                            wall_time,
                        );
                        (key.clone(), metrics)
                    })
                    .collect();

                let _ = response_tx.send(all_metrics);
            }
            WorkerControl::GetRecentRequests { response_tx } => {
                let history: Vec<RequestTrace> = self.request_history.iter().cloned().collect();
                let _ = response_tx.send(history);
            }
            WorkerControl::DrainRequestHistory {
                cutoff_ms,
                response_tx,
            } => {
                let drained = self.drain_request_history_before(cutoff_ms);
                let _ = response_tx.send(drained);
            }
        }
    }

    /// Handle a single request
    async fn process_request(&mut self, request: &WorkerRequest) -> IsolateResponse {
        let start = Instant::now();
        self.metrics.total_requests.fetch_add(1, Ordering::Relaxed);
        self.load.queued_requests.fetch_sub(1, Ordering::Relaxed);
        let track_requests =
            self.config.enable_metrics || self.introspect_profiling.load(Ordering::Relaxed);

        let queue_wait_ms = request.enqueued_at.elapsed().as_millis() as u64;

        if self.config.queue_timeout_ms > 0 {
            let queued_for = Duration::from_millis(queue_wait_ms);
            if queued_for > Duration::from_millis(self.config.queue_timeout_ms) {
                self.record_request_trace(RequestTrace {
                    id: request.request_id.clone(),
                    handler_name: request.handler_key.name.clone(),
                    isolate_id: String::new(),
                    worker_id: self.worker_id,
                    started_at_ms: now_millis(),
                    state: RequestState::QueueTimeout {
                        waited_ms: queued_for.as_millis() as u64,
                    },
                    op_timings: Vec::new(),
                    queue_wait_ms,
                    warm_time_us: 0,
                    total_time_us: 0,
                    heap_before_bytes: 0,
                    heap_after_bytes: 0,
                    heap_delta_bytes: 0,
                    response_status: None,
                    response_body: None,
                });
                return IsolateResponse {
                    success: false,
                    error: Some(format!(
                        "Request timed out in queue after {}ms",
                        queued_for.as_millis()
                    )),
                    result: None,
                    warm_time_us: 0,
                    total_time_us: queued_for.as_micros() as u64,
                    cache_hit: false,
                };
            }
        }

        let use_esm_for_hash = request.request_data.handler_entry.is_some()
            && std::env::var("DEKA_RUNTIME_ESM")
                .map(|value| value != "0" && value != "false")
                .unwrap_or(true);

        // Compute source hash for cache validation
        let source_hash = if use_esm_for_hash {
            if let Some(entry) = request.request_data.handler_entry.as_ref() {
                match hash_module_graph(Path::new(entry)) {
                    Ok(hash) => hash,
                    Err(_) => Self::hash_source(entry),
                }
            } else {
                Self::hash_source("")
            }
        } else if request.request_data.handler_code.trim().is_empty() {
            if let Some(entry) = request.request_data.handler_entry.as_ref() {
                match std::fs::read_to_string(entry) {
                    Ok(contents) => Self::hash_source(&contents),
                    Err(_) => Self::hash_source(entry),
                }
            } else {
                Self::hash_source("")
            }
        } else {
            Self::hash_source(&request.request_data.handler_code)
        };
        let key = request.handler_key.clone();

        // Check cache and get/create isolate
        let (cache_hit, warm_time) = match self
            .ensure_isolate(&key, source_hash, request.request_data.handler_entry.as_deref())
            .await
        {
            Ok(value) => value,
            Err(err) => {
                return IsolateResponse {
                    success: false,
                    error: Some(err),
                    result: None,
                    warm_time_us: 0,
                    total_time_us: 0,
                    cache_hit: false,
                };
            }
        };

        let isolate_id = self
            .isolates
            .get(&key)
            .map(|isolate| isolate.isolate_id.clone())
            .unwrap_or_default();

        let op_snapshot_before = if track_requests {
            self.isolates.get(&key).and_then(|isolate| {
                isolate
                    .op_metrics
                    .as_ref()
                    .map(|metrics| metrics.snapshot())
            })
        } else {
            None
        };

        if track_requests {
            self.record_request_trace(RequestTrace {
                id: request.request_id.clone(),
                handler_name: request.handler_key.name.clone(),
                isolate_id,
                worker_id: self.worker_id,
                started_at_ms: now_millis(),
                state: RequestState::Executing,
                op_timings: Vec::new(),
                queue_wait_ms,
                warm_time_us: 0,
                total_time_us: 0,
                heap_before_bytes: 0,
                heap_after_bytes: 0,
                heap_delta_bytes: 0,
                response_status: None,
                response_body: None,
            });
        }

        self.load.active_requests.fetch_add(1, Ordering::Relaxed);

        // Execute in the isolate
        let (exec_result, exec_profile) = self.execute_in_isolate(&key, &request).await;

        let total_time = start.elapsed();
        self.load.active_requests.fetch_sub(1, Ordering::Relaxed);

        // Only log timing details in debug mode
        if self.config.enable_metrics {
            tracing::debug!(
                "Worker {} handler {} - warm: {:?}, total: {:?}, hit: {}, cache: {}",
                self.worker_id,
                request.handler_key.name,
                warm_time,
                total_time,
                cache_hit,
                self.isolates.len()
            );
        }

        let duration_ms = total_time.as_millis() as u64;
        let op_timings = if track_requests {
            self.isolates
                .get(&key)
                .and_then(|isolate| {
                    isolate.op_metrics.as_ref().and_then(|metrics| {
                        op_snapshot_before
                            .as_ref()
                            .map(|snap| metrics.diff(snap, 20))
                    })
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        if self.introspect_profiling.load(Ordering::Relaxed) && !op_timings.is_empty() {
            let summary = op_timings
                .iter()
                .map(|op| format!("{} {}x {:.2}ms", op.name, op.count, op.total_ms))
                .collect::<Vec<_>>()
                .join(", ");
            tracing::debug!(
                "Worker {} handler {} ops: {}",
                self.worker_id,
                request.handler_key.name,
                summary
            );
        }
        let (response, state, response_status, response_body) = match exec_result {
            ExecutionOutcome::Ok(result) => (
                IsolateResponse {
                    success: true,
                    error: None,
                    result: Some(result),
                    warm_time_us: warm_time.as_micros() as u64,
                    total_time_us: total_time.as_micros() as u64,
                    cache_hit,
                },
                RequestState::Completed { duration_ms },
                None,
                None,
            ),
            ExecutionOutcome::TimedOut => {
                self.isolates.remove(&key);
                self.lru_order.retain(|k| k != &key);
                (
                    IsolateResponse {
                        success: false,
                        error: Some("Handler execution timed out".to_string()),
                        result: None,
                        warm_time_us: warm_time.as_micros() as u64,
                        total_time_us: total_time.as_micros() as u64,
                        cache_hit,
                    },
                    RequestState::Failed {
                        error: "timeout".to_string(),
                        duration_ms,
                    },
                    None,
                    None,
                )
            }
            ExecutionOutcome::Err(e) => {
                let error =
                    validation::analyze_runtime_error(&e, &request.request_data.handler_code);
                (
                    IsolateResponse {
                        success: false,
                        error: Some(error.clone()),
                        result: None,
                        warm_time_us: warm_time.as_micros() as u64,
                        total_time_us: total_time.as_micros() as u64,
                        cache_hit,
                    },
                    RequestState::Failed { error, duration_ms },
                    None,
                    None,
                )
            }
        };

        let (response_status, response_body) = if let Some(result_json) = response.result.as_ref() {
            let status = result_json
                .get("status")
                .and_then(|value| value.as_u64())
                .unwrap_or(200) as u16;
            let body = result_json
                .get("body")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());
            (Some(status), body)
        } else {
            (response_status, response_body)
        };
        if track_requests {
            self.update_request_trace(
                &request.request_id,
                state,
                op_timings,
                warm_time.as_micros() as u64,
                total_time.as_micros() as u64,
                exec_profile.heap_before_bytes,
                exec_profile.heap_after_bytes,
                response_status,
                response_body,
            );
        }

        if perf_profile_enabled() {
            let count = PERF_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            PERF_QUEUE_TOTAL_MS.fetch_add(queue_wait_ms, Ordering::Relaxed);
            PERF_WARM_TOTAL_MS.fetch_add(warm_time.as_millis() as u64, Ordering::Relaxed);
            PERF_EXEC_TOTAL_MS.fetch_add(exec_profile.exec_script_ms, Ordering::Relaxed);
            PERF_EVENT_TOTAL_MS.fetch_add(exec_profile.event_loop_ms, Ordering::Relaxed);
            PERF_RESULT_TOTAL_MS.fetch_add(exec_profile.result_decode_ms, Ordering::Relaxed);
            PERF_TOTAL_MS.fetch_add(total_time.as_millis() as u64, Ordering::Relaxed);

            if count % 200 == 0 {
                let denom = count.max(1);
                let avg_queue = PERF_QUEUE_TOTAL_MS.load(Ordering::Relaxed) / denom;
                let avg_warm = PERF_WARM_TOTAL_MS.load(Ordering::Relaxed) / denom;
                let avg_exec = PERF_EXEC_TOTAL_MS.load(Ordering::Relaxed) / denom;
                let avg_event = PERF_EVENT_TOTAL_MS.load(Ordering::Relaxed) / denom;
                let avg_result = PERF_RESULT_TOTAL_MS.load(Ordering::Relaxed) / denom;
                let avg_total = PERF_TOTAL_MS.load(Ordering::Relaxed) / denom;
                deka_stdio::log("perf_avg_queue_ms", &avg_queue.to_string());
                deka_stdio::log("perf_avg_warm_ms", &avg_warm.to_string());
                deka_stdio::log("perf_avg_exec_ms", &avg_exec.to_string());
                deka_stdio::log("perf_avg_event_ms", &avg_event.to_string());
                deka_stdio::log("perf_avg_result_ms", &avg_result.to_string());
                deka_stdio::log("perf_avg_total_ms", &avg_total.to_string());
                deka_stdio::log("perf_count", &denom.to_string());
            }
        }

        response
    }

    fn record_request_trace(&mut self, trace: RequestTrace) {
        self.request_history.push_back(trace);
        if self.request_history.len() > REQUEST_HISTORY_LIMIT {
            self.request_history.pop_front();
        }
    }

    fn drain_request_history_before(&mut self, cutoff_ms: u64) -> Vec<RequestTrace> {
        let mut drained = Vec::new();

        while let Some(front) = self.request_history.front() {
            if front.started_at_ms > cutoff_ms {
                break;
            }

            if matches!(front.state, RequestState::Executing) {
                break;
            }

            drained.push(self.request_history.pop_front().unwrap());
        }

        drained
    }

    fn update_request_trace(
        &mut self,
        request_id: &str,
        state: RequestState,
        op_timings: Vec<RequestOpTiming>,
        warm_time_us: u64,
        total_time_us: u64,
        heap_before_bytes: usize,
        heap_after_bytes: usize,
        response_status: Option<u16>,
        response_body: Option<String>,
    ) {
        if let Some(entry) = self
            .request_history
            .iter_mut()
            .find(|entry| entry.id == request_id)
        {
            entry.state = state;
            entry.op_timings = op_timings;
            entry.warm_time_us = warm_time_us;
            entry.total_time_us = total_time_us;
            entry.heap_before_bytes = heap_before_bytes;
            entry.heap_after_bytes = heap_after_bytes;
            entry.heap_delta_bytes = heap_after_bytes as i64 - heap_before_bytes as i64;
            entry.response_status = response_status;
            entry.response_body = response_body;
        }
    }

    /// Ensure we have a valid isolate for this handler, creating if needed
    async fn ensure_isolate(
        &mut self,
        key: &HandlerKey,
        source_hash: u64,
        handler_entry: Option<&str>,
    ) -> Result<(bool, std::time::Duration), String> {
        let start = Instant::now();

        // Check if we have a valid cached isolate
        let needs_create = if let Some(isolate) = self.isolates.get(key) {
            if isolate.source_hash == source_hash {
                // Valid cache hit - update LRU and metadata
                self.metrics.cache_hits.fetch_add(1, Ordering::Relaxed);
                self.touch_lru(key);

                // Update metadata on the isolate
                if let Some(isolate) = self.isolates.get_mut(key) {
                    isolate.last_used = Instant::now();
                    isolate.request_count += 1;
                }
                return Ok((true, start.elapsed()));
            } else {
                // Handler was redeployed - invalidate
                tracing::debug!(
                    "Worker {} handler {} source changed, invalidating cache",
                    self.worker_id,
                    key.name
                );
                true
            }
        } else {
            true
        };

        if needs_create {
            // Remove stale entry if exists
            self.isolates.remove(key);
            self.lru_order.retain(|k| k != key);

            // Cache MISS
            self.metrics.cache_misses.fetch_add(1, Ordering::Relaxed);

            // Check if we need to evict (only if max_isolates_per_worker > 0)
            if self.config.max_isolates_per_worker > 0
                && self.isolates.len() >= self.config.max_isolates_per_worker
            {
                self.evict_lru();
            }

            // Create new warm isolate
            match self.create_warm_isolate(source_hash, handler_entry) {
                Ok(isolate) => {
                    self.isolates.insert(key.clone(), isolate);
                    self.lru_order.push(key.clone());
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }

        Ok((false, start.elapsed()))
    }

    /// Create a new warm isolate
    fn create_warm_isolate(
        &self,
        source_hash: u64,
        handler_entry: Option<&str>,
    ) -> Result<WarmIsolate, String> {
        let extensions = (self.extensions_provider)();
        let isolate_id = format!("isolate_{}", nanoid!(10, &ID_ALPHABET));

        let op_metrics = if self.introspect_profiling.load(Ordering::Relaxed) {
            Some(Rc::new(OpTimingTracker::default()))
        } else {
            None
        };
        let (module_loader, entry_specifier) = if let Some(entry) = handler_entry {
            let entry_path = Path::new(entry).to_path_buf();
            let project_root = resolve_project_root(&entry_path)?;
            let wrapper_path = entry_wrapper_path(&project_root);
            let wrapper_specifier = ModuleSpecifier::from_file_path(&wrapper_path)
                .map_err(|_| "invalid entry wrapper path".to_string())?;
            let loader =
                PhpxEsmLoader::new(project_root, entry_path).map_err(|err| err.to_string())?;
            let loader: Rc<dyn deno_core::ModuleLoader> = Rc::new(loader);
            (Some(loader), Some(wrapper_specifier))
        } else {
            (None, None)
        };

        let runtime = JsRuntime::new(RuntimeOptions {
            extensions,
            op_metrics_factory_fn: op_metrics
                .as_ref()
                .map(|metrics| metrics.clone().op_metrics_factory_fn()),
            module_loader,
            ..Default::default()
        });

        Ok(WarmIsolate {
            isolate_id,
            runtime,
            last_used: Instant::now(),
            request_count: 1,
            active_requests: 0,
            source_hash,
            bootstrapped: false, // Will be bootstrapped on first request
            total_cpu_time: Duration::ZERO,
            created_at: Instant::now(),
            heap_used_bytes: 0,
            heap_limit_bytes: 0,
            state: IsolateState::Idle,
            op_metrics,
            handler_loaded: false,
            entry_specifier,
        })
    }

    /// Execute a request in the warm isolate
    async fn execute_in_isolate(
        &mut self,
        key: &HandlerKey,
        request: &WorkerRequest,
    ) -> (ExecutionOutcome, ExecutionProfile) {
        // Get mutable reference to isolate
        let use_code_cache = self.config.enable_code_cache;
        let (isolates, code_cache) = (&mut self.isolates, &mut self.code_cache);
        let isolate = isolates
            .get_mut(key)
            .ok_or_else(|| "Isolate not found".to_string());

        let isolate = match isolate {
            Ok(isolate) => isolate,
            Err(err) => return (ExecutionOutcome::Err(err), ExecutionProfile::empty()),
        };

        isolate.active_requests = 1;
        isolate.state = IsolateState::Executing {
            request_id: request.request_id.clone(),
            started_at: Instant::now(),
        };

        // Bootstrap on first use (load Web APIs polyfills if needed)
        if !isolate.bootstrapped {
            let bootstrap_start = Instant::now();

            // Basic Web API polyfills
            const BOOTSTRAP: &str = r#"
                // Basic console implementation
                if (typeof globalThis.console === 'undefined') {
                    globalThis.console = {
                        log(...args) { Deno.core.print(args.join(' ') + '\n'); },
                        error(...args) { Deno.core.print('[ERROR] ' + args.join(' ') + '\n'); },
                        warn(...args) { Deno.core.print('[WARN] ' + args.join(' ') + '\n'); },
                        info(...args) { Deno.core.print('[INFO] ' + args.join(' ') + '\n'); },
                        debug(...args) { Deno.core.print('[DEBUG] ' + args.join(' ') + '\n'); },
                    };
                }

                if (typeof globalThis.__dekaPrint !== 'function') {
                    globalThis.__dekaPrint = (value, isErr = false) => {
                        const text = value == null ? '' : String(value);
                        Deno.core.print(text, !!isErr);
                    };
                }

                if (!globalThis.TextEncoder) {
                    globalThis.TextEncoder = class TextEncoder {
                        encode(input) {
                            const str = String(input);
                            const utf8 = [];
                            for (let i = 0; i < str.length; i++) {
                                let charCode = str.charCodeAt(i);
                                if (charCode < 0x80) {
                                    utf8.push(charCode);
                                } else if (charCode < 0x800) {
                                    utf8.push(0xc0 | (charCode >> 6), 0x80 | (charCode & 0x3f));
                                } else if (charCode < 0xd800 || charCode >= 0xe000) {
                                    utf8.push(0xe0 | (charCode >> 12), 0x80 | ((charCode >> 6) & 0x3f), 0x80 | (charCode & 0x3f));
                                } else {
                                    i++;
                                    charCode = 0x10000 + (((charCode & 0x3ff) << 10) | (str.charCodeAt(i) & 0x3ff));
                                    utf8.push(
                                        0xf0 | (charCode >> 18),
                                        0x80 | ((charCode >> 12) & 0x3f),
                                        0x80 | ((charCode >> 6) & 0x3f),
                                        0x80 | (charCode & 0x3f)
                                    );
                                }
                            }
                            return new Uint8Array(utf8);
                        }
                    };
                }

                if (!globalThis.TextDecoder) {
                    globalThis.TextDecoder = class TextDecoder {
                        decode(bytes) {
                            if (!bytes) return '';
                            const arr = new Uint8Array(bytes);
                            let str = '';
                            let i = 0;
                            while (i < arr.length) {
                                let byte = arr[i++];
                                if (byte < 0x80) {
                                    str += String.fromCharCode(byte);
                                } else if (byte < 0xe0) {
                                    str += String.fromCharCode(((byte & 0x1f) << 6) | (arr[i++] & 0x3f));
                                } else if (byte < 0xf0) {
                                    str += String.fromCharCode(
                                        ((byte & 0x0f) << 12) | ((arr[i++] & 0x3f) << 6) | (arr[i++] & 0x3f)
                                    );
                                } else {
                                    const code =
                                        ((byte & 0x07) << 18) |
                                        ((arr[i++] & 0x3f) << 12) |
                                        ((arr[i++] & 0x3f) << 6) |
                                        (arr[i++] & 0x3f);
                                    const high = ((code - 0x10000) >> 10) | 0xd800;
                                    const low = ((code - 0x10000) & 0x3ff) | 0xdc00;
                                    str += String.fromCharCode(high, low);
                                }
                            }
                            return str;
                        }
                    };
                }

                // Performance API polyfill
                if (typeof globalThis.performance === 'undefined') {
                    const startTime = Date.now();
                    globalThis.performance = {
                        now() {
                            return Date.now() - startTime;
                        }
                    };
                }

                // Minimal URL polyfill for parsing URLs
                if (typeof globalThis.URL === 'undefined') {
                    globalThis.URL = class URL {
                        constructor(url) {
                            this.href = url;

                            // Parse protocol
                            const protocolMatch = url.match(/^([a-z][a-z0-9+.-]*):\/\//i);
                            this.protocol = protocolMatch ? protocolMatch[1] + ':' : '';

                            // Remove protocol
                            let remaining = protocolMatch ? url.slice(protocolMatch[0].length) : url;

                            // Remove hostname/port (everything before first / or ?, or end of string)
                            const hostMatch = remaining.match(/^([^\/\\?#]*)/);
                            this.host = hostMatch ? hostMatch[1] : '';
                            remaining = remaining.slice(this.host.length);

                            // If nothing left after host, pathname is '/'
                            if (!remaining) {
                                this.pathname = '/';
                                this.search = '';
                                this.hash = '';
                                return;
                            }

                            // Extract pathname, search, and hash
                            const pathMatch = remaining.match(/^([^?#]*)(\\?[^#]*)?(#.*)?$/);
                            if (pathMatch) {
                                this.pathname = pathMatch[1] || '/';
                                this.search = pathMatch[2] || '';
                                this.hash = pathMatch[3] || '';
                            } else {
                                this.pathname = '/';
                                this.search = '';
                                this.hash = '';
                            }
                        }
                    };
                }

                // Runtime bridge helpers for PHPX stdlib (JS runtime path)
                if (typeof globalThis.function_exists !== 'function') {
                    globalThis.function_exists = function(name) {
                        return typeof globalThis[name] === 'function';
                    };
                }

                if (typeof globalThis.is_array !== 'function') {
                    globalThis.is_array = function(value) {
                        return Array.isArray(value);
                    };
                }
                if (typeof globalThis.is_string !== 'function') {
                    globalThis.is_string = function(value) {
                        return typeof value === 'string';
                    };
                }
                if (typeof globalThis.is_int !== 'function') {
                    globalThis.is_int = function(value) {
                        return typeof value === 'number' && Number.isInteger(value);
                    };
                }
                if (typeof globalThis.is_float !== 'function') {
                    globalThis.is_float = function(value) {
                        return typeof value === 'number' && !Number.isNaN(value) && !Number.isInteger(value);
                    };
                }
                if (typeof globalThis.is_bool !== 'function') {
                    globalThis.is_bool = function(value) {
                        return typeof value === 'boolean';
                    };
                }
                if (typeof globalThis.is_object !== 'function') {
                    globalThis.is_object = function(value) {
                        return value !== null && typeof value === 'object' && !Array.isArray(value);
                    };
                }
                if (typeof globalThis.is_numeric !== 'function') {
                    globalThis.is_numeric = function(value) {
                        if (typeof value === 'number') {
                            return !Number.isNaN(value) && Number.isFinite(value);
                        }
                        if (typeof value === 'string') {
                            if (value.trim() === '') return false;
                            const num = Number(value);
                            return !Number.isNaN(num) && Number.isFinite(num);
                        }
                        return false;
                    };
                }
                if (typeof globalThis.is_callable !== 'function') {
                    globalThis.is_callable = function(value) {
                        return typeof value === 'function';
                    };
                }
                if (typeof globalThis.gettype !== 'function') {
                    globalThis.gettype = function(value) {
                        if (value === null || value === undefined) return 'NULL';
                        if (Array.isArray(value)) return 'array';
                        const t = typeof value;
                        if (t === 'string') return 'string';
                        if (t === 'boolean') return 'boolean';
                        if (t === 'number') return Number.isInteger(value) ? 'integer' : 'double';
                        if (t === 'object') return 'object';
                        if (t === 'function') return 'object';
                        return 'unknown';
                    };
                }

                if (typeof globalThis.__bridge !== 'function') {
                    const ops = (Deno && Deno.core && Deno.core.ops) ? Deno.core.ops : {};
                    const routeHostCall = (kind, action, payload) => {
                        if (kind === 'db') {
                            if (typeof ops.op_php_db_call_proto === 'function' && typeof ops.op_php_db_proto_encode === 'function' && typeof ops.op_php_db_proto_decode === 'function') {
                                const request = ops.op_php_db_proto_encode(String(action || ''), payload || {});
                                const response = ops.op_php_db_call_proto(request);
                                return ops.op_php_db_proto_decode(response);
                            }
                            return { ok: false, error: 'db protobuf bridge ops unavailable' };
                        }
                        if (kind === 'neo4j') {
                            if (typeof ops.op_neo4j_call === 'function') {
                                return ops.op_neo4j_call(String(action || ''), payload || {});
                            }
                            return { ok: false, error: 'neo4j bridge op unavailable' };
                        }
                        if (kind === 'redis') {
                            if (typeof ops.op_redis_call === 'function') {
                                const shopId = globalThis.__shopId;
                                const p = payload || {};
                                // Auto-prefix Redis keys with tenant ID (transparent to PHPX code)
                                if (shopId && p.key && action !== 'connect' && action !== 'close' && action !== 'flush' && action !== 'keys') {
                                    p.key = shopId + ':' + p.key;
                                }
                                // For 'keys' action, prefix the pattern
                                if (shopId && action === 'keys' && p.pattern) {
                                    p.pattern = shopId + ':' + p.pattern;
                                }
                                return ops.op_redis_call(String(action || ''), p);
                            }
                            return { ok: false, error: 'redis bridge op unavailable' };
                        }
                        if (kind === 'net') {
                            if (typeof ops.op_php_net_call_proto === 'function' && typeof ops.op_php_net_proto_encode === 'function' && typeof ops.op_php_net_proto_decode === 'function') {
                                const request = ops.op_php_net_proto_encode(String(action || ''), payload || {});
                                const response = ops.op_php_net_call_proto(request);
                                return ops.op_php_net_proto_decode(response);
                            }
                            return { ok: false, error: 'net protobuf bridge ops unavailable' };
                        }
                        if (kind === 'fs') {
                            if (typeof ops.op_php_fs_call_proto === 'function' && typeof ops.op_php_fs_proto_encode === 'function' && typeof ops.op_php_fs_proto_decode === 'function') {
                                const request = ops.op_php_fs_proto_encode(String(action || ''), payload || {});
                                const response = ops.op_php_fs_call_proto(request);
                                const decoded = ops.op_php_fs_proto_decode(response);
                                return Object.entries(decoded || {});
                            }
                            return { ok: false, error: 'fs protobuf bridge ops unavailable' };
                        }
                        if (kind === 'time') {
                            const act = String(action || '');
                            const req = payload || {};
                            if (act === 'now_ms') {
                                return Object.entries({ ok: true, now_ms: Date.now() });
                            }
                            if (act === 'sleep_ms') {
                                const msRaw = Number(req.milliseconds ?? req.ms ?? 0);
                                const ms = Number.isFinite(msRaw) ? Math.max(0, Math.floor(msRaw)) : 0;
                                try {
                                    if (ms > 0) {
                                        if (typeof SharedArrayBuffer !== 'undefined' && typeof Atomics !== 'undefined' && typeof Atomics.wait === 'function') {
                                            const sab = new SharedArrayBuffer(4);
                                            const arr = new Int32Array(sab);
                                            Atomics.wait(arr, 0, 0, ms);
                                        } else {
                                            const end = Date.now() + ms;
                                            while (Date.now() < end) {}
                                        }
                                    }
                                    return Object.entries({ ok: true, slept_ms: ms });
                                } catch (err) {
                                    return Object.entries({ ok: false, error: err && err.message ? err.message : String(err) });
                                }
                            }
                            return { ok: false, error: `unknown time action '${act}'` };
                        }
                        if (kind === 'crypto') {
                            const act = String(action || '');
                            if (act === 'random_bytes') {
                                const req = payload || {};
                                const n = Number(req.length ?? req.len ?? 0);
                                if (!Number.isFinite(n) || n <= 0) {
                                    return { ok: false, error: 'length must be > 0' };
                                }
                                const bytes = new Uint8Array(Math.floor(n));
                                let filled = false;
                                if (!filled && typeof ops.op_php_random_bytes === 'function') {
                                    const raw = ops.op_php_random_bytes(Math.floor(n));
                                    if (raw && typeof raw.length === 'number') {
                                        bytes.set(raw);
                                        filled = true;
                                    }
                                }
                                if (!filled && globalThis.crypto && typeof globalThis.crypto.getRandomValues === 'function') {
                                    globalThis.crypto.getRandomValues(bytes);
                                    filled = true;
                                }
                                if (!filled) {
                                    return { ok: false, error: 'secure random source unavailable' };
                                }
                                return Object.entries({ ok: true, data: Array.from(bytes) });
                            }
                            return { ok: false, error: `unknown crypto action '${act}'` };
                        }
                        if (kind === 'json') {
                            const act = String(action || '');
                            const req = payload || {};
                            if (act === 'encode') {
                                try {
                                    return Object.entries({ ok: true, json: JSON.stringify(req.value ?? null) });
                                } catch (err) {
                                    return Object.entries({ ok: false, error: err && err.message ? err.message : String(err) });
                                }
                            }
                            if (act === 'decode') {
                                try {
                                    const src = String(req.json ?? '');
                                    return Object.entries({ ok: true, value: JSON.parse(src) });
                                } catch (err) {
                                    return Object.entries({ ok: false, error: err && err.message ? err.message : String(err) });
                                }
                            }
                            if (act === 'validate') {
                                try {
                                    const src = String(req.json ?? '');
                                    JSON.parse(src);
                                    return Object.entries({ ok: true, valid: true });
                                } catch (_err) {
                                    return Object.entries({ ok: true, valid: false });
                                }
                            }
                            return Object.entries({ ok: false, error: `unknown json action '${act}'` });
                        }
                        return { ok: false, error: `unknown bridge kind '${kind}'` };
                    };

                    globalThis.__bridge = (kind, action, payload) => {
                        try {
                            return routeHostCall(String(kind || ''), String(action || ''), payload || {});
                        } catch (err) {
                            return { ok: false, error: err && err.message ? String(err.message) : String(err) };
                        }
                    };
                    globalThis.__bridge_async = async (kind, action, payload) => {
                        try {
                            return routeHostCall(String(kind || ''), String(action || ''), payload || {});
                        } catch (err) {
                            return { ok: false, error: err && err.message ? String(err.message) : String(err) };
                        }
                    };
                    globalThis.__deka_wasm_call = (moduleId, exportName, payload) => {
                        const name = String(moduleId || '');
                        if (name.startsWith('__deka_')) {
                            const kind = name.replace(/^__deka_/, '');
                            return routeHostCall(kind, exportName, payload || {});
                        }
                        return { ok: false, error: `unknown host bridge module '${name}'` };
                    };
                    globalThis.__deka_wasm_call_async = async (moduleId, exportName, payload) => {
                        const name = String(moduleId || '');
                        if (name.startsWith('__deka_')) {
                            const kind = name.replace(/^__deka_/, '');
                            return routeHostCall(kind, exportName, payload || {});
                        }
                        return { ok: false, error: `unknown host bridge module '${name}'` };
                    };
                }

                if (typeof globalThis.__dekaRuntime !== 'object') {
                    globalThis.__dekaRuntime = {
                        executePhpx: async function(_source, file, _props) {
                            if (!(globalThis.__dekaPhp && typeof globalThis.__dekaPhp.runFile === 'function')) {
                                throw new Error('runtime.executePhpx requires __dekaPhp.runFile');
                            }
                            const result = await globalThis.__dekaPhp.runFile(String(file || ''));
                            const stdout = result && result.stdout ? String(result.stdout) : "";
                            let stderr = result && result.stderr ? String(result.stderr) : "";
                            if (!stderr && result && result.error) {
                                stderr = String(result.error);
                            }
                            if (stdout) Deno.core.print(stdout, false);
                            if (stderr) Deno.core.print(stderr, true);
                            const ok = result && result.ok !== false;
                            let exitCode = result && typeof result.exit_code === 'number' ? result.exit_code : 0;
                            if (!ok && exitCode === 0) exitCode = 1;
                            if (exitCode) globalThis.__dekaExitCode = exitCode;
                            return result;
                        }
                    };
                }

                if (typeof globalThis.__dekaExecuteRequest !== 'function') {
                    globalThis.__dekaExecuteRequest = async function() {
                        function base64Encode(bytes) {
                            if (typeof btoa === "function") {
                                let binary = "";
                                for (let i = 0; i < bytes.length; i += 1) {
                                    binary += String.fromCharCode(bytes[i]);
                                }
                                return btoa(binary);
                            }
                            const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
                            let output = "";
                            for (let i = 0; i < bytes.length; i += 3) {
                                const a = bytes[i];
                                const b = i + 1 < bytes.length ? bytes[i + 1] : 0;
                                const c = i + 2 < bytes.length ? bytes[i + 2] : 0;
                                const triple = (a << 16) | (b << 8) | c;
                                output += alphabet[(triple >> 18) & 63];
                                output += alphabet[(triple >> 12) & 63];
                                output += i + 1 < bytes.length ? alphabet[(triple >> 6) & 63] : "=";
                                output += i + 2 < bytes.length ? alphabet[triple & 63] : "=";
                            }
                            return output;
                        }

                        const requestData = globalThis.__requestData || {};
                        requestData.__body = requestData.body ?? "";
                        requestData.params = requestData.params || {};
                        if (typeof requestData.json !== "function") {
                            requestData.json = async function() {
                                const body = this.__body || "";
                                if (!body) return null;
                                return JSON.parse(body);
                            };
                        }
                        if (typeof requestData.text !== "function") {
                            requestData.text = async function() {
                                return this.__body || "";
                            };
                        }
                        const context = globalThis.__requestContext || requestData.context || null;
                        const handler = globalThis.app;

                        if (!handler) {
                            throw new Error('Handler did not define "app" variable');
                        }

                        const wsEvent = requestData.__dekaWsEvent;
                        if (wsEvent) {
                            const wsHandler = handler.websocket || globalThis.__dekaWebsocket;
                            if (wsHandler) {
                                const ws = globalThis.__dekaWsCreate
                                    ? globalThis.__dekaWsCreate(requestData.__dekaWsId, requestData.__dekaWsData)
                                    : null;
                                if (wsEvent === "message" && requestData.__dekaWsBinary && Array.isArray(requestData.__dekaWsMessage)) {
                                    requestData.__dekaWsMessage = new Uint8Array(requestData.__dekaWsMessage);
                                }

                                if (wsEvent === "open" && typeof wsHandler.open === "function") {
                                    wsHandler.open(ws);
                                } else if (wsEvent === "message" && typeof wsHandler.message === "function") {
                                    wsHandler.message(ws, requestData.__dekaWsMessage);
                                } else if (wsEvent === "close" && typeof wsHandler.close === "function") {
                                    wsHandler.close(ws, requestData.__dekaWsCode, requestData.__dekaWsReason);
                                } else if (wsEvent === "drain" && typeof wsHandler.drain === "function") {
                                    wsHandler.drain(ws);
                                }
                            }

                            return { status: 204, headers: {}, body: "" };
                        }

                        let response;
                        if (typeof handler.fetch === "function") {
                            response = await handler.fetch(requestData, context);
                        } else if (typeof handler === "function") {
                            response = await handler(requestData, context);
                        } else {
                            throw new Error('Handler is not callable');
                        }

                        const normalized = globalThis.__dekaResponse || (globalThis.__dekaResponse = {
                            status: 200,
                            headers: {},
                            body: "",
                            body_base64: undefined,
                            upgrade: undefined,
                        });
                        normalized.status = 200;
                        normalized.body = "";
                        normalized.body_base64 = undefined;
                        normalized.upgrade = undefined;
                        const headerTarget = normalized.headers;
                        for (const key in headerTarget) {
                            delete headerTarget[key];
                        }

                        const applyHeaders = (headers) => {
                            if (!headers) return;
                            if (typeof headers.forEach === "function") {
                                headers.forEach((value, key) => {
                                    headerTarget[key] = String(value);
                                });
                                return;
                            }
                            for (const key in headers) {
                                headerTarget[key] = String(headers[key]);
                            }
                        };

                        if (response && typeof response.text === "function") {
                            if (typeof response.status === "number") {
                                normalized.status = response.status;
                            }
                            applyHeaders(response.headers);
                            if (response.upgrade) {
                                normalized.upgrade = response.upgrade;
                            }
                            const bodyValue = response.body;
                            if (bodyValue instanceof Uint8Array) {
                                normalized.body_base64 = base64Encode(bodyValue);
                            } else if (bodyValue instanceof ArrayBuffer) {
                                normalized.body_base64 = base64Encode(new Uint8Array(bodyValue));
                            } else {
                                const contentType = String(headerTarget["content-type"] || headerTarget["Content-Type"] || "").toLowerCase();
                                const isTextLike = contentType.startsWith("text/")
                                    || contentType.includes("json")
                                    || contentType.includes("javascript")
                                    || contentType.includes("xml")
                                    || contentType.includes("svg")
                                    || contentType.includes("x-www-form-urlencoded");
                                if (!isTextLike && typeof response.arrayBuffer === "function") {
                                    const bytes = new Uint8Array(await response.arrayBuffer());
                                    normalized.body_base64 = base64Encode(bytes);
                                } else {
                                    normalized.body = await response.text();
                                }
                            }
                        } else if (response && typeof response === "object") {
                            if (typeof response.status === "number") {
                                normalized.status = response.status;
                            }
                            applyHeaders(response.headers);
                            if (typeof response.body_base64 === "string") {
                                normalized.body_base64 = response.body_base64;
                            }
                            if (response.body != null) {
                                if (response.body instanceof Uint8Array) {
                                    normalized.body_base64 = base64Encode(response.body);
                                } else if (response.body instanceof ArrayBuffer) {
                                    normalized.body_base64 = base64Encode(new Uint8Array(response.body));
                                } else if (typeof response.body === "string") {
                                    normalized.body = response.body;
                                } else {
                                    const bodyObj = response.body;
                                    if (bodyObj && typeof bodyObj === "object") {
                                        const keys = Object.keys(bodyObj);
                                        if (keys.length > 0 && keys.every((k) => /^\d+$/.test(k))) {
                                            const bytes = keys
                                                .sort((a, b) => Number(a) - Number(b))
                                                .map((k) => Number(bodyObj[k]) || 0);
                                            normalized.body_base64 = base64Encode(new Uint8Array(bytes));
                                        } else {
                                            normalized.body = JSON.stringify(bodyObj);
                                        }
                                    } else {
                                        normalized.body = JSON.stringify(response.body);
                                    }
                                }
                            }
                            if (response.upgrade) {
                                normalized.upgrade = response.upgrade;
                            }
                        } else if (response != null) {
                            normalized.body = String(response);
                        }

                        return normalized;
                    };
                }

                // The deka/router module is already loaded as an extension
                // and exposes itself as globalThis.__dekaRouter automatically
            "#;

            if let Err(err) = isolate.runtime.execute_script(
                "bootstrap.js",
                ModuleCodeString::from(BOOTSTRAP.to_string()),
            ) {
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                return (
                    ExecutionOutcome::Err(format!("Bootstrap failed: {}", err)),
                    ExecutionProfile::empty(),
                );
            }

            isolate.bootstrapped = true;
            tracing::debug!(
                "Worker {} bootstrapped {} in {:?}",
                self.worker_id,
                key.name,
                bootstrap_start.elapsed()
            );
        }

        if handler_is_unsupported_script(&key.name) {
            isolate.active_requests = 0;
            isolate.state = IsolateState::Idle;
            return (
                ExecutionOutcome::Err(
                    "JavaScript/TypeScript handlers are not supported in reboot MVP. Use .php/.phpx handlers or serve JS/TS files as static assets.".to_string(),
                ),
                ExecutionProfile::empty(),
            );
        }

        let use_esm = request.request_data.handler_entry.is_some()
            && std::env::var("DEKA_RUNTIME_ESM")
                .map(|value| value != "0" && value != "false")
                .unwrap_or(true);

        let wrapped_handler_code = if !use_esm {
            // Execute the handler - transform import/export statements
            // Replace ES6 import with global access
            let handler_code = request
                .request_data
                .handler_code
                .replace(
                    "import { Router, cors, logger, prettyJSON } from 'deka/router'",
                    "const { Router, cors, logger, prettyJSON } = globalThis.__dekaRouter;",
                )
                .replace(
                    "import { Router, cors, logger, prettyJSON } from \"deka/router\"",
                    "const { Router, cors, logger, prettyJSON } = globalThis.__dekaRouter;",
                )
                .replace(
                    "import { Router } from 'deka/router'",
                    "const { Router } = globalThis.__dekaRouter;",
                )
                .replace(
                    "import { Router } from \"deka/router\"",
                    "const { Router } = globalThis.__dekaRouter;",
                )
                .replace(
                    "import { Database, Statement } from 'deka/sqlite'",
                    "const { Database, Statement } = globalThis.__dekaSqlite;",
                )
                .replace(
                    "import { Database, Statement } from \"deka/sqlite\"",
                    "const { Database, Statement } = globalThis.__dekaSqlite;",
                )
                .replace(
                    "import { Database } from 'deka/sqlite'",
                    "const { Database } = globalThis.__dekaSqlite;",
                )
                .replace(
                    "import { Database } from \"deka/sqlite\"",
                    "const { Database } = globalThis.__dekaSqlite;",
                )
                .replace(
                    "import { t4, T4Client, T4File, write } from 'deka/t4'",
                    "const { t4, T4Client, T4File, write } = globalThis.__dekaT4;",
                )
                .replace(
                    "import { t4, T4Client, T4File, write } from \"deka/t4\"",
                    "const { t4, T4Client, T4File, write } = globalThis.__dekaT4;",
                )
                .replace(
                    "import { t4 } from 'deka/t4'",
                    "const { t4 } = globalThis.__dekaT4;",
                )
                .replace(
                    "import { t4 } from \"deka/t4\"",
                    "const { t4 } = globalThis.__dekaT4;",
                )
                .replace(
                    "import { Mesh, IsolatePool, Isolate, serve } from 'deka'",
                    "const { Mesh, IsolatePool, Isolate, serve } = globalThis.__deka;",
                )
                .replace(
                    "import { Mesh, IsolatePool, Isolate, serve } from \"deka\"",
                    "const { Mesh, IsolatePool, Isolate, serve } = globalThis.__deka;",
                )
                // Remove export default statement - we'll capture 'app' variable directly
                .replace("export default app", "// export default app")
                .replace("export default ", "const __dekaDefault = ");

            let wrapped = format!(
                "(function() {{\n{}\nif (typeof globalThis.app === 'undefined') {{ if (typeof __dekaDefault !== 'undefined') {{ if (typeof __dekaDefault === 'function' && typeof globalThis.__dekaNodeExpressAdapter === 'function' && (typeof __dekaDefault.handle === 'function' || typeof __dekaDefault.listen === 'function')) {{ globalThis.app = globalThis.__dekaNodeExpressAdapter(__dekaDefault); }} else if (__dekaDefault && typeof __dekaDefault === 'object' && !__dekaDefault.__dekaServer && (typeof __dekaDefault.fetch === 'function' || typeof __dekaDefault.routes === 'object')) {{ globalThis.app = globalThis.__deka.serve(__dekaDefault); }} else {{ globalThis.app = __dekaDefault; }} }} else if (typeof app !== 'undefined') {{ if (typeof app === 'function' && typeof globalThis.__dekaNodeExpressAdapter === 'function' && (typeof app.handle === 'function' || typeof app.listen === 'function')) {{ globalThis.app = globalThis.__dekaNodeExpressAdapter(app); }} else {{ globalThis.app = app; }} }} }}\n}})();",
                handler_code
            );

            let setup_code = "globalThis.app = undefined; globalThis.Deka = globalThis.Deka || {};";
            if let Err(err) = isolate
                .runtime
                .execute_script("setup.js", ModuleCodeString::from(setup_code.to_string()))
            {
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                return (
                    ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                    ExecutionProfile::empty(),
                );
            }

            Some(wrapped)
        } else {
            None
        };

        if let Err(err) = set_request_globals(
            &mut isolate.runtime,
            &request.request_data.request_value,
            request.request_data.request_parts.as_ref(),
            &self.deka_args,
        ) {
            isolate.active_requests = 0;
            isolate.state = IsolateState::Idle;
            return (
                ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                ExecutionProfile::empty(),
            );
        }

        let exec_mode = match request.request_data.mode {
            ExecutionMode::Module => "module",
            _ => "request",
        };
        if let Err(err) = isolate.runtime.execute_script(
            "exec_mode.js",
            ModuleCodeString::from(format!(
                "globalThis.__dekaExecMode = {};",
                serde_json::to_string(exec_mode).unwrap_or_else(|_| "\"request\"".to_string())
            )),
        ) {
            isolate.active_requests = 0;
            isolate.state = IsolateState::Idle;
            return (
                ExecutionOutcome::Err(format!("Setup failed: {}", err)),
                ExecutionProfile::empty(),
            );
        }

        if let Some(wrapped_handler_code) = wrapped_handler_code.as_ref() {
            if use_code_cache {
                let source_hash = Self::hash_source(&request.request_data.handler_code);
                if let Err(err) = Self::compile_handler(
                    &mut isolate.runtime,
                    code_cache,
                    source_hash,
                    wrapped_handler_code,
                ) {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    let formatted = validation::format_runtime_syntax_error(
                        &err,
                        &request.request_data.handler_code,
                        &key.name,
                    );
                    return (
                        ExecutionOutcome::Err(formatted.unwrap_or(err)),
                        ExecutionProfile::empty(),
                    );
                }
            } else if let Err(err) = isolate
                .runtime
                .execute_script("handler.js", ModuleCodeString::from(wrapped_handler_code.to_string()))
            {
                let raw = err.to_string();
                if parse_exit_code(&raw).is_none() {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    let formatted = validation::format_runtime_syntax_error(
                        &raw,
                        &request.request_data.handler_code,
                        &key.name,
                    );
                    return (
                        ExecutionOutcome::Err(
                            formatted
                                .unwrap_or_else(|| format!("Handler execution failed: {}", err)),
                        ),
                        ExecutionProfile::empty(),
                    );
                }
            }
        } else if use_esm {
            if !isolate.handler_loaded {
                let spec = match isolate.entry_specifier.as_ref() {
                    Some(spec) => spec,
                    None => {
                        isolate.active_requests = 0;
                        isolate.state = IsolateState::Idle;
                        return (
                            ExecutionOutcome::Err("missing module entry specifier".to_string()),
                            ExecutionProfile::empty(),
                        );
                    }
                };
                let module_id = match isolate.runtime.load_main_es_module(spec).await {
                    Ok(id) => id,
                    Err(err) => {
                        isolate.active_requests = 0;
                        isolate.state = IsolateState::Idle;
                        return (
                            ExecutionOutcome::Err(format!("Failed to load module: {}", err)),
                            ExecutionProfile::empty(),
                        );
                    }
                };
                let eval = isolate.runtime.mod_evaluate(module_id);
                if let Err(err) = isolate
                    .runtime
                    .run_event_loop(deno_core::PollEventLoopOptions::default())
                    .await
                {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Err(format!("Module event loop failed: {}", err)),
                        ExecutionProfile::empty(),
                    );
                }
                if let Err(err) = eval.await {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Err(format!("Module evaluation failed: {}", err)),
                        ExecutionProfile::empty(),
                    );
                }
                isolate.handler_loaded = true;
            }
        }

        if request.request_data.mode == ExecutionMode::Module {
            let exit_value = isolate.runtime.execute_script(
                "handler.js",
                ModuleCodeString::from(
                    "const __code = globalThis.__dekaExitCode; globalThis.__dekaExitCode = undefined; __code ?? null".to_string(),
                ),
            );
            if let Ok(value) = exit_value {
                deno_core::scope!(scope, &mut isolate.runtime);
                let local = deno_core::v8::Local::new(scope, &value);
                if let Ok(parsed) = serde_v8::from_v8::<serde_json::Value>(scope, local) {
                    if let Some(code) = parsed.as_i64() {
                        isolate.active_requests = 0;
                        isolate.state = IsolateState::Idle;
                        return (
                            ExecutionOutcome::Ok(serde_json::json!({ "exit_code": code })),
                            ExecutionProfile::empty(),
                        );
                    }
                }
            }
        }

        if !isolate.handler_loaded {
            isolate.handler_loaded = true;
            if std::env::var("DEKA_DEBUG").is_ok() {
                deka_stdio::log(
                    "handler",
                    &format!("loaded {} on worker {}", key.name, self.worker_id),
                );
            }
        }

        let heap_before_bytes = isolate
            .runtime
            .v8_isolate()
            .get_heap_statistics()
            .used_heap_size();

        // Track CPU time for this execution
        let cpu_start = get_thread_cpu_time();
        let timeout_ms = self.config.request_timeout_ms;
        let timeout_flag = Arc::new(AtomicUsize::new(0));
        let timeout_flag_handle = Arc::clone(&timeout_flag);
        let isolate_handle = isolate.runtime.v8_isolate().thread_safe_handle();

        let watchdog = if timeout_ms > 0 {
            Some(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
                timeout_flag_handle.store(1, Ordering::Relaxed);
                isolate_handle.terminate_execution();
            }))
        } else {
            None
        };

        let exec_start = Instant::now();
        let mut needs_event_loop = false;
        let result = if request.request_data.mode == ExecutionMode::Module {
            isolate
                .runtime
                .execute_script(
                    "handler.js",
                    ModuleCodeString::from("undefined".to_string()),
                )
                .map_err(|err| err.to_string())
        } else {
            // Execute the handler fetch using the globals
            const EXEC_CALL: &str = r#"globalThis.__dekaExecuteRequest()"#;
            let code = EXEC_CALL.to_string();
            isolate
                .runtime
                .execute_script("handler.js", ModuleCodeString::from(code))
                .map_err(|err| err.to_string())
        };

        let result = match result {
            Ok(value) => value,
            Err(err) => {
                if let Some(watchdog) = watchdog {
                    watchdog.abort();
                }
                if let Some(code) = parse_exit_code(&err) {
                    isolate.active_requests = 0;
                    isolate.state = IsolateState::Idle;
                    return (
                        ExecutionOutcome::Ok(serde_json::json!({ "exit_code": code })),
                        ExecutionProfile::empty(),
                    );
                }
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                let profile = finalize_profile(
                    heap_before_bytes,
                    isolate,
                    exec_start.elapsed().as_millis() as u64,
                    0,
                    0,
                );
                return (
                    ExecutionOutcome::Err(format!("Handler execution failed: {}", err)),
                    profile,
                );
            }
        };
        let exec_script_ms = exec_start.elapsed().as_millis() as u64;

        if matches!(
            request.request_data.mode,
            ExecutionMode::Request | ExecutionMode::Module
        ) {
            // Run event loop to complete async operations
            {
                deno_core::scope!(scope, &mut isolate.runtime);
                let local = deno_core::v8::Local::new(scope, &result);
                if let Ok(promise) = deno_core::v8::Local::<deno_core::v8::Promise>::try_from(local)
                {
                    match promise.state() {
                        deno_core::v8::PromiseState::Pending => {
                            needs_event_loop = true;
                        }
                        _ => needs_event_loop = false,
                    }
                }
            }
        }

        let event_loop_ms = if needs_event_loop {
            let event_start = Instant::now();
            if let Err(err) = isolate
                .runtime
                .run_event_loop(deno_core::PollEventLoopOptions::default())
                .await
            {
                if let Some(watchdog) = watchdog {
                    watchdog.abort();
                }
                isolate.active_requests = 0;
                isolate.state = IsolateState::Idle;
                let profile = finalize_profile(
                    heap_before_bytes,
                    isolate,
                    exec_script_ms,
                    event_start.elapsed().as_millis() as u64,
                    0,
                );
                return (
                    ExecutionOutcome::Err(format!("Event loop failed: {}", err)),
                    profile,
                );
            }
            event_start.elapsed().as_millis() as u64
        } else {
            0
        };

        if let Some(watchdog) = watchdog {
            watchdog.abort();
        }

        // Calculate CPU time consumed
        let cpu_elapsed = get_thread_cpu_time() - cpu_start;
        isolate.total_cpu_time += cpu_elapsed;

        update_heap_stats(isolate);

        // Get the result from the promise
        let decode_start = Instant::now();
        let outcome = if request.request_data.mode == ExecutionMode::Module {
            let exit_value = isolate.runtime.execute_script(
                "handler.js",
                ModuleCodeString::from(
                    "const __code = globalThis.__dekaExitCode; globalThis.__dekaExitCode = undefined; __code ?? null".to_string(),
                ),
            );
            if let Ok(value) = exit_value {
                deno_core::scope!(scope, &mut isolate.runtime);
                let local = deno_core::v8::Local::new(scope, &value);
                if let Ok(parsed) = serde_v8::from_v8::<serde_json::Value>(scope, local) {
                    if let Some(code) = parsed.as_i64() {
                        ExecutionOutcome::Ok(serde_json::json!({ "exit_code": code }))
                    } else {
                        ExecutionOutcome::Ok(serde_json::Value::Null)
                    }
                } else {
                    ExecutionOutcome::Ok(serde_json::Value::Null)
                }
            } else {
                ExecutionOutcome::Ok(serde_json::Value::Null)
            }
        } else {
            deno_core::scope!(scope, &mut isolate.runtime);
            let local = deno_core::v8::Local::new(scope, &result);

            let value_result: Result<deno_core::v8::Local<deno_core::v8::Value>, String> =
                if let Ok(promise) = deno_core::v8::Local::<deno_core::v8::Promise>::try_from(local)
                {
                    match promise.state() {
                        deno_core::v8::PromiseState::Fulfilled => Ok(promise.result(scope)),
                        deno_core::v8::PromiseState::Rejected => {
                            let reason = promise.result(scope);
                            Err(format!(
                                "Handler rejected: {}",
                                reason.to_rust_string_lossy(scope)
                            ))
                        }
                        deno_core::v8::PromiseState::Pending => {
                            Err("Handler promise still pending after event loop".to_string())
                        }
                    }
                } else {
                    Ok(local)
                };

            match value_result {
                Ok(value) => match serde_v8::from_v8::<serde_json::Value>(scope, value) {
                    Ok(value) => ExecutionOutcome::Ok(value),
                    Err(err) => ExecutionOutcome::Err(format!(
                        "Handler returned non-serializable result: {}",
                        err
                    )),
                },
                Err(err) => ExecutionOutcome::Err(err),
            }
        };

        isolate.active_requests = 0;
        isolate.state = IsolateState::Idle;
        let outcome = if timeout_flag.load(Ordering::Relaxed) == 1 {
            isolate.state = IsolateState::Stuck {
                request_id: request.request_id.clone(),
                started_at: Instant::now(),
                timeout_triggered: true,
            };
            ExecutionOutcome::TimedOut
        } else {
            outcome
        };

        let result_decode_ms = decode_start.elapsed().as_millis() as u64;
        let profile = finalize_profile(
            heap_before_bytes,
            isolate,
            exec_script_ms,
            event_loop_ms,
            result_decode_ms,
        );
        (outcome, profile)
    }

    fn compile_handler(
        runtime: &mut JsRuntime,
        code_cache: &mut HashMap<u64, Vec<u8>>,
        source_hash: u64,
        handler_code: &str,
    ) -> Result<(), String> {
        let mut fallback_to_execute = false;
        let mut should_write_cache = false;

        {
            deno_core::scope!(scope, runtime);

            let source_str = v8::String::new(scope, handler_code)
                .ok_or_else(|| "Failed to allocate handler source".to_string())?;
            let resource_name = v8::String::new(scope, "handler.js")
                .ok_or_else(|| "Failed to allocate handler name".to_string())?;
            let origin = v8::ScriptOrigin::new(
                scope,
                resource_name.into(),
                0,
                0,
                false,
                0,
                None,
                false,
                false,
                false,
                None,
            );

            let cached_bytes = code_cache.get(&source_hash).cloned();
            let cached_data = cached_bytes
                .as_ref()
                .map(|data| v8::script_compiler::CachedData::new(data));

            let mut source = if let Some(cached_data) = cached_data {
                v8::script_compiler::Source::new_with_cached_data(
                    source_str,
                    Some(&origin),
                    cached_data,
                )
            } else {
                v8::script_compiler::Source::new(source_str, Some(&origin))
            };

            let compiled = match v8::script_compiler::compile_unbound_script(
                scope,
                &mut source,
                if cached_bytes.is_some() {
                    v8::script_compiler::CompileOptions::ConsumeCodeCache
                } else {
                    v8::script_compiler::CompileOptions::NoCompileOptions
                },
                v8::script_compiler::NoCacheReason::NoReason,
            ) {
                Some(script) => Some(script),
                None => {
                    fallback_to_execute = true;
                    None
                }
            };

            if let Some(mut unbound_script) = compiled {
                if cached_bytes.is_some() {
                    if let Some(cached_data) = source.get_cached_data() {
                        if cached_data.rejected() {
                            code_cache.remove(&source_hash);

                            let source_str = v8::String::new(scope, handler_code)
                                .ok_or_else(|| "Failed to allocate handler source".to_string())?;
                            let mut retry_source =
                                v8::script_compiler::Source::new(source_str, Some(&origin));
                            unbound_script = v8::script_compiler::compile_unbound_script(
                                scope,
                                &mut retry_source,
                                v8::script_compiler::CompileOptions::NoCompileOptions,
                                v8::script_compiler::NoCacheReason::NoReason,
                            )
                            .ok_or_else(|| {
                                "Handler compile failed after cache rejection".to_string()
                            })?;
                        }
                    }
                } else {
                    should_write_cache = true;
                }

                let script = unbound_script.bind_to_current_context(scope);
                if script.run(scope).is_none() {
                    fallback_to_execute = true;
                } else if should_write_cache {
                    if let Some(new_cache) = unbound_script.create_code_cache() {
                        code_cache.insert(source_hash, new_cache.as_ref().to_vec());
                    }
                }
            }
        }

        if fallback_to_execute {
            runtime
                .execute_script(
                    "handler.js",
                    ModuleCodeString::from(handler_code.to_string()),
                )
                .map_err(|e| format!("Handler execution failed: {}", e))?;
        }

        Ok(())
    }

    /// Move key to back of LRU list (most recently used)
    fn touch_lru(&mut self, key: &HandlerKey) {
        if let Some(pos) = self.lru_order.iter().position(|k| k == key) {
            self.lru_order.remove(pos);
            self.lru_order.push(key.clone());
        }
    }

    /// Evict the least recently used isolate
    fn evict_lru(&mut self) {
        if let Some(oldest_key) = self.lru_order.first().cloned() {
            self.isolates.remove(&oldest_key);
            self.lru_order.remove(0);
            self.metrics.evictions.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(
                "Worker {} evicted isolate: {}",
                self.worker_id,
                oldest_key.name
            );
        }
    }

    /// Hash handler source for cache invalidation
    fn hash_source(source: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        hasher.finish()
    }
}

fn set_request_globals(
    runtime: &mut JsRuntime,
    request: &serde_json::Value,
    request_parts: Option<&RequestParts>,
    deka_args: &serde_json::Value,
) -> Result<(), String> {
    deno_core::scope!(scope, runtime);
    let context = scope.get_current_context();
    let global = context.global(scope);
    let handler_path = std::env::var("HANDLER_PATH").unwrap_or_default();
    let cwd = std::env::current_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    if let Some(parts) = request_parts {
        let (request_uri, request_pathname) = split_request_url(&parts.url);
        let obj = v8::Object::new(scope);

        let url_key = v8::String::new(scope, "url").ok_or_else(|| "url key".to_string())?;
        let url_val = v8::String::new(scope, &parts.url).ok_or_else(|| "url val".to_string())?;
        obj.set(scope, url_key.into(), url_val.into());

        let path_key = v8::String::new(scope, "path").ok_or_else(|| "path key".to_string())?;
        let path_val =
            v8::String::new(scope, &request_uri).ok_or_else(|| "path val".to_string())?;
        obj.set(scope, path_key.into(), path_val.into());

        let pathname_key =
            v8::String::new(scope, "pathname").ok_or_else(|| "pathname key".to_string())?;
        let pathname_val = v8::String::new(scope, &request_pathname)
            .ok_or_else(|| "pathname val".to_string())?;
        obj.set(scope, pathname_key.into(), pathname_val.into());

        let method_key =
            v8::String::new(scope, "method").ok_or_else(|| "method key".to_string())?;
        let method_val =
            v8::String::new(scope, &parts.method).ok_or_else(|| "method val".to_string())?;
        obj.set(scope, method_key.into(), method_val.into());

        let headers_key =
            v8::String::new(scope, "headers").ok_or_else(|| "headers key".to_string())?;
        let headers_obj = v8::Object::new(scope);
        for (key, value) in &parts.headers {
            let k = v8::String::new(scope, key).ok_or_else(|| "header key".to_string())?;
            let v = v8::String::new(scope, value).ok_or_else(|| "header val".to_string())?;
            headers_obj.set(scope, k.into(), v.into());
        }
        obj.set(scope, headers_key.into(), headers_obj.into());

        let body_key = v8::String::new(scope, "body").ok_or_else(|| "body key".to_string())?;
        let body_val = match &parts.body {
            Some(body) => v8::String::new(scope, body)
                .ok_or_else(|| "body val".to_string())?
                .into(),
            None => v8::null(scope).into(),
        };
        obj.set(scope, body_key.into(), body_val);

        let request_key = v8::String::new(scope, "__requestData")
            .ok_or_else(|| "request data key".to_string())?;
        global.set(scope, request_key.into(), obj.into());

        let server = v8::Object::new(scope);
        let request_uri_key =
            v8::String::new(scope, "REQUEST_URI").ok_or_else(|| "request uri key".to_string())?;
        let request_uri_val = v8::String::new(scope, &request_uri)
            .ok_or_else(|| "request uri val".to_string())?;
        server.set(scope, request_uri_key.into(), request_uri_val.into());

        let path_info_key =
            v8::String::new(scope, "PATH_INFO").ok_or_else(|| "path info key".to_string())?;
        let path_info_val = v8::String::new(scope, &request_pathname)
            .ok_or_else(|| "path info val".to_string())?;
        server.set(scope, path_info_key.into(), path_info_val.into());

        let pwd_key = v8::String::new(scope, "PWD").ok_or_else(|| "pwd key".to_string())?;
        let pwd_val = v8::String::new(scope, &cwd).ok_or_else(|| "pwd val".to_string())?;
        server.set(scope, pwd_key.into(), pwd_val.into());

        if !handler_path.is_empty() {
            let script_key = v8::String::new(scope, "SCRIPT_FILENAME")
                .ok_or_else(|| "script filename key".to_string())?;
            let script_val = v8::String::new(scope, &handler_path)
                .ok_or_else(|| "script filename val".to_string())?;
            server.set(scope, script_key.into(), script_val.into());
        }

        let server_key = v8::String::new(scope, "_SERVER").ok_or_else(|| "_SERVER key".to_string())?;
        global.set(scope, server_key.into(), server.into());

        let get_key = v8::String::new(scope, "_GET").ok_or_else(|| "_GET key".to_string())?;
        global.set(scope, get_key.into(), v8::Object::new(scope).into());
    } else {
        let request_key = v8::String::new(scope, "__requestData")
            .ok_or_else(|| "request data key".to_string())?;
        let request_value = serde_v8::to_v8(scope, request)
            .map_err(|err| format!("request data to v8: {}", err))?;
        global.set(scope, request_key.into(), request_value);
    }

    let ctx_value = request
        .get("context")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let ctx_v8 = serde_v8::to_v8(scope, &ctx_value)
        .map_err(|err| format!("request context to v8: {}", err))?;
    let ctx_key = v8::String::new(scope, "__requestContext")
        .ok_or_else(|| "request context key".to_string())?;
    global.set(scope, ctx_key.into(), ctx_v8);

    let deka_key = v8::String::new(scope, "Deka").ok_or_else(|| "deka key".to_string())?;
    let deka_val = global.get(scope, deka_key.into());
    let deka_obj = if let Some(val) = deka_val {
        if val.is_object() {
            val.to_object(scope).unwrap()
        } else {
            let obj = v8::Object::new(scope);
            global.set(scope, deka_key.into(), obj.into());
            obj
        }
    } else {
        let obj = v8::Object::new(scope);
        global.set(scope, deka_key.into(), obj.into());
        obj
    };

    let args_key = v8::String::new(scope, "args").ok_or_else(|| "deka args key".to_string())?;
    let args_val =
        serde_v8::to_v8(scope, deka_args).map_err(|err| format!("deka args to v8: {}", err))?;
    deka_obj.set(scope, args_key.into(), args_val);

    // Tenant context: resolve shop_id from Host header and inject as globals
    if let Some(parts) = request_parts {
        let shop_id = crate::tenant::resolve_tenant_from_headers(&parts.headers)
            .unwrap_or_default();
        if !shop_id.is_empty() {
            // globalThis.__shopId — used by bridge layer for Redis prefixing
            let shop_id_key = v8::String::new(scope, "__shopId")
                .ok_or_else(|| "shop id key".to_string())?;
            let shop_id_val = v8::String::new(scope, &shop_id)
                .ok_or_else(|| "shop id val".to_string())?;
            global.set(scope, shop_id_key.into(), shop_id_val.into());

            // Also add to _SERVER for PHPX access as $_SERVER['SHOP_ID']
            if let Some(server_key) = v8::String::new(scope, "_SERVER") {
                if let Some(server_val) = global.get(scope, server_key.into()) {
                    if let Some(server_obj) = server_val.to_object(scope) {
                        if let Some(k) = v8::String::new(scope, "SHOP_ID") {
                            if let Some(v) = v8::String::new(scope, &shop_id) {
                                server_obj.set(scope, k.into(), v.into());
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn split_request_url(url: &str) -> (String, String) {
    let mut path = url.trim().to_string();
    if let Some(scheme_idx) = path.find("://") {
        let after_scheme = &path[(scheme_idx + 3)..];
        path = match after_scheme.find('/') {
            Some(slash_idx) => after_scheme[slash_idx..].to_string(),
            None => "/".to_string(),
        };
    }
    if path.is_empty() {
        path = "/".to_string();
    }
    let pathname = path
        .split('?')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("/")
        .to_string();
    (path, pathname)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn update_heap_stats(isolate: &mut WarmIsolate) -> usize {
    let heap_stats = isolate.runtime.v8_isolate().get_heap_statistics();
    isolate.heap_used_bytes = heap_stats.used_heap_size();
    isolate.heap_limit_bytes = heap_stats.heap_size_limit();
    isolate.heap_used_bytes
}

fn finalize_profile(
    heap_before_bytes: usize,
    isolate: &mut WarmIsolate,
    exec_script_ms: u64,
    event_loop_ms: u64,
    result_decode_ms: u64,
) -> ExecutionProfile {
    let heap_after_bytes = update_heap_stats(isolate);
    ExecutionProfile {
        heap_before_bytes,
        heap_after_bytes,
        exec_script_ms,
        event_loop_ms,
        result_decode_ms,
    }
}

fn handler_is_unsupported_script(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".ts")
        || lower.ends_with(".tsx")
        || lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".mjs")
        || lower.ends_with(".cjs")
}

#[cfg(test)]
mod tests {
    use super::split_request_url;

    #[test]
    fn split_request_url_handles_absolute_urls() {
        let (request_uri, pathname) = split_request_url("http://localhost:8530/users?id=1");
        assert_eq!(request_uri, "/users?id=1");
        assert_eq!(pathname, "/users");
    }

    #[test]
    fn split_request_url_handles_relative_paths() {
        let (request_uri, pathname) = split_request_url("/docs/getting-started?tab=init");
        assert_eq!(request_uri, "/docs/getting-started?tab=init");
        assert_eq!(pathname, "/docs/getting-started");
    }
}
