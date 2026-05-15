use axum::{
    extract::{Path, Query, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::{auth, git, hooks};

pub(crate) async fn handle_info_refs_public(
    Path((owner, repo)): Path<(String, String)>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    req: Request,
) -> Response {
    let repo_name = repo.strip_suffix(".git").unwrap_or(&repo);
    let service = match params.get("service") {
        Some(s) => s.as_str(),
        None => return (StatusCode::BAD_REQUEST, "Missing service parameter").into_response(),
    };

    let is_read = service == "git-upload-pack";
    let is_public = auth::is_repo_public(&owner, repo_name).await;

    // Write operations always require auth
    if !is_read {
        let auth_user = match auth::get_auth_user(&req) {
            Some(user) => user,
            None => {
                return (StatusCode::UNAUTHORIZED, "Authentication required for push")
                    .into_response()
            }
        };
        if !auth_user.has_scope("repo:write") {
            return (StatusCode::FORBIDDEN, "repo:write scope required").into_response();
        }
        if !auth_user.can_access_repo(repo_name) && !auth_user.can_access_repo(&repo) {
            return (StatusCode::FORBIDDEN, "Access denied to this repository").into_response();
        }
        auth::log_audit(
            Some(auth_user.token_id),
            &auth_user.key_type,
            &auth_user.owner,
            "push.info_refs",
            Some(&format!("{}/{}", owner, repo)),
            None,
            None,
            None,
        )
        .await;
    } else if !is_public {
        // Private repo read requires auth
        let auth_user = match auth::get_auth_user(&req) {
            Some(user) => user,
            None => return (StatusCode::UNAUTHORIZED, "Authentication required").into_response(),
        };
        if !auth_user.has_scope("repo:read") {
            return (StatusCode::FORBIDDEN, "repo:read scope required").into_response();
        }
        if !auth_user.can_access_repo(repo_name) && !auth_user.can_access_repo(&repo) {
            return (StatusCode::FORBIDDEN, "Access denied to this repository").into_response();
        }
        auth::log_audit(
            Some(auth_user.token_id),
            &auth_user.key_type,
            &auth_user.owner,
            "fetch.info_refs",
            Some(&format!("{}/{}", owner, repo)),
            None,
            None,
            None,
        )
        .await;
    }
    // Public repo read: no auth needed, proceed

    match git::protocol::advertise_refs(&owner, &repo, service).await {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("Failed to advertise refs: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// upload-pack handler that allows public repo reads without auth.
pub(crate) async fn handle_upload_pack_public(
    Path((owner, repo)): Path<(String, String)>,
    req: Request,
) -> Response {
    let repo_name = repo.strip_suffix(".git").unwrap_or(&repo);
    let is_public = auth::is_repo_public(&owner, repo_name).await;

    if !is_public {
        let auth_user = match auth::get_auth_user(&req) {
            Some(user) => user.clone(),
            None => return (StatusCode::UNAUTHORIZED, "Authentication required").into_response(),
        };
        if !auth_user.has_scope("repo:read") {
            return (StatusCode::FORBIDDEN, "repo:read scope required").into_response();
        }
        if !auth_user.can_access_repo(repo_name) && !auth_user.can_access_repo(&repo) {
            return (StatusCode::FORBIDDEN, "Access denied to this repository").into_response();
        }
        let body = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!("Failed to read request body: {}", e);
                return (StatusCode::BAD_REQUEST, "Failed to read body").into_response();
            }
        };
        auth::log_audit(
            Some(auth_user.token_id),
            &auth_user.key_type,
            &auth_user.owner,
            "clone",
            Some(&format!("{}/{}", owner, repo)),
            None,
            None,
            None,
        )
        .await;
        match git::upload_pack::handle(&owner, &repo, body).await {
            Ok(response) => response,
            Err(e) => {
                tracing::error!("upload-pack failed: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    } else {
        // Public repo: no auth required for reads
        let body = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!("Failed to read request body: {}", e);
                return (StatusCode::BAD_REQUEST, "Failed to read body").into_response();
            }
        };
        match git::upload_pack::handle(&owner, &repo, body).await {
            Ok(response) => response,
            Err(e) => {
                tracing::error!("upload-pack failed: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    }
}

pub(crate) async fn handle_receive_pack(
    Path((owner, repo)): Path<(String, String)>,
    req: Request,
) -> Response {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user.clone(),
        None => return (StatusCode::UNAUTHORIZED, "Authentication required").into_response(),
    };

    if !auth_user.has_scope("repo:write") {
        return (StatusCode::FORBIDDEN, "repo:write scope required").into_response();
    }

    let repo_name = repo.strip_suffix(".git").unwrap_or(&repo);
    if !auth_user.can_access_repo(repo_name) && !auth_user.can_access_repo(&repo) {
        return (StatusCode::FORBIDDEN, "Access denied to this repository").into_response();
    }

    let body = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to read request body: {}", e);
            return (StatusCode::BAD_REQUEST, "Failed to read body").into_response();
        }
    };

    auth::log_audit(
        Some(auth_user.token_id),
        &auth_user.key_type,
        &auth_user.owner,
        "push",
        Some(&format!("{}/{}", owner, repo)),
        None,
        None,
        None,
    )
    .await;

    // Keep a copy of the body for post-receive hook parsing
    let body_bytes = body.to_vec();

    match git::receive_pack::handle(&owner, &repo, body).await {
        Ok(response) => {
            // Post-receive hook: trigger rebuild if main was pushed
            let hook_owner = owner.clone();
            let hook_repo = repo.clone();
            let hook_token_id = auth_user.token_id;
            let hook_key_type = auth_user.key_type.clone();
            let hook_auth_owner = auth_user.owner.clone();
            tokio::spawn(async move {
                hooks::run_post_receive(
                    &hook_owner,
                    &hook_repo,
                    &body_bytes,
                    Some(hook_token_id),
                    &hook_key_type,
                    &hook_auth_owner,
                )
                .await;
            });
            response
        }
        Err(e) => {
            tracing::error!("receive-pack failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
