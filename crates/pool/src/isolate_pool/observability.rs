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

pub(super) const REQUEST_HISTORY_LIMIT: usize = 200;
