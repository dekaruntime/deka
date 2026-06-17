mod audit;
mod auth;
mod commands;
mod config;
mod handlers;
mod protocol;
mod requests;
mod server;
mod systemctl;
mod units;

#[cfg(test)]
mod tests;

use anyhow::Result;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let state = Arc::new(config::load_state()?);
    server::serve(state).await
}
