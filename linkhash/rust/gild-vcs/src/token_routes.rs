use axum::{
    extract::{Path, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::auth;

#[derive(Debug, Deserialize)]
pub(crate) struct CreateTokenPayload {
    key_type: String,
    owner: String,
    scopes: Option<Vec<String>>,
    repos: Option<Vec<String>>,
    expires_in_days: Option<i64>,
}

pub(crate) async fn handle_create_token(
    Json(payload): Json<CreateTokenPayload>,
) -> impl IntoResponse {
    let req = auth::CreateTokenRequest {
        key_type: payload.key_type,
        owner: payload.owner,
        scopes: payload.scopes,
        repos: payload.repos,
        expires_in_days: payload.expires_in_days,
    };

    match auth::create_token(req).await {
        Ok(result) => {
            auth::log_audit(
                Some(result.id),
                &result.key_type,
                &result.owner,
                "token.create",
                None,
                None,
                Some(&format!("scopes: {:?}", result.scopes)),
                None,
            )
            .await;
            (StatusCode::CREATED, Json(serde_json::json!(result)))
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_list_tokens(req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    match auth::list_tokens().await {
        Ok(tokens) => (
            StatusCode::OK,
            Json(serde_json::json!({ "tokens": tokens })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_revoke_token(Path(id): Path<i64>, req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user.clone(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    match auth::revoke_token(id).await {
        Ok(true) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "token.revoke",
                None,
                None,
                Some(&format!("revoked token_id={}", id)),
                None,
            )
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "revoked" })),
            )
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Token not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}
