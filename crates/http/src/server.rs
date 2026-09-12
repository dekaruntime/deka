use std::net::SocketAddr;
use std::sync::Arc;

use engine::RuntimeState;

use crate::config::HttpConfig;
use crate::fast::serve_http_fast;
use crate::listener::bind_reuseport;
use crate::rate_limit::RateLimiter;
use crate::router::app_router_with_rate_limiter;

pub async fn serve_http(
    state: Arc<RuntimeState>,
    port: u16,
    listeners: usize,
    perf_mode: bool,
    config: HttpConfig,
) -> Result<(), String> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("🚀 Deka Runtime listening on {}", addr);
    tracing::info!("📦 Loaded modules: deka, postgres, docker, router, t4, sqlite");

    let listener_count = listeners.max(1);
    let rate_limiter = Arc::new(RateLimiter::new(config.rate_limit.clone()));
    rate_limiter.spawn_janitor();
    if listener_count == 1 {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|err| format_bind_error(addr, &err.to_string()))?;
        if perf_mode {
            serve_http_fast(listener, state, Arc::clone(&rate_limiter), config.debug).await;
            return Ok(());
        }

        let app = app_router_with_rate_limiter(
            Arc::clone(&state),
            Arc::clone(&rate_limiter),
            config,
        );
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .map_err(|err| format!("HTTP server exited with error: {}", err))?;
        return Ok(());
    }

    let mut bound_listeners = Vec::with_capacity(listener_count);
    for _ in 0..listener_count {
        let listener = bind_reuseport(addr).map_err(|err| format_bind_error(addr, &err))?;
        bound_listeners.push(listener);
    }

    let mut handles = Vec::with_capacity(listener_count);
    for listener in bound_listeners {
        let state = Arc::clone(&state);
        let config = config.clone();
        if perf_mode {
            let rate_limiter = Arc::clone(&rate_limiter);
            let debug = config.debug;
            handles.push(tokio::spawn(async move {
                serve_http_fast(listener, state, rate_limiter, debug).await;
                Ok::<(), String>(())
            }));
        } else {
            let app = app_router_with_rate_limiter(Arc::clone(&state), Arc::clone(&rate_limiter), config);
            handles.push(tokio::spawn(async move {
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .map_err(|err| format!("HTTP listener exited: {}", err))
            }));
        }
    }

    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(err) => return Err(format!("HTTP listener task failed: {}", err)),
        }
    }

    Ok(())
}

fn format_bind_error(addr: SocketAddr, err: &str) -> String {
    let mut message = format!("failed to bind HTTP listener on {}: {}", addr, err);
    if err.to_ascii_lowercase().contains("address already in use") {
        message.push_str(". Port is already in use. Stop the existing process or pass --port <n>.");
    }
    message
}
