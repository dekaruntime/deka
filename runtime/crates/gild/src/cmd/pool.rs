use clap::{Args, Subcommand};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

use crate::Result;

#[derive(Debug, Args)]
pub struct PoolArgs {
    #[command(subcommand)]
    command: PoolCommand,
}

#[derive(Debug, Subcommand)]
enum PoolCommand {
    /// Print warm-pool metrics.
    Status,
}

pub async fn run(args: PoolArgs) -> Result<()> {
    match args.command {
        PoolCommand::Status => status().await,
    }
}

async fn status() -> Result<()> {
    let socket = std::env::var("GILD_SOCKET").unwrap_or_else(|_| "/run/gild/sock".to_string());
    let mut stream = UnixStream::connect(socket).await?;
    stream
        .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;

    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or(&response);
    let warm_count = parse_metric(body, "gild_pool_warm_count")
        .ok_or("gild_pool_warm_count not present in /metrics response")?;

    println!("gild_pool_warm_count {}", warm_count);
    Ok(())
}

fn parse_metric(body: &str, name: &str) -> Option<String> {
    body.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let metric = parts.next()?;
            let value = parts.next()?;
            (metric == name).then(|| value.to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::parse_metric;

    #[test]
    fn parses_warm_count_metric() {
        let metrics = "# HELP gild_pool_warm_count warm\n\
            gild_pool_warm_count 3\n";
        assert_eq!(
            parse_metric(metrics, "gild_pool_warm_count").as_deref(),
            Some("3")
        );
    }
}
