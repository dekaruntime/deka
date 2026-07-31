use super::*;

mod scheduling;

// ========== Worker Handle ==========

/// Handle to communicate with a worker thread
struct WorkerHandle {
    request_tx: mpsc::UnboundedSender<WorkerRequest>,
    control_tx: mpsc::UnboundedSender<WorkerControl>,
    load: Arc<WorkerLoad>,
    #[allow(dead_code)]
    thread: Option<JoinHandle<()>>,
}

// ========== Main Pool ==========

/// The isolate pool manager
pub struct IsolatePool {
    workers: Vec<WorkerHandle>,
    config: PoolConfig,
    metrics: Arc<PoolMetrics>,
    request_seq: AtomicU64,
    introspect_profiling: Arc<AtomicBool>,
    secrets_cache: Arc<SecretsCache>,
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
        let secrets_cache = Arc::new(SecretsCache::from_env());
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
            let worker_secrets_cache = Arc::clone(&secrets_cache);
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
                    worker_secrets_cache,
                );
                worker.run(rx, ctrl_rx);
            });

            workers.push(WorkerHandle {
                request_tx: tx,
                control_tx: ctrl_tx,
                load,
                thread: Some(thread),
            });
        }

        Self {
            workers,
            config,
            metrics,
            request_seq: AtomicU64::new(0),
            introspect_profiling,
            secrets_cache,
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

    pub fn secrets_cache(&self) -> Arc<SecretsCache> {
        Arc::clone(&self.secrets_cache)
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

    /// Evict every cached isolate whose handler name starts with `prefix`.
    /// Returns the total number of isolates evicted across all workers.
    ///
    /// Broadcasts to every worker (not just the consistent-hash target)
    /// because under LeastLoaded scheduling, a tenant's isolate may live
    /// on any worker. Safe no-op if no matches.
    pub async fn evict_by_prefix(&self, prefix: impl Into<String>) -> usize {
        let prefix = prefix.into();
        let mut total_evicted = 0;
        let mut receivers = Vec::new();

        for worker in &self.workers {
            let (tx, rx) = oneshot::channel();
            if worker
                .control_tx
                .send(WorkerControl::EvictByPrefix {
                    prefix: prefix.clone(),
                    response_tx: tx,
                })
                .is_ok()
            {
                receivers.push(rx);
            }
        }

        for rx in receivers {
            if let Ok(count) = rx.await {
                total_evicted += count;
            }
        }

        if total_evicted > 0 {
            self.metrics
                .evictions
                .fetch_add(total_evicted as u64, Ordering::Relaxed);
            tracing::info!(
                "Evicted {} isolate(s) matching prefix '{}' across {} worker(s)",
                total_evicted,
                prefix,
                self.workers.len()
            );
        }

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

impl Drop for IsolatePool {
    fn drop(&mut self) {
        // Dropping a JoinHandle detaches its thread. That allowed test pools
        // to leave thread-affine JsRuntimes alive until the process was
        // already tearing V8 down. Ask each worker to dispose its isolates,
        // wait for that acknowledgement, then join it before returning.
        let workers = std::mem::take(&mut self.workers);
        let mut shutdown_acks = Vec::with_capacity(workers.len());
        let mut threads = Vec::with_capacity(workers.len());

        for mut worker in workers {
            // `IsolatePool` commonly drops from an async test. Use the
            // standard channel here: `oneshot::Receiver::blocking_recv()`
            // would panic when called from that Tokio runtime.
            let (response_tx, response_rx) = std_mpsc::channel();
            if worker
                .control_tx
                .send(WorkerControl::Shutdown { response_tx })
                .is_ok()
            {
                shutdown_acks.push(response_rx);
            }
            if let Some(thread) = worker.thread.take() {
                threads.push(thread);
            }
        }

        for ack in shutdown_acks {
            let _ = ack.recv();
        }
        for thread in threads {
            let _ = thread.join();
        }
    }
}
