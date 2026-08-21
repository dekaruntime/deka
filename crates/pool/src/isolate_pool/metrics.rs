use super::*;

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
pub(super) struct WorkerLoad {
    pub(super) queued_requests: AtomicUsize,
    pub(super) active_requests: AtomicUsize,
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
    pub(super) fn from_isolate(
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
pub(super) struct OpTimingAccum {
    count: u64,
    total: Duration,
}

#[derive(Default)]
pub(super) struct OpTimingTracker {
    names: RefCell<Vec<String>>,
    totals: RefCell<Vec<OpTimingAccum>>,
    inflight: RefCell<Vec<VecDeque<Instant>>>,
}

#[derive(Clone)]
pub(super) struct OpTimingSnapshot {
    names: Vec<String>,
    totals: Vec<OpTimingAccum>,
}

pub(super) struct ExecutionProfile {
    pub(super) heap_before_bytes: usize,
    pub(super) heap_after_bytes: usize,
    pub(super) exec_script_ms: u64,
    pub(super) event_loop_ms: u64,
    pub(super) result_decode_ms: u64,
}

impl ExecutionProfile {
    pub(super) fn empty() -> Self {
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
    pub(super) fn op_metrics_factory_fn(self: Rc<Self>) -> OpMetricsFactoryFn {
        Box::new(move |op_id, total, op_decl| {
            self.ensure_capacity(total);
            self.names.borrow_mut()[op_id as usize] = op_decl.name.to_string();
            Some(self.clone().op_metrics_fn())
        })
    }

    pub(super) fn op_metrics_fn(self: Rc<Self>) -> OpMetricsFn {
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

    pub(super) fn top_ops(&self, limit: usize) -> Vec<OpTimingSummary> {
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

    pub(super) fn snapshot(&self) -> OpTimingSnapshot {
        OpTimingSnapshot {
            names: self.names.borrow().clone(),
            totals: self.totals.borrow().clone(),
        }
    }

    pub(super) fn diff(&self, before: &OpTimingSnapshot, limit: usize) -> Vec<RequestOpTiming> {
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
