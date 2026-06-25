use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

use crate::auth::{
    PeerCredentials, authorize_peer, forbidden_peer_reply, peer_credentials, prepare_socket_path,
    secure_socket,
};
use crate::commands::{create_agent, not_implemented, remove_agent};
use crate::config::{AppState, GILD_AGENT_ROOT, READ_TIMEOUT_MS};
use crate::protocol::{HttpRequest, ServiceReply, parse_json, read_http_request, write_reply};
use crate::requests::{
    CreateAgentRequest, DeleteUnitRequest, RemoveAgentRequest, RestartUnitRequest,
    RotateHmacRequest, SystemctlRequest, WriteUnitRequest,
};
use crate::systemctl::handle_systemctl_request;
use crate::units::{handle_delete_unit_request, handle_write_unit_request};

pub(crate) async fn serve(state: Arc<AppState>) -> Result<()> {
    prepare_socket_path(&state.socket_path)?;
    let listener = UnixListener::bind(&state.socket_path)
        .with_context(|| format!("bind {}", state.socket_path.display()))?;
    secure_socket(&state.socket_path, state.orchestrator_gid)?;
    eprintln!("gild-agent listening on {}", state.socket_path.display());

    loop {
        let (stream, _) = listener.accept().await.context("accept unix connection")?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, state).await {
                eprintln!("gild-agent connection error: {err:#}");
            }
        });
    }
}

async fn handle_connection(mut stream: UnixStream, state: Arc<AppState>) -> Result<()> {
    let peer = peer_credentials(&stream)?;
    if !authorize_peer(peer, state.orchestrator_gid)? {
        write_reply(&mut stream, forbidden_peer_reply()).await?;
        return Ok(());
    }

    let request = timeout(
        Duration::from_millis(READ_TIMEOUT_MS),
        read_http_request(&mut stream),
    )
    .await
    .context("request read timed out")??;
    let reply = handle_request(&state, &request, peer).await;
    write_reply(&mut stream, reply).await
}

pub(crate) async fn handle_request(
    state: &AppState,
    request: &HttpRequest,
    peer: PeerCredentials,
) -> ServiceReply {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/v1/health") => ServiceReply::json_value(
            200,
            serde_json::json!({
                "ok": true,
                "uptime": state.started_at.elapsed().as_secs()
            }),
        ),
        ("POST", "/v1/agent/create") => match parse_json::<CreateAgentRequest>(request) {
            Ok(body) => create_agent(state, peer, &body),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/agent/remove") => match parse_json::<RemoveAgentRequest>(request) {
            Ok(body) => remove_agent(state, peer, &body),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/systemd/restart") => match parse_json::<RestartUnitRequest>(request) {
            Ok(body) => not_implemented("systemd.restart", &format!("unit {}", body.unit)),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/systemd/reload") => not_implemented("systemd.reload", "daemon-reload"),
        ("POST", "/v1/hmac/rotate") => match parse_json::<RotateHmacRequest>(request) {
            Ok(body) => crate::handlers::hmac_rotate::handle_hmac_rotate_request(
                &body,
                peer,
                Path::new(GILD_AGENT_ROOT),
                &state.audit_log_path,
                Some((0, 0)),
            ),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/systemctl") => match parse_json::<SystemctlRequest>(request) {
            Ok(body) => handle_systemctl_request(state, &body, peer),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/write_unit") => match parse_json::<WriteUnitRequest>(request) {
            Ok(body) => handle_write_unit_request(state, &body, peer),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        ("POST", "/v1/delete_unit") => match parse_json::<DeleteUnitRequest>(request) {
            Ok(body) => handle_delete_unit_request(state, &body, peer),
            Err(err) => {
                ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }))
            }
        },
        (_, "/v1/health")
        | (_, "/v1/agent/create")
        | (_, "/v1/agent/remove")
        | (_, "/v1/systemd/restart")
        | (_, "/v1/systemd/reload")
        | (_, "/v1/hmac/rotate")
        | (_, "/v1/systemctl")
        | (_, "/v1/write_unit")
        | (_, "/v1/delete_unit") => {
            ServiceReply::json_value(405, serde_json::json!({ "error": "method not allowed" }))
        }
        _ => ServiceReply::json_value(404, serde_json::json!({ "error": "not found" })),
    }
}
