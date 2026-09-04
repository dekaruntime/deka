use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use pool::{RequestOpTiming, RequestState, RequestTrace};

#[derive(Clone)]
pub struct IntrospectArchive {
    db_path: PathBuf,
    retention_days: u64,
}

impl IntrospectArchive {
    pub fn new(db_path: PathBuf, retention_days: u64) -> Self {
        Self {
            db_path,
            retention_days,
        }
    }

    pub fn record_traces(&self, traces: &[RequestTrace]) -> rusqlite::Result<()> {
        if traces.is_empty() {
            return Ok(());
        }

        let mut conn = self.connect()?;
        {
            let tx = conn.transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT OR REPLACE INTO request_traces
                    (id, handler_name, isolate_id, worker_id, started_at_ms, state, duration_ms, error, op_timings, queue_wait_ms, warm_time_us, total_time_us, heap_before_bytes, heap_after_bytes, heap_delta_bytes, response_status, response_body)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                )?;

                for trace in traces {
                    let (state, duration_ms, error) = state_parts(&trace.state);
                    let op_timings = serde_json::to_string(&trace.op_timings)
                        .unwrap_or_else(|_| "[]".to_string());
                    stmt.execute(params![
                        trace.id,
                        trace.handler_name,
                        trace.isolate_id,
                        trace.worker_id as i64,
                        trace.started_at_ms as i64,
                        state,
                        duration_ms.map(|v| v as i64),
                        error,
                        op_timings,
                        trace.queue_wait_ms as i64,
                        trace.warm_time_us as i64,
                        trace.total_time_us as i64,
                        trace.heap_before_bytes as i64,
                        trace.heap_after_bytes as i64,
                        trace.heap_delta_bytes as i64,
                        trace.response_status.map(|v| v as i64),
                        trace.response_body,
                    ])?;
                }
            }
            tx.commit()?;
        }
        self.prune(&conn)?;
        Ok(())
    }

    pub fn fetch_traces_before(
        &self,
        limit: usize,
        cutoff_ms: u64,
    ) -> rusqlite::Result<Vec<RequestTrace>> {
        let conn = self.connect()?;
        self.fetch_traces_with_conn(&conn, limit, Some(cutoff_ms))
    }

    fn fetch_traces_with_conn(
        &self,
        conn: &Connection,
        limit: usize,
        cutoff_ms: Option<u64>,
    ) -> rusqlite::Result<Vec<RequestTrace>> {
        let mut traces = Vec::new();
        let sql = if cutoff_ms.is_some() {
            "SELECT id, handler_name, isolate_id, worker_id, started_at_ms, state, duration_ms, error, op_timings, queue_wait_ms, warm_time_us, total_time_us, heap_before_bytes, heap_after_bytes, heap_delta_bytes, response_status, response_body
             FROM request_traces
             WHERE started_at_ms <= ?1
             ORDER BY started_at_ms DESC
             LIMIT ?2"
        } else {
            "SELECT id, handler_name, isolate_id, worker_id, started_at_ms, state, duration_ms, error, op_timings, queue_wait_ms, warm_time_us, total_time_us, heap_before_bytes, heap_after_bytes, heap_delta_bytes, response_status, response_body
             FROM request_traces
             ORDER BY started_at_ms DESC
             LIMIT ?1"
        };

        let mut stmt = conn.prepare(sql)?;
        let rows = if let Some(cutoff_ms) = cutoff_ms {
            stmt.query_map(params![cutoff_ms as i64, limit as i64], row_to_trace)?
        } else {
            stmt.query_map(params![limit as i64], row_to_trace)?
        };

        for row in rows {
            traces.push(row?);
        }

        Ok(traces)
    }

    fn connect(&self) -> rusqlite::Result<Connection> {
        if let Some(parent) = self.db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(&self.db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS request_traces (
                id TEXT PRIMARY KEY,
                handler_name TEXT NOT NULL,
                isolate_id TEXT NOT NULL,
                worker_id INTEGER NOT NULL,
                started_at_ms INTEGER NOT NULL,
                state TEXT NOT NULL,
                duration_ms INTEGER,
                error TEXT,
                op_timings TEXT,
                queue_wait_ms INTEGER,
                warm_time_us INTEGER,
                total_time_us INTEGER,
                heap_before_bytes INTEGER,
                heap_after_bytes INTEGER,
                heap_delta_bytes INTEGER,
                response_status INTEGER,
                response_body TEXT
            );
            CREATE INDEX IF NOT EXISTS request_traces_started_at
            ON request_traces(started_at_ms DESC);",
        )?;
        if let Err(err) = conn.execute("ALTER TABLE request_traces ADD COLUMN op_timings TEXT", [])
        {
            if !err.to_string().contains("duplicate column") {
                return Err(err);
            }
        }
        if let Err(err) = conn.execute(
            "ALTER TABLE request_traces ADD COLUMN queue_wait_ms INTEGER",
            [],
        ) {
            if !err.to_string().contains("duplicate column") {
                return Err(err);
            }
        }
        if let Err(err) = conn.execute(
            "ALTER TABLE request_traces ADD COLUMN warm_time_us INTEGER",
            [],
        ) {
            if !err.to_string().contains("duplicate column") {
                return Err(err);
            }
        }
        if let Err(err) = conn.execute(
            "ALTER TABLE request_traces ADD COLUMN total_time_us INTEGER",
            [],
        ) {
            if !err.to_string().contains("duplicate column") {
                return Err(err);
            }
        }
        if let Err(err) = conn.execute(
            "ALTER TABLE request_traces ADD COLUMN heap_before_bytes INTEGER",
            [],
        ) {
            if !err.to_string().contains("duplicate column") {
                return Err(err);
            }
        }
        if let Err(err) = conn.execute(
            "ALTER TABLE request_traces ADD COLUMN heap_after_bytes INTEGER",
            [],
        ) {
            if !err.to_string().contains("duplicate column") {
                return Err(err);
            }
        }
        if let Err(err) = conn.execute(
            "ALTER TABLE request_traces ADD COLUMN heap_delta_bytes INTEGER",
            [],
        ) {
            if !err.to_string().contains("duplicate column") {
                return Err(err);
            }
        }
        if let Err(err) = conn.execute(
            "ALTER TABLE request_traces ADD COLUMN response_status INTEGER",
            [],
        ) {
            if !err.to_string().contains("duplicate column") {
                return Err(err);
            }
        }
        if let Err(err) = conn.execute(
            "ALTER TABLE request_traces ADD COLUMN response_body TEXT",
            [],
        ) {
            if !err.to_string().contains("duplicate column") {
                return Err(err);
            }
        }
        Ok(conn)
    }

    fn prune(&self, conn: &Connection) -> rusqlite::Result<()> {
        if self.retention_days == 0 {
            return Ok(());
        }

        let cutoff = now_millis().saturating_sub(
            Duration::from_secs(self.retention_days * 24 * 60 * 60).as_millis() as u64,
        );

        conn.execute(
            "DELETE FROM request_traces WHERE started_at_ms < ?1",
            params![cutoff as i64],
        )?;
        Ok(())
    }
}

fn state_parts(state: &RequestState) -> (String, Option<u64>, Option<String>) {
    match state {
        RequestState::Executing => ("executing".to_string(), None, None),
        RequestState::Completed { duration_ms } => {
            ("completed".to_string(), Some(*duration_ms), None)
        }
        RequestState::Failed { error, duration_ms } => (
            "failed".to_string(),
            Some(*duration_ms),
            Some(error.clone()),
        ),
        RequestState::QueueTimeout { waited_ms } => {
            ("queue_timeout".to_string(), Some(*waited_ms), None)
        }
    }
}

fn row_to_trace(row: &rusqlite::Row) -> rusqlite::Result<RequestTrace> {
    let state: String = row.get(5)?;
    let duration_ms: Option<i64> = row.get(6)?;
    let error: Option<String> = row.get(7)?;
    let op_timings_raw: Option<String> = row.get(8).ok();
    let op_timings: Vec<RequestOpTiming> = op_timings_raw
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    let queue_wait_ms: Option<i64> = row.get(9).ok();
    let warm_time_us: Option<i64> = row.get(10).ok();
    let total_time_us: Option<i64> = row.get(11).ok();
    let heap_before_bytes: Option<i64> = row.get(12).ok();
    let heap_after_bytes: Option<i64> = row.get(13).ok();
    let heap_delta_bytes: Option<i64> = row.get(14).ok();
    let response_status: Option<i64> = row.get(15).ok();
    let response_body: Option<String> = row.get(16).ok();

    let request_state = match state.as_str() {
        "completed" => RequestState::Completed {
            duration_ms: duration_ms.unwrap_or(0) as u64,
        },
        "failed" => RequestState::Failed {
            error: error.unwrap_or_else(|| "unknown error".to_string()),
            duration_ms: duration_ms.unwrap_or(0) as u64,
        },
        "queue_timeout" => RequestState::QueueTimeout {
            waited_ms: duration_ms.unwrap_or(0) as u64,
        },
        _ => RequestState::Executing,
    };

    Ok(RequestTrace {
        id: row.get(0)?,
        handler_name: row.get(1)?,
        isolate_id: row.get(2)?,
        worker_id: row.get::<_, i64>(3)? as usize,
        started_at_ms: row.get::<_, i64>(4)? as u64,
        state: request_state,
        op_timings,
        queue_wait_ms: queue_wait_ms.unwrap_or(0) as u64,
        warm_time_us: warm_time_us.unwrap_or(0) as u64,
        total_time_us: total_time_us.unwrap_or(0) as u64,
        heap_before_bytes: heap_before_bytes.unwrap_or(0) as usize,
        heap_after_bytes: heap_after_bytes.unwrap_or(0) as usize,
        heap_delta_bytes: heap_delta_bytes.unwrap_or(0) as i64,
        response_status: response_status.and_then(|v| u16::try_from(v).ok()),
        response_body,
    })
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}
