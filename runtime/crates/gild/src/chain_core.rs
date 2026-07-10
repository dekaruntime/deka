use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_STATE_DB: &str = "/var/lib/gild-chain/state.db";
pub const SUBSCRIPTION_KIND: &str = "agent.*";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Policy {
    DefaultFlow,
    ManualMerge,
    DryRun,
}

impl Policy {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "default" | "default-flow" => Ok(Self::DefaultFlow),
            "manual-merge" => Ok(Self::ManualMerge),
            "dry-run" => Ok(Self::DryRun),
            _ => bail!("unknown policy {value:?}"),
        }
    }

    pub fn supported_cli_policies() -> &'static [&'static str] {
        &["default-flow"]
    }
}

impl fmt::Display for Policy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefaultFlow => f.write_str("default-flow"),
            Self::ManualMerge => f.write_str("manual-merge"),
            Self::DryRun => f.write_str("dry-run"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChainStatus {
    WaitingRunCompleted,
    WaitingPr,
    WaitingReview,
    Approved,
    Merged,
    Closed,
    Failed,
}

impl ChainStatus {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "waiting-run-completed" => Ok(Self::WaitingRunCompleted),
            "waiting-pr" => Ok(Self::WaitingPr),
            "waiting-review" => Ok(Self::WaitingReview),
            "approved" => Ok(Self::Approved),
            "merged" => Ok(Self::Merged),
            "closed" => Ok(Self::Closed),
            "failed" => Ok(Self::Failed),
            _ => bail!("unknown chain status {value:?}"),
        }
    }
}

impl fmt::Display for ChainStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WaitingRunCompleted => f.write_str("waiting-run-completed"),
            Self::WaitingPr => f.write_str("waiting-pr"),
            Self::WaitingReview => f.write_str("waiting-review"),
            Self::Approved => f.write_str("approved"),
            Self::Merged => f.write_str("merged"),
            Self::Closed => f.write_str("closed"),
            Self::Failed => f.write_str("failed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chain {
    pub id: String,
    pub policy: Policy,
    pub status: ChainStatus,
    pub dispatch_agent: String,
    pub task: String,
    pub issue_id: Option<i64>,
    pub run_id: Option<String>,
    pub agent_slug: Option<String>,
    pub repo: Option<String>,
    pub pr_number: Option<i64>,
    pub pr_state: Option<String>,
    pub review_status: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_event: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum PulseEvent {
    AgentRunCompleted {
        id: Option<i64>,
        chain_id: Option<String>,
        source_id: Option<String>,
        agent_slug: Option<String>,
        run_id: Option<String>,
        repo: Option<String>,
        pr_number: Option<i64>,
        conclusion: Option<String>,
    },
    AgentPrOpened {
        id: Option<i64>,
        chain_id: Option<String>,
        source_id: Option<String>,
        run_id: Option<String>,
        agent_slug: Option<String>,
        repo: Option<String>,
        pr_number: i64,
    },
    AgentReviewPosted {
        id: Option<i64>,
        chain_id: Option<String>,
        source_id: Option<String>,
        run_id: Option<String>,
        agent_slug: Option<String>,
        reviewer: Option<String>,
        verdict: Option<String>,
        repo: Option<String>,
        pr_number: Option<i64>,
    },
    AgentPrMerged {
        id: Option<i64>,
        chain_id: Option<String>,
        source_id: Option<String>,
        run_id: Option<String>,
        agent_slug: Option<String>,
        repo: Option<String>,
        pr_number: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Action {
    RequestReview {
        reviewer: String,
        repo: Option<String>,
        pr_number: i64,
    },
    MergePullRequest {
        repo: Option<String>,
        pr_number: i64,
    },
    Log(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Deserialize)]
struct RawPulseEvent {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    source_id: Option<String>,
    kind: String,
    #[serde(default)]
    payload: Option<RawPulsePayload>,
    #[serde(default)]
    chain_id: Option<String>,
    #[serde(default, rename = "chainId")]
    chain_id_camel: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    agent_slug: Option<String>,
    #[serde(default, rename = "agentSlug")]
    agent_slug_camel: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default, rename = "runId")]
    run_id_camel: Option<String>,
    #[serde(default)]
    pr_number: Option<i64>,
    #[serde(default, rename = "prNumber")]
    pr_number_camel: Option<i64>,
    #[serde(default)]
    pull_number: Option<i64>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    reviewer: Option<String>,
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    repo: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawPulsePayload {
    #[serde(default)]
    chain_id: Option<String>,
    #[serde(default, rename = "chainId")]
    chain_id_camel: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default, rename = "runId")]
    run_id_camel: Option<String>,
    #[serde(default)]
    agent_slug: Option<String>,
    #[serde(default, rename = "agentSlug")]
    agent_slug_camel: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    pr_number: Option<i64>,
    #[serde(default, rename = "prNumber")]
    pr_number_camel: Option<i64>,
    #[serde(default)]
    pull_number: Option<i64>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    reviewer: Option<String>,
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

pub fn state_db_path() -> PathBuf {
    std::env::var("GILD_CHAIN_STATE_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_STATE_DB))
}

pub fn new_chain(policy: Policy, dispatch_agent: String, task: String) -> Chain {
    let now = now_ms();
    Chain {
        id: format!("chain_{now:x}"),
        policy,
        status: ChainStatus::WaitingRunCompleted,
        dispatch_agent,
        task,
        issue_id: None,
        run_id: None,
        agent_slug: None,
        repo: None,
        pr_number: None,
        pr_state: None,
        review_status: None,
        created_at_ms: now,
        updated_at_ms: now,
        last_event: Some("chain.created".into()),
    }
}

pub fn apply_event(chain: &mut Chain, event: &PulseEvent) -> Vec<Action> {
    if chain.policy != Policy::DefaultFlow {
        chain.status = ChainStatus::Failed;
        chain.last_event = Some("policy.not_implemented".into());
        chain.updated_at_ms = now_ms();
        return vec![Action::Log(format!(
            "policy {} is not yet implemented",
            chain.policy
        ))];
    }

    let actions = match event {
        PulseEvent::AgentRunCompleted {
            run_id,
            agent_slug,
            repo,
            pr_number,
            conclusion,
            ..
        } => {
            chain.run_id = run_id.clone().or_else(|| chain.run_id.clone());
            chain.agent_slug = agent_slug.clone().or_else(|| chain.agent_slug.clone());
            chain.repo = repo.clone().or_else(|| chain.repo.clone());
            if conclusion.as_deref() == Some("failed") {
                chain.status = ChainStatus::Failed;
                vec![Action::Log("dispatch run failed".into())]
            } else if let Some(pr_number) = pr_number.or(chain.pr_number) {
                chain.status = ChainStatus::WaitingReview;
                chain.pr_number = Some(pr_number);
                chain.pr_state = Some("open".into());
                vec![Action::RequestReview {
                    reviewer: "Amina".into(),
                    repo: chain.repo.clone(),
                    pr_number,
                }]
            } else {
                chain.status = ChainStatus::WaitingPr;
                vec![Action::Log(
                    "run completed; waiting for PR opened event".into(),
                )]
            }
        }
        PulseEvent::AgentPrOpened {
            run_id,
            agent_slug,
            repo,
            pr_number,
            ..
        } => {
            chain.run_id = run_id.clone().or_else(|| chain.run_id.clone());
            chain.agent_slug = agent_slug.clone().or_else(|| chain.agent_slug.clone());
            chain.repo = repo.clone().or_else(|| chain.repo.clone());
            chain.pr_number = Some(*pr_number);
            chain.pr_state = Some("open".into());
            if matches!(chain.status, ChainStatus::WaitingPr) {
                chain.status = ChainStatus::WaitingReview;
                vec![Action::RequestReview {
                    reviewer: "Amina".into(),
                    repo: chain.repo.clone(),
                    pr_number: *pr_number,
                }]
            } else {
                Vec::new()
            }
        }
        PulseEvent::AgentReviewPosted {
            run_id,
            agent_slug,
            repo,
            verdict,
            pr_number,
            ..
        } => {
            chain.run_id = run_id.clone().or_else(|| chain.run_id.clone());
            chain.agent_slug = agent_slug.clone().or_else(|| chain.agent_slug.clone());
            chain.repo = repo.clone().or_else(|| chain.repo.clone());
            chain.review_status = verdict.clone();
            if !matches!(
                verdict.as_deref(),
                Some("approved") | Some("APPROVED") | Some("approve") | Some("APPROVE")
            ) {
                return finish_event(chain, event, Vec::new());
            }
            let pr_number = pr_number.or(chain.pr_number);
            chain.status = ChainStatus::Approved;
            chain.review_status = Some("APPROVED".into());
            if chain.pr_state.as_deref() == Some("open") {
                pr_number
                    .map(|pr_number| Action::MergePullRequest {
                        repo: chain.repo.clone(),
                        pr_number,
                    })
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            }
        }
        PulseEvent::AgentPrMerged {
            run_id,
            agent_slug,
            repo,
            pr_number,
            ..
        } => {
            chain.run_id = run_id.clone().or_else(|| chain.run_id.clone());
            chain.agent_slug = agent_slug.clone().or_else(|| chain.agent_slug.clone());
            chain.repo = repo.clone().or_else(|| chain.repo.clone());
            chain.pr_number = Some(*pr_number);
            chain.pr_state = Some("merged".into());
            chain.status = ChainStatus::Closed;
            Vec::new()
        }
    };
    finish_event(chain, event, actions)
}

fn finish_event(chain: &mut Chain, event: &PulseEvent, actions: Vec<Action>) -> Vec<Action> {
    chain.last_event = Some(event_name(event).into());
    chain.updated_at_ms = now_ms();
    actions
}

pub fn parse_sse_frames(input: &str) -> Vec<SseFrame> {
    input
        .split("\n\n")
        .filter_map(|chunk| {
            let mut event = None;
            let mut data_lines = Vec::new();
            for line in chunk.lines() {
                let line = line.trim_end_matches('\r');
                if line.is_empty() || line.starts_with(':') {
                    continue;
                }
                if let Some(value) = line.strip_prefix("event:") {
                    event = Some(value.trim_start().to_string());
                } else if let Some(value) = line.strip_prefix("data:") {
                    data_lines.push(value.trim_start().to_string());
                }
            }
            if data_lines.is_empty() {
                None
            } else {
                Some(SseFrame {
                    event,
                    data: data_lines.join("\n"),
                })
            }
        })
        .collect()
}

pub fn parse_pulse_event(frame: &SseFrame) -> Result<Option<PulseEvent>> {
    let raw: RawPulseEvent = serde_json::from_str(&frame.data).context("parse pulse event JSON")?;
    let payload = raw.payload.unwrap_or_default();
    let chain_id = payload
        .chain_id
        .or(payload.chain_id_camel)
        .or(raw.chain_id)
        .or(raw.chain_id_camel);
    let run_id = payload
        .run_id
        .or(payload.run_id_camel)
        .or(raw.run_id)
        .or(raw.run_id_camel);
    let agent_slug = payload
        .agent_slug
        .or(payload.agent_slug_camel)
        .or(payload.agent)
        .or(raw.agent_slug)
        .or(raw.agent_slug_camel)
        .or(raw.agent);
    let repo = payload.repo.or(raw.repo);
    let pr_number = payload
        .pr_number
        .or(payload.pr_number_camel)
        .or(payload.pull_number)
        .or(raw.pr_number)
        .or(raw.pr_number_camel)
        .or(raw.pull_number);
    let conclusion = payload
        .conclusion
        .or(payload.status)
        .or(raw.conclusion)
        .or(raw.status);
    let reviewer = payload.reviewer.or(raw.reviewer);
    let verdict = payload.verdict.or(raw.verdict);
    let event = match raw.kind.as_str() {
        "agent.run.completed" => PulseEvent::AgentRunCompleted {
            id: raw.id,
            chain_id,
            source_id: raw.source_id,
            agent_slug,
            run_id,
            repo,
            pr_number,
            conclusion,
        },
        "agent.pr.opened" => PulseEvent::AgentPrOpened {
            id: raw.id,
            chain_id,
            source_id: raw.source_id,
            run_id,
            agent_slug,
            repo,
            pr_number: pr_number.ok_or_else(|| anyhow!("agent.pr.opened missing pr_number"))?,
        },
        "agent.review.posted" => PulseEvent::AgentReviewPosted {
            id: raw.id,
            chain_id,
            source_id: raw.source_id,
            run_id,
            agent_slug,
            reviewer,
            verdict,
            repo,
            pr_number,
        },
        "agent.pr.merged" => PulseEvent::AgentPrMerged {
            id: raw.id,
            chain_id,
            source_id: raw.source_id,
            run_id,
            agent_slug,
            repo,
            pr_number: pr_number.ok_or_else(|| anyhow!("agent.pr.merged missing pr_number"))?,
        },
        _ => return Ok(None),
    };
    Ok(Some(event))
}

pub fn event_chain_id(event: &PulseEvent) -> Option<&str> {
    match event {
        PulseEvent::AgentRunCompleted { chain_id, .. }
        | PulseEvent::AgentPrOpened { chain_id, .. }
        | PulseEvent::AgentReviewPosted { chain_id, .. }
        | PulseEvent::AgentPrMerged { chain_id, .. } => chain_id.as_deref(),
    }
}

pub fn event_run_id(event: &PulseEvent) -> Option<&str> {
    match event {
        PulseEvent::AgentRunCompleted { run_id, .. }
        | PulseEvent::AgentPrOpened { run_id, .. }
        | PulseEvent::AgentReviewPosted { run_id, .. }
        | PulseEvent::AgentPrMerged { run_id, .. } => run_id.as_deref(),
    }
}

pub fn event_pr_number(event: &PulseEvent) -> Option<i64> {
    match event {
        PulseEvent::AgentRunCompleted { pr_number, .. }
        | PulseEvent::AgentReviewPosted { pr_number, .. } => *pr_number,
        PulseEvent::AgentPrOpened { pr_number, .. }
        | PulseEvent::AgentPrMerged { pr_number, .. } => Some(*pr_number),
    }
}

pub fn event_name(event: &PulseEvent) -> &'static str {
    match event {
        PulseEvent::AgentRunCompleted { .. } => "agent.run.completed",
        PulseEvent::AgentPrOpened { .. } => "agent.pr.opened",
        PulseEvent::AgentReviewPosted { .. } => "agent.review.posted",
        PulseEvent::AgentPrMerged { .. } => "agent.pr.merged",
    }
}

pub async fn subscribe_once<F>(pulse_url: &str, since: Option<i64>, mut handler: F) -> Result<()>
where
    F: FnMut(PulseEvent) -> Result<()>,
{
    let client = reqwest::Client::new();
    let mut request = client.get(pulse_url).query(&[("kind", SUBSCRIPTION_KIND)]);
    let since_string;
    if let Some(since) = since {
        since_string = since.to_string();
        request = request.query(&[("since", since_string.as_str())]);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("connect to pulse SSE {pulse_url}"))?
        .error_for_status()
        .context("pulse SSE status")?;
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read pulse SSE chunk")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(index) = buffer.find("\n\n") {
            let frame_text = buffer[..index + 2].to_string();
            buffer.drain(..index + 2);
            for frame in parse_sse_frames(&frame_text) {
                if let Some(event) = parse_pulse_event(&frame)? {
                    handler(event)?;
                }
            }
        }
    }
    Ok(())
}

pub struct StateStore {
    conn: Connection,
}

impl StateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let conn = Connection::open(path.as_ref())
            .with_context(|| format!("open sqlite state {}", path.as_ref().display()))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn in_memory() -> Result<Self> {
        let store = Self {
            conn: Connection::open_in_memory().context("open in-memory sqlite state")?,
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn insert_chain(&self, chain: &Chain) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chains (
                id, policy, status, dispatch_agent, task, issue_id, run_id, agent_slug, repo,
                pr_number, pr_state, review_status, created_at_ms, updated_at_ms, last_event
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                chain.id,
                chain.policy.to_string(),
                chain.status.to_string(),
                chain.dispatch_agent,
                chain.task,
                chain.issue_id,
                chain.run_id,
                chain.agent_slug,
                chain.repo,
                chain.pr_number,
                chain.pr_state,
                chain.review_status,
                chain.created_at_ms,
                chain.updated_at_ms,
                chain.last_event,
            ],
        )?;
        Ok(())
    }

    pub fn save_chain(&self, chain: &Chain) -> Result<()> {
        self.conn.execute(
            "UPDATE chains SET
                policy = ?2,
                status = ?3,
                dispatch_agent = ?4,
                task = ?5,
                issue_id = ?6,
                run_id = ?7,
                agent_slug = ?8,
                repo = ?9,
                pr_number = ?10,
                pr_state = ?11,
                review_status = ?12,
                updated_at_ms = ?13,
                last_event = ?14
             WHERE id = ?1",
            params![
                chain.id,
                chain.policy.to_string(),
                chain.status.to_string(),
                chain.dispatch_agent,
                chain.task,
                chain.issue_id,
                chain.run_id,
                chain.agent_slug,
                chain.repo,
                chain.pr_number,
                chain.pr_state,
                chain.review_status,
                chain.updated_at_ms,
                chain.last_event,
            ],
        )?;
        Ok(())
    }

    pub fn get_chain(&self, id: &str) -> Result<Option<Chain>> {
        self.conn
            .query_row(
                "SELECT id, policy, status, dispatch_agent, task, issue_id, run_id,
                    agent_slug, repo, pr_number, pr_state, review_status,
                    created_at_ms, updated_at_ms, last_event
                 FROM chains WHERE id = ?1",
                [id],
                row_to_chain,
            )
            .optional()
            .context("load chain")
    }

    pub fn find_chain_for_event(&self, event: &PulseEvent) -> Result<Option<Chain>> {
        if let Some(chain_id) = event_chain_id(event) {
            if let Some(chain) = self.get_chain(chain_id)? {
                return Ok(Some(chain));
            }
        }
        if let Some(run_id) = event_run_id(event) {
            if let Some(chain) = self
                .conn
                .query_row(
                    "SELECT id, policy, status, dispatch_agent, task, issue_id, run_id,
                        agent_slug, repo, pr_number, pr_state, review_status,
                        created_at_ms, updated_at_ms, last_event
                     FROM chains WHERE run_id = ?1 ORDER BY updated_at_ms DESC LIMIT 1",
                    [run_id],
                    row_to_chain,
                )
                .optional()
                .context("load chain by run_id")?
            {
                return Ok(Some(chain));
            }
        }
        if let Some(pr_number) = event_pr_number(event) {
            return self
                .conn
                .query_row(
                    "SELECT id, policy, status, dispatch_agent, task, issue_id, run_id,
                        agent_slug, repo, pr_number, pr_state, review_status,
                        created_at_ms, updated_at_ms, last_event
                     FROM chains WHERE pr_number = ?1 ORDER BY updated_at_ms DESC LIMIT 1",
                    [pr_number],
                    row_to_chain,
                )
                .optional()
                .context("load chain by pr_number");
        }
        Ok(None)
    }

    pub fn list_active(&self) -> Result<Vec<Chain>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, policy, status, dispatch_agent, task, issue_id, run_id,
                agent_slug, repo, pr_number, pr_state, review_status,
                created_at_ms, updated_at_ms, last_event
             FROM chains
             WHERE status NOT IN ('closed', 'failed')
             ORDER BY updated_at_ms DESC",
        )?;
        let chains = stmt
            .query_map([], row_to_chain)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(chains)
    }

    pub fn summary(&self) -> Result<ChainSummary> {
        let active_runs = self.count_where("status NOT IN ('closed', 'failed')")?;
        let completed_flows = self.count_where("status = 'closed'")?;
        let failed_flows = self.count_where("status = 'failed'")?;
        let pending_merges =
            self.count_where("status = 'approved' AND COALESCE(pr_state, '') != 'merged'")?;
        Ok(ChainSummary {
            active_runs,
            completed_flows,
            failed_flows,
            pending_merges,
        })
    }

    fn count_where(&self, predicate: &str) -> Result<i64> {
        self.conn
            .query_row(
                &format!("SELECT COUNT(*) FROM chains WHERE {predicate}"),
                [],
                |row| row.get(0),
            )
            .context("count chain rows")
    }

    pub fn record_event(
        &self,
        chain_id: &str,
        event: &PulseEvent,
        actions: &[Action],
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chain_events (chain_id, kind, payload_json, actions_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                chain_id,
                event_name(event),
                serde_json::to_string(event)?,
                serde_json::to_string(actions)?,
                now_ms(),
            ],
        )?;
        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chains (
                id TEXT PRIMARY KEY,
                policy TEXT NOT NULL,
                status TEXT NOT NULL,
                dispatch_agent TEXT NOT NULL,
                task TEXT NOT NULL,
                issue_id INTEGER,
                run_id TEXT,
                agent_slug TEXT,
                repo TEXT,
                pr_number INTEGER,
                pr_state TEXT,
                review_status TEXT,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                last_event TEXT
            );
            CREATE TABLE IF NOT EXISTS chain_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chain_id TEXT NOT NULL REFERENCES chains(id) ON DELETE CASCADE,
                kind TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                actions_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_chains_status ON chains(status);
            CREATE INDEX IF NOT EXISTS idx_chains_run_id ON chains(run_id);
            CREATE INDEX IF NOT EXISTS idx_chains_pr_number ON chains(pr_number);
            CREATE INDEX IF NOT EXISTS idx_chain_events_chain ON chain_events(chain_id, id);",
        )?;
        self.add_column_if_missing("chains", "run_id", "TEXT")?;
        self.add_column_if_missing("chains", "agent_slug", "TEXT")?;
        self.add_column_if_missing("chains", "repo", "TEXT")?;
        self.add_column_if_missing("chains", "pr_state", "TEXT")?;
        Ok(())
    }

    fn add_column_if_missing(&self, table: &str, column: &str, kind: &str) -> Result<()> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if !columns.iter().any(|existing| existing == column) {
            self.conn.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"),
                [],
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChainSummary {
    pub active_runs: i64,
    pub completed_flows: i64,
    pub failed_flows: i64,
    pub pending_merges: i64,
}

fn row_to_chain(row: &rusqlite::Row<'_>) -> rusqlite::Result<Chain> {
    let policy: String = row.get(1)?;
    let status: String = row.get(2)?;
    Ok(Chain {
        id: row.get(0)?,
        policy: Policy::parse(&policy).map_err(to_sql_error)?,
        status: ChainStatus::parse(&status).map_err(to_sql_error)?,
        dispatch_agent: row.get(3)?,
        task: row.get(4)?,
        issue_id: row.get(5)?,
        run_id: row.get(6)?,
        agent_slug: row.get(7)?,
        repo: row.get(8)?,
        pr_number: row.get(9)?,
        pr_state: row.get(10)?,
        review_status: row.get(11)?,
        created_at_ms: row.get(12)?,
        updated_at_ms: row.get(13)?,
        last_event: row.get(14)?,
    })
}

fn to_sql_error(err: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::<dyn std::error::Error + Send + Sync>::from(err),
    )
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn default_flow_waits_for_pr_after_completed_run() {
        let mut chain = new_chain(
            Policy::DefaultFlow,
            "agent-khalid".into(),
            "build skeleton".into(),
        );
        let chain_id = chain.id.clone();
        let actions = apply_event(
            &mut chain,
            &PulseEvent::AgentRunCompleted {
                id: Some(1),
                chain_id: Some(chain_id),
                source_id: Some("gild".into()),
                agent_slug: Some("agent-khalid".into()),
                run_id: Some("run_1".into()),
                repo: Some("tana/deka".into()),
                pr_number: None,
                conclusion: Some("success".into()),
            },
        );
        assert_eq!(chain.status, ChainStatus::WaitingPr);
        assert_eq!(chain.run_id.as_deref(), Some("run_1"));
        assert_eq!(
            actions,
            vec![Action::Log(
                "run completed; waiting for PR opened event".into()
            )]
        );
    }

    #[test]
    fn default_flow_pr_opened_requests_review_after_completed_run() {
        let mut chain = new_chain(Policy::DefaultFlow, "agent-khalid".into(), "task".into());
        chain.status = ChainStatus::WaitingPr;
        chain.run_id = Some("run_1".into());
        let actions = apply_event(
            &mut chain,
            &PulseEvent::AgentPrOpened {
                id: Some(2),
                chain_id: None,
                source_id: Some("gild".into()),
                run_id: Some("run_1".into()),
                agent_slug: Some("agent-khalid".into()),
                repo: Some("tana/deka".into()),
                pr_number: 42,
            },
        );
        assert_eq!(chain.status, ChainStatus::WaitingReview);
        assert_eq!(chain.pr_number, Some(42));
        assert_eq!(
            actions,
            vec![Action::RequestReview {
                reviewer: "Amina".into(),
                repo: Some("tana/deka".into()),
                pr_number: 42
            }]
        );
    }

    #[test]
    fn default_flow_approved_review_requests_merge() {
        let mut chain = new_chain(Policy::DefaultFlow, "agent-khalid".into(), "task".into());
        chain.status = ChainStatus::WaitingReview;
        chain.pr_number = Some(7);
        chain.pr_state = Some("open".into());
        let actions = apply_event(
            &mut chain,
            &PulseEvent::AgentReviewPosted {
                id: Some(3),
                chain_id: None,
                source_id: Some("gild".into()),
                run_id: Some("run_1".into()),
                agent_slug: Some("agent-amina".into()),
                reviewer: Some("Amina".into()),
                verdict: Some("approved".into()),
                repo: Some("tana/deka".into()),
                pr_number: None,
            },
        );
        assert_eq!(chain.status, ChainStatus::Approved);
        assert_eq!(chain.review_status.as_deref(), Some("APPROVED"));
        assert_eq!(
            actions,
            vec![Action::MergePullRequest {
                repo: Some("tana/deka".into()),
                pr_number: 7
            }]
        );
    }

    #[test]
    fn parses_pulse_event_kinds() {
        let frames = parse_sse_frames(
            "event: message\n\
             data: {\"id\":1,\"source_id\":\"gild\",\"kind\":\"agent.run.completed\",\"payload\":{\"chain_id\":\"chain_1\",\"agent_slug\":\"agent-X\",\"run_id\":\"run_9\",\"repo\":\"tana/deka\",\"conclusion\":\"success\"}}\n\n\
             data: {\"id\":2,\"source_id\":\"gild\",\"kind\":\"agent.pr.opened\",\"payload\":{\"run_id\":\"run_9\",\"agent_slug\":\"agent-X\",\"repo\":\"tana/deka\",\"pr_number\":13}}\n\n\
             data: {\"id\":3,\"source_id\":\"gild\",\"kind\":\"agent.review.posted\",\"payload\":{\"run_id\":\"run_9\",\"agent_slug\":\"agent-amina\",\"reviewer\":\"Amina\",\"repo\":\"tana/deka\",\"pr_number\":13,\"verdict\":\"approved\"}}\n\n\
             data: {\"id\":4,\"source_id\":\"gild\",\"kind\":\"agent.pr.merged\",\"payload\":{\"run_id\":\"run_9\",\"agent_slug\":\"agent-X\",\"repo\":\"tana/deka\",\"pr_number\":13}}\n\n",
        );
        assert_eq!(frames.len(), 4);
        let event = parse_pulse_event(&frames[0]).unwrap().unwrap();
        assert_eq!(
            event,
            PulseEvent::AgentRunCompleted {
                id: Some(1),
                chain_id: Some("chain_1".into()),
                source_id: Some("gild".into()),
                agent_slug: Some("agent-X".into()),
                run_id: Some("run_9".into()),
                repo: Some("tana/deka".into()),
                pr_number: None,
                conclusion: Some("success".into()),
            }
        );
        assert!(matches!(
            parse_pulse_event(&frames[1]).unwrap().unwrap(),
            PulseEvent::AgentPrOpened { pr_number: 13, .. }
        ));
        assert!(matches!(
            parse_pulse_event(&frames[2]).unwrap().unwrap(),
            PulseEvent::AgentReviewPosted {
                verdict: Some(_),
                pr_number: Some(13),
                ..
            }
        ));
        assert!(matches!(
            parse_pulse_event(&frames[3]).unwrap().unwrap(),
            PulseEvent::AgentPrMerged { pr_number: 13, .. }
        ));
    }

    #[test]
    fn persists_chain_state_in_sqlite() {
        let store = StateStore::in_memory().unwrap();
        let mut chain = new_chain(Policy::DefaultFlow, "agent-X".into(), "do work".into());
        store.insert_chain(&chain).unwrap();

        let chain_id = chain.id.clone();
        apply_event(
            &mut chain,
            &PulseEvent::AgentRunCompleted {
                id: Some(1),
                chain_id: Some(chain_id),
                source_id: Some("gild".into()),
                agent_slug: Some("agent-X".into()),
                run_id: Some("run_88".into()),
                repo: Some("tana/deka".into()),
                pr_number: Some(88),
                conclusion: Some("success".into()),
            },
        );
        store.save_chain(&chain).unwrap();

        let loaded = store.get_chain(&chain.id).unwrap().unwrap();
        assert_eq!(loaded.status, ChainStatus::WaitingReview);
        assert_eq!(loaded.pr_number, Some(88));
        assert_eq!(store.list_active().unwrap().len(), 1);
    }

    #[test]
    fn mock_event_sequence_updates_sqlite_and_side_effects() {
        let store = StateStore::in_memory().unwrap();
        let chain = new_chain(Policy::DefaultFlow, "agent-X".into(), "do work".into());
        store.insert_chain(&chain).unwrap();
        let actions = apply_and_store(
            &store,
            PulseEvent::AgentRunCompleted {
                id: Some(1),
                chain_id: Some(chain.id.clone()),
                source_id: Some("gild".into()),
                agent_slug: Some("agent-X".into()),
                run_id: Some("run_1".into()),
                repo: Some("tana/deka".into()),
                pr_number: None,
                conclusion: Some("success".into()),
            },
        );
        assert_eq!(actions.len(), 1);
        let actions = apply_and_store(
            &store,
            PulseEvent::AgentPrOpened {
                id: Some(2),
                chain_id: None,
                source_id: Some("gild".into()),
                run_id: Some("run_1".into()),
                agent_slug: Some("agent-X".into()),
                repo: Some("tana/deka".into()),
                pr_number: 11,
            },
        );
        assert_eq!(
            actions,
            vec![Action::RequestReview {
                reviewer: "Amina".into(),
                repo: Some("tana/deka".into()),
                pr_number: 11
            }]
        );
        let actions = apply_and_store(
            &store,
            PulseEvent::AgentReviewPosted {
                id: Some(3),
                chain_id: None,
                source_id: Some("gild".into()),
                run_id: Some("run_1".into()),
                agent_slug: Some("agent-amina".into()),
                reviewer: Some("Amina".into()),
                verdict: Some("approved".into()),
                repo: Some("tana/deka".into()),
                pr_number: Some(11),
            },
        );
        assert_eq!(
            actions,
            vec![Action::MergePullRequest {
                repo: Some("tana/deka".into()),
                pr_number: 11
            }]
        );
        apply_and_store(
            &store,
            PulseEvent::AgentPrMerged {
                id: Some(4),
                chain_id: None,
                source_id: Some("gild".into()),
                run_id: Some("run_1".into()),
                agent_slug: Some("agent-X".into()),
                repo: Some("tana/deka".into()),
                pr_number: 11,
            },
        );
        let loaded = store.get_chain(&chain.id).unwrap().unwrap();
        assert_eq!(loaded.status, ChainStatus::Closed);
        assert_eq!(loaded.pr_state.as_deref(), Some("merged"));
    }

    #[tokio::test]
    async fn consumes_full_chain_from_local_mock_sse_stream() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 2048];
            let read = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.contains("kind=agent.%2A") || request.contains("kind=agent.*"));
            let body = concat!(
                "event: pulse.event\n",
                "data: {\"id\":1,\"source_id\":\"gild\",\"kind\":\"agent.run.completed\",\"payload\":{\"chain_id\":\"chain_sse\",\"run_id\":\"run_sse\",\"agent_slug\":\"agent-X\",\"repo\":\"tana/deka\",\"conclusion\":\"success\"}}\n\n",
                "event: pulse.event\n",
                "data: {\"id\":2,\"source_id\":\"gild\",\"kind\":\"agent.pr.opened\",\"payload\":{\"run_id\":\"run_sse\",\"agent_slug\":\"agent-X\",\"repo\":\"tana/deka\",\"pr_number\":21}}\n\n",
                "event: pulse.event\n",
                "data: {\"id\":3,\"source_id\":\"gild\",\"kind\":\"agent.review.posted\",\"payload\":{\"run_id\":\"run_sse\",\"agent_slug\":\"agent-amina\",\"reviewer\":\"Amina\",\"repo\":\"tana/deka\",\"pr_number\":21,\"verdict\":\"approved\"}}\n\n",
                "event: pulse.event\n",
                "data: {\"id\":4,\"source_id\":\"gild\",\"kind\":\"agent.pr.merged\",\"payload\":{\"run_id\":\"run_sse\",\"agent_slug\":\"agent-X\",\"repo\":\"tana/deka\",\"pr_number\":21}}\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let store = StateStore::in_memory().unwrap();
        let mut chain = new_chain(Policy::DefaultFlow, "agent-X".into(), "do work".into());
        chain.id = "chain_sse".into();
        store.insert_chain(&chain).unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let observed_events = Arc::clone(&observed);
        subscribe_once(&format!("http://{address}/v1/subscribe"), None, |event| {
            let actions = apply_and_store(&store, event);
            observed_events.lock().unwrap().extend(actions);
            Ok(())
        })
        .await
        .unwrap();
        server.await.unwrap();

        let loaded = store.get_chain("chain_sse").unwrap().unwrap();
        assert_eq!(loaded.status, ChainStatus::Closed);
        assert_eq!(
            *observed.lock().unwrap(),
            vec![
                Action::Log("run completed; waiting for PR opened event".into()),
                Action::RequestReview {
                    reviewer: "Amina".into(),
                    repo: Some("tana/deka".into()),
                    pr_number: 21,
                },
                Action::MergePullRequest {
                    repo: Some("tana/deka".into()),
                    pr_number: 21,
                }
            ]
        );
    }

    fn apply_and_store(store: &StateStore, event: PulseEvent) -> Vec<Action> {
        let mut chain = store.find_chain_for_event(&event).unwrap().unwrap();
        let actions = apply_event(&mut chain, &event);
        store.save_chain(&chain).unwrap();
        store.record_event(&chain.id, &event, &actions).unwrap();
        actions
    }
}
