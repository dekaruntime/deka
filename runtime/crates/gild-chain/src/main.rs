use anyhow::{Context, Result, bail};
use gild_chain::{
    Action, Chain, Policy, PulseEvent, StateStore, apply_event, new_chain, state_db_path,
    subscribe_once,
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
    let dispatcher_url = option_value(args, "--dispatcher-url").map(str::to_string);
    let git_api_url = option_value(args, "--git-api-url").map(str::to_string);
    let git_token = option_value(args, "--git-token")
        .map(str::to_string)
        .or_else(|| std::env::var("GILD_CHAIN_GIT_TOKEN").ok());
    let store = StateStore::open(state_db_path())?;
    let executor = ActionExecutor {
        dispatcher_url,
        git_api_url,
        git_token,
    };
    subscribe_forever(&store, pulse_url, &executor).await
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

async fn subscribe_forever(
    store: &StateStore,
    pulse_url: &str,
    executor: &ActionExecutor,
) -> Result<()> {
    loop {
        if let Err(err) = subscribe_once(pulse_url, None, |event| {
            handle_event(store, executor, event)
        })
        .await
        {
            eprintln!("gild-chain pulse subscription error: {err:#}");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}

fn handle_event(store: &StateStore, executor: &ActionExecutor, event: PulseEvent) -> Result<()> {
    let Some(mut chain) = store.find_chain_for_event(&event)? else {
        eprintln!(
            "ignoring event for unknown chain: {}",
            gild_chain::event_name(&event)
        );
        return Ok(());
    };
    let actions = apply_event(&mut chain, &event);
    store.save_chain(&chain)?;
    store.record_event(&chain.id, &event, &actions)?;
    for action in actions {
        executor.emit_action(&action)?;
    }
    Ok(())
}

struct ActionExecutor {
    dispatcher_url: Option<String>,
    git_api_url: Option<String>,
    git_token: Option<String>,
}

impl ActionExecutor {
    fn emit_action(&self, action: &Action) -> Result<()> {
        match action {
            Action::RequestReview {
                reviewer,
                repo,
                pr_number,
            } => self.request_review(reviewer, repo.as_deref(), *pr_number)?,
            Action::MergePullRequest { repo, pr_number } => {
                self.merge_pull_request(repo.as_deref(), *pr_number)?
            }
            Action::Log(message) => println!("note: {message}"),
        }
        let _ = io::stdout().flush();
        Ok(())
    }

    fn request_review(&self, reviewer: &str, repo: Option<&str>, pr_number: i64) -> Result<()> {
        let Some(dispatcher_url) = &self.dispatcher_url else {
            println!("stub: request {reviewer} review for PR #{pr_number}");
            return Ok(());
        };
        let task = match repo {
            Some(repo) => format!("Review {repo} PR #{pr_number}"),
            None => format!("Review PR #{pr_number}"),
        };
        let response = reqwest::blocking::Client::new()
            .post(format!("{}/task", dispatcher_url.trim_end_matches('/')))
            .json(&serde_json::json!({
                "agent": reviewer,
                "task": task,
                "runtime": "codex"
            }))
            .send()
            .context("dispatch review request")?
            .error_for_status()
            .context("review dispatcher status")?;
        println!(
            "requested {reviewer} review for PR #{pr_number}: {}",
            response.status()
        );
        Ok(())
    }

    fn merge_pull_request(&self, repo: Option<&str>, pr_number: i64) -> Result<()> {
        let Some(git_api_url) = &self.git_api_url else {
            println!("stub: merge PR #{pr_number} through git-server API");
            return Ok(());
        };
        let repo = repo.context("repo is required to merge pull request")?;
        let (owner, name) = repo
            .split_once('/')
            .context("repo must be formatted as owner/name")?;
        let mut request = reqwest::blocking::Client::new()
            .patch(format!(
                "{}/api/repos/{owner}/{name}/pulls/{pr_number}",
                git_api_url.trim_end_matches('/')
            ))
            .json(&serde_json::json!({ "state": "merged" }));
        if let Some(token) = &self.git_token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .context("merge pull request")?
            .error_for_status()
            .context("git-server merge status")?;
        println!("merged {repo} PR #{pr_number}: {}", response.status());
        Ok(())
    }
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
    if let Some(run_id) = &chain.run_id {
        println!("run: {run_id}");
    }
    if let Some(repo) = &chain.repo {
        println!("repo: {repo}");
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
