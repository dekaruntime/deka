use axum::{
    extract::{Path, Query, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::{auth, pulls};

pub(crate) async fn handle_list_pulls(
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<pulls::ListPullsQuery>,
) -> impl IntoResponse {
    match pulls::list_pulls(&owner, &repo, &query).await {
        Ok(list) => (StatusCode::OK, Json(serde_json::json!({ "pulls": list }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_create_pull(
    Path((owner, repo)): Path<(String, String)>,
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

    let body = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid body" })),
            )
        }
    };

    let create_req: pulls::CreatePullRequest = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    match pulls::create_pull(&owner, &repo, &auth_user.owner, create_req).await {
        Ok(pr) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "pr.create",
                Some(&format!("{}/{}", owner, repo)),
                None,
                Some(&format!("#{}: {}", pr.number, pr.title)),
                None,
            )
            .await;
            (StatusCode::CREATED, Json(serde_json::json!(pr)))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_get_pull(
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> impl IntoResponse {
    match pulls::get_pull(&owner, &repo, number).await {
        Ok(Some(pr)) => {
            let comments = pulls::list_pull_comments(&owner, &repo, number)
                .await
                .unwrap_or_default();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "pull": pr, "comments": comments })),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Pull request not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_update_pull(
    Path((owner, repo, number)): Path<(String, String, i64)>,
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

    let body = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid body" })),
            )
        }
    };
    let update_req: pulls::UpdatePullRequest = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };
    match pulls::update_pull(&owner, &repo, number, update_req).await {
        Ok(Some(pr)) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "pr.update",
                Some(&format!("{}/{}", owner, repo)),
                None,
                Some(&format!("#{} -> {}", number, pr.state)),
                None,
            )
            .await;
            (StatusCode::OK, Json(serde_json::json!(pr)))
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Pull request not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_list_pull_comments(
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> impl IntoResponse {
    match pulls::list_pull_comments(&owner, &repo, number).await {
        Ok(comments) => (
            StatusCode::OK,
            Json(serde_json::json!({ "comments": comments })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_create_pull_comment(
    Path((owner, repo, number)): Path<(String, String, i64)>,
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
    let body = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid body" })),
            )
        }
    };
    let create_req: pulls::CreatePullComment = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };
    match pulls::add_pull_comment(&owner, &repo, number, &auth_user.owner, create_req).await {
        Ok(Some(comment)) => (StatusCode::CREATED, Json(serde_json::json!(comment))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Pull request not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

// --- Package registry ---
