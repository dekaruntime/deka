use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::Engine;
use serde::Deserialize;

use crate::{Resolver, Store};

#[derive(Clone)]
struct AppState<S> {
    resolver: Arc<Resolver<S>>,
}

#[derive(Debug, Deserialize)]
struct DnsQuery {
    dns: String,
}

pub fn router<S: Store>(resolver: Arc<Resolver<S>>) -> Router {
    Router::new()
        .route(
            "/dns-query",
            get(get_dns_query::<S>).post(post_dns_query::<S>),
        )
        .with_state(AppState { resolver })
}

pub async fn serve<S: Store>(
    addr: SocketAddr,
    resolver: Arc<Resolver<S>>,
) -> Result<(), std::io::Error> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("deka-dns DoH listening on http://{addr}/dns-query");
    axum::serve(listener, router(resolver)).await
}

async fn get_dns_query<S: Store>(
    State(state): State<AppState<S>>,
    Query(query): Query<DnsQuery>,
) -> Response {
    match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(query.dns.as_bytes()) {
        Ok(bytes) => dns_response(state, &bytes).await,
        Err(_) => (StatusCode::BAD_REQUEST, "invalid dns query parameter").into_response(),
    }
}

async fn post_dns_query<S: Store>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if !content_type.starts_with("application/dns-message") {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "expected application/dns-message",
        )
            .into_response();
    }

    dns_response(state, &body).await
}

async fn dns_response<S: Store>(state: AppState<S>, bytes: &[u8]) -> Response {
    match state.resolver.resolve_bytes(bytes).await {
        Ok(response) => (
            [(header::CONTENT_TYPE, "application/dns-message")],
            response,
        )
            .into_response(),
        Err(err) => {
            tracing::warn!("DoH query failed: {err}");
            (StatusCode::BAD_REQUEST, "invalid DNS query").into_response()
        }
    }
}
