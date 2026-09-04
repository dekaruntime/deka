use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::{State, WebSocketUpgrade},
    response::{IntoResponse, Response},
};

use crate::{RuntimeState, WsOptions};
use deka_http::websocket::handle_websocket;

pub async fn serve_ws(state: Arc<RuntimeState>, options: WsOptions) -> Result<(), String> {
    let addr = SocketAddr::from(([0, 0, 0, 0], options.port));
    tracing::info!("🛰️ Deka WebSocket transport listening on {}", addr);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => return Err(format!("WebSocket bind failed: {}", err)),
    };

    let app = Router::new().fallback(handle_upgrade).with_state(state);

    if let Err(err) = axum::serve(listener, app).await {
        return Err(err.to_string());
    }

    Ok(())
}

async fn handle_upgrade(
    ws: Option<WebSocketUpgrade>,
    State(state): State<Arc<RuntimeState>>,
) -> impl IntoResponse {
    if let Some(ws) = ws {
        return ws
            .on_upgrade(|socket| handle_websocket(socket, state, None))
            .into_response();
    }

    Response::builder()
        .status(426)
        .body(Body::from("WebSocket upgrade required"))
        .unwrap()
}
