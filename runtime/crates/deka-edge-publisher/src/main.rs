mod neo4j_reader;
mod redis_writer;
mod state;
mod zega_writer;

use anyhow::{Context, Result};
use neo4j_reader::{EdgeSnapshot, Neo4jReader};
use redis_writer::RedisWriter;
use state::{load_state, save_state};
use std::path::PathBuf;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env();
    let reader = Neo4jReader::connect(
        &config.neo4j_uri,
        &config.neo4j_user,
        &config.neo4j_password,
    )
    .await?;
    let writer = RedisWriter::new(&config.redis_url)?;
    let mut state = load_state(&config.state_dir)?;

    let full = reader
        .full_sync()
        .await
        .context("initial full sync failed")?;
    let full_max = apply_snapshot(&writer, full).await?;
    if full_max > state.last_seen_timestamp {
        state.last_seen_timestamp = full_max;
        save_state(&config.state_dir, state)?;
    }
    eprintln!(
        "[deka-edge-publisher] full sync complete; last_seen={}",
        state.last_seen_timestamp
    );

    let mut interval = tokio::time::interval(config.poll_interval);
    loop {
        interval.tick().await;
        match reader.changes_since(state.last_seen_timestamp).await {
            Ok(snapshot) => match apply_snapshot(&writer, snapshot).await {
                Ok(max_seen) if max_seen > state.last_seen_timestamp => {
                    state.last_seen_timestamp = max_seen;
                    if let Err(err) = save_state(&config.state_dir, state) {
                        eprintln!("[deka-edge-publisher] failed to save state: {err:#}");
                    }
                }
                Ok(_) => {}
                Err(err) => eprintln!("[deka-edge-publisher] redis write failed: {err:#}"),
            },
            Err(err) => eprintln!("[deka-edge-publisher] neo4j poll failed: {err:#}"),
        }
    }
}

async fn apply_snapshot(writer: &RedisWriter, snapshot: EdgeSnapshot) -> Result<i64> {
    let mut max_seen = 0;

    for shop in snapshot.shops {
        max_seen = max_seen.max(shop.updated_at);
        writer.apply_shop(&shop).await?;
        if zega_writer::zega_writes_enabled()
            && let Err(e) = zega_writer::write_shop_subdomain(&shop)
        {
            eprintln!("[deka-edge-publisher] zega write failed: {e:#}");
        }
    }

    for domain in snapshot.domains {
        max_seen = max_seen.max(domain.updated_at);
        writer.apply_domain(&domain).await?;
        if zega_writer::zega_writes_enabled()
            && let Err(e) = zega_writer::write_domain_records(&domain)
        {
            eprintln!("[deka-edge-publisher] zega domain write failed: {e:#}");
        }
    }

    Ok(max_seen)
}

struct Config {
    neo4j_uri: String,
    neo4j_user: String,
    neo4j_password: String,
    redis_url: String,
    state_dir: PathBuf,
    poll_interval: Duration,
}

impl Config {
    fn from_env() -> Self {
        let poll_secs = std::env::var("POLL_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .unwrap_or(5);

        Self {
            neo4j_uri: std::env::var("NEO4J_URI")
                .unwrap_or_else(|_| "bolt://localhost:7687".to_string()),
            neo4j_user: std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string()),
            neo4j_password: std::env::var("NEO4J_PASSWORD").unwrap_or_default(),
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".to_string()),
            state_dir: std::env::var("STATE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/var/lib/deka-edge-publisher")),
            poll_interval: Duration::from_secs(poll_secs),
        }
    }
}
