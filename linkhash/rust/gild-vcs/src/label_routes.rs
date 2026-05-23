use axum::{
    extract::{Path, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::issues;

pub(crate) async fn handle_list_labels(
    Path((owner, repo)): Path<(String, String)>,
) -> impl IntoResponse {
    match issues::list_labels(&owner, &repo).await {
        Ok(labels) => (
            StatusCode::OK,
            Json(serde_json::json!({ "labels": labels })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(serde::Deserialize)]
struct CreateLabelRequest {
    name: String,
    color: Option<String>,
    description: Option<String>,
}

pub(crate) async fn handle_create_label(
    Path((owner, repo)): Path<(String, String)>,
    req: Request,
) -> impl IntoResponse {
    let body = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid body" })),
            )
        }
    };

    let label_req: CreateLabelRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    let color = label_req.color.as_deref().unwrap_or("6e7681");
    match issues::create_label(
        &owner,
        &repo,
        &label_req.name,
        color,
        label_req.description.as_deref(),
    )
    .await
    {
        Ok(label) => (StatusCode::CREATED, Json(serde_json::json!(label))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_add_label(
    Path((owner, repo, number, label)): Path<(String, String, i64, String)>,
) -> impl IntoResponse {
    match issues::add_label_to_issue(&owner, &repo, number, &label).await {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "added" })),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Issue or label not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_remove_label(
    Path((owner, repo, number, label)): Path<(String, String, i64, String)>,
) -> impl IntoResponse {
    match issues::remove_label_from_issue(&owner, &repo, number, &label).await {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "removed" })),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Issue or label not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}
