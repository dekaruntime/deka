use clap::Args;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{config::AgentRegistry, hmac, Result};

#[derive(Debug, Args)]
pub struct DispatchArgs {
    pub agent: String,
    pub task: String,
    #[arg(long, default_value = "codex")]
    pub runtime: String,
    #[arg(long, default_value_t = 300)]
    pub time_budget: u64,
}

#[derive(Serialize)]
struct DispatchRequest<'a> {
    task: &'a str,
    runtime: &'a str,
    time_budget: u64,
}

#[derive(Deserialize)]
struct DispatchResponse {
    run_id: String,
}

pub async fn run(args: DispatchArgs) -> Result<()> {
    let registry = AgentRegistry::load()?;
    let agent = registry
        .find(&args.agent)
        .ok_or_else(|| format!("unknown agent {}", args.agent))?;
    let port = agent
        .port
        .ok_or_else(|| format!("agent {} has no dispatcher port", agent.slug))?;

    let payload = DispatchRequest {
        task: &args.task,
        runtime: &args.runtime,
        time_budget: args.time_budget,
    };
    let body = serde_json::to_vec(&payload)?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let secret = hmac::dispatch_secret()?;
    let signature = hmac::sign(timestamp, &body, secret.as_bytes());

    let response = reqwest::Client::new()
        .post(format!("http://localhost:{port}/task"))
        .header("content-type", "application/json")
        .header("x-gild-timestamp", timestamp.to_string())
        .header("x-gild-signature", signature)
        .body(body)
        .send()
        .await?
        .error_for_status()?;

    let response: DispatchResponse = response.json().await?;
    println!("{}", response.run_id);
    Ok(())
}
