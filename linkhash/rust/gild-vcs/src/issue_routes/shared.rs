use axum::{extract::Request, http::StatusCode, Json};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::auth::{self, AuthUser};

pub(super) type RouteResponse = (StatusCode, Json<Value>);

pub(super) fn json_response(status: StatusCode, value: Value) -> RouteResponse {
    (status, Json(value))
}

pub(super) fn json_error(status: StatusCode, message: impl ToString) -> RouteResponse {
    json_response(status, serde_json::json!({ "error": message.to_string() }))
}

pub(super) fn require_auth_user(req: &Request) -> Result<AuthUser, RouteResponse> {
    auth::get_auth_user(req)
        .cloned()
        .ok_or_else(|| json_error(StatusCode::UNAUTHORIZED, "Authentication required"))
}

pub(super) async fn parse_json_body<T: DeserializeOwned>(req: Request) -> Result<T, RouteResponse> {
    let body = axum::body::to_bytes(req.into_body(), 1024 * 1024)
        .await
        .map_err(|_| json_error(StatusCode::BAD_REQUEST, "Invalid body"))?;

    serde_json::from_slice(&body).map_err(|e| json_error(StatusCode::BAD_REQUEST, e))
}

pub(super) async fn audit_issue_action(
    auth_user: &AuthUser,
    action: &str,
    owner: &str,
    repo: &str,
    details: String,
) {
    auth::log_audit(
        Some(auth_user.token_id),
        &auth_user.key_type,
        &auth_user.owner,
        action,
        Some(&format!("{}/{}", owner, repo)),
        None,
        Some(&details),
        None,
    )
    .await;
}
