use std::sync::Arc;

use deka_dns::{Config, RedisStore, Resolver, doh, udp};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env();
    let store = RedisStore::new(&config.redis_url)?;
    let resolver = Arc::new(Resolver::new(config.clone(), store));

    let udp_resolver = Arc::clone(&resolver);
    let udp_addr = config.udp_addr;
    let doh_addr = config.doh_addr;

    let udp_task = tokio::spawn(async move { udp::serve(udp_addr, udp_resolver).await });
    let doh_task = tokio::spawn(async move { doh::serve(doh_addr, resolver).await });

    tokio::select! {
        result = udp_task => result??,
        result = doh_task => result??,
    }

    Ok(())
}
