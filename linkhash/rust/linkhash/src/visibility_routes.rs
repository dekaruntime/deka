use axum::{
    extract::{Path, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::auth;

pub(crate) async fn handle_get_visibility(
    Path((owner, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let visibility = auth::get_repo_visibility(&owner, &name).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "owner": owner,
            "repo": name,
            "visibility": visibility
        })),
    )
}

#[derive(Debug, Deserialize)]
struct SetVisibilityRequest {
    visibility: String,
}

pub(crate) async fn handle_set_visibility(
    Path((owner, name)): Path<(String, String)>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user.clone(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    // Requires: system token, repo owner, wildcard scope, or repo:write on the repo
    let is_system = auth_user.key_type == "system";
    let is_owner = auth_user.owner == owner;
    let has_wildcard = auth_user.has_scope("*");
    let has_write = auth_user.has_scope("repo:write")
        && auth_user.can_access_repo(&format!("{}/{}", owner, name));
    if !is_system && !is_owner && !has_wildcard && !has_write {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Insufficient permissions to change visibility" })),
        );
    }

    let body = match axum::body::to_bytes(req.into_body(), 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid body" })),
            )
        }
    };

    let vis_req: SetVisibilityRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    if vis_req.visibility != "public" && vis_req.visibility != "private" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "visibility must be 'public' or 'private'" })),
        );
    }

    match auth::set_repo_visibility(&owner, &name, &vis_req.visibility).await {
        Ok(()) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "repo.visibility",
                Some(&format!("{}/{}", owner, name)),
                None,
                Some(&format!("set to {}", vis_req.visibility)),
                None,
            )
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "owner": owner,
                    "repo": name,
                    "visibility": vis_req.visibility
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

// --- Git protocol handlers (public-aware) ---
