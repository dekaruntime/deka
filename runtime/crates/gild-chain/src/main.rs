use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use gild_chain::{
    Action, Chain, Policy, StateStore, apply_event, event_chain_id, new_chain, parse_pulse_event,
    parse_sse_frames, state_db_path,
};
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("daemon") => daemon(&args[1..]).await,
        Some("run") => run_once(&args[1..]).await,
        Some("status") => status(&args[1..]),
        Some("ls") => list_active(),
        _ => {
            print_usage();
            Ok(())
        }
    }
}

async fn daemon(args: &[String]) -> Result<()> {
    let pulse_url = option_value(args, "--pulse-url").context("--pulse-url URL is required")?;
    let store = StateStore::open(state_db_path())?;
    subscribe_forever(&store, pulse_url).await
}

async fn run_once(args: &[String]) -> Result<()> {
    let policy = Policy::parse(
        option_value(args, "--policy")
            .unwrap_or("default-flow")
            .trim(),
    )?;
    let dispatch = option_value(args, "--dispatch").context("--dispatch agent is required")?;
    let task = option_value(args, "--task").context("--task text is required")?;
    let store = StateStore::open(state_db_path())?;
    let chain = new_chain(policy, dispatch.to_string(), task.to_string());
    store.insert_chain(&chain)?;
    println!("{}", chain.id);
    println!(
        "dispatch stub: would start {} with policy {}",
        chain.dispatch_agent, chain.policy
    );
    println!("tail stub: run `gild-chain daemon --pulse-url URL` to consume pulse events");
    Ok(())
}

fn status(args: &[String]) -> Result<()> {
    let chain_id = args.first().context("chain_id is required")?;
    let store = StateStore::open(state_db_path())?;
    match store.get_chain(chain_id)? {
        Some(chain) => print_chain(&chain),
        None => bail!("chain {chain_id} not found"),
    }
    Ok(())
}

fn list_active() -> Result<()> {
    let store = StateStore::open(state_db_path())?;
    for chain in store.list_active()? {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            chain.id, chain.policy, chain.status, chain.dispatch_agent, chain.task
        );
    }
    Ok(())
}

async fn subscribe_forever(store: &StateStore, pulse_url: &str) -> Result<()> {
    loop {
        if let Err(err) = subscribe_once(store, pulse_url).await {
            eprintln!("gild-chain pulse subscription error: {err:#}");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}

async fn subscribe_once(store: &StateStore, pulse_url: &str) -> Result<()> {
    let response = reqwest::Client::new()
        .get(pulse_url)
        .query(&[("kind", "agent.run.completed")])
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
                    handle_event(store, event)?;
                }
            }
        }
    }
    Ok(())
}

fn handle_event(store: &StateStore, event: gild_chain::PulseEvent) -> Result<()> {
    let chain_id = event_chain_id(&event).to_string();
    let Some(mut chain) = store.get_chain(&chain_id)? else {
        eprintln!("ignoring event for unknown chain {chain_id}");
        return Ok(());
    };
    let actions = apply_event(&mut chain, &event);
    store.save_chain(&chain)?;
    store.record_event(&chain_id, &event, &actions)?;
    for action in actions {
        emit_action(&action);
    }
    Ok(())
}

fn emit_action(action: &Action) {
    match action {
        Action::RequestReview {
            reviewer,
            pr_number,
        } => println!("stub: request {reviewer} review for PR #{pr_number}"),
        Action::MergePullRequest { pr_number } => {
            println!("stub: merge PR #{pr_number} through git-server API")
        }
        Action::CloseIssue => println!("stub: close linked issue"),
        Action::Log(message) => println!("note: {message}"),
    }
    let _ = io::stdout().flush();
}

fn print_chain(chain: &Chain) {
    println!("id: {}", chain.id);
    println!("policy: {}", chain.policy);
    println!("status: {}", chain.status);
    println!("dispatch: {}", chain.dispatch_agent);
    println!("task: {}", chain.task);
    if let Some(pr_number) = chain.pr_number {
        println!("pr: #{pr_number}");
    }
    if let Some(review_status) = &chain.review_status {
        println!("review: {review_status}");
    }
    if let Some(last_event) = &chain.last_event {
        println!("last_event: {last_event}");
    }
}

fn option_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].as_str())
}

fn print_usage() {
    eprintln!(
        "usage:\n  gild-chain daemon --pulse-url URL\n  gild-chain run --policy default --dispatch agent-X --task '...'\n  gild-chain status <chain_id>\n  gild-chain ls"
    );
}
