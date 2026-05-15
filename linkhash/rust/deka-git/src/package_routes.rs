use axum::{
    extract::{Path, Query, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::{auth, packages};

#[derive(Debug, Deserialize)]
pub(crate) struct BlobQuery {
    pub(crate) path: Option<String>,
}

pub(crate) async fn handle_list_packages(req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    if !auth_user.has_scope("packages:read") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "packages:read scope required" })),
        );
    }

    match packages::list_all_packages().await {
        Ok(list) => (
            StatusCode::OK,
            Json(serde_json::json!({ "packages": list })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_preflight_publish(req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user.clone(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    if !auth_user.has_scope("packages:write") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "packages:write scope required" })),
        );
    }

    let body = match axum::body::to_bytes(req.into_body(), 2 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid body" })),
            )
        }
    };

    let publish_req: packages::PublishPackageRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    auth::log_audit(
        Some(auth_user.token_id),
        &auth_user.key_type,
        &auth_user.owner,
        "package.preflight",
        Some(&format!("{}/{}", auth_user.owner, publish_req.repo)),
        Some(&publish_req.version),
        Some(&publish_req.name),
        None,
    )
    .await;

    match packages::preflight_publish(&auth_user.owner, &publish_req).await {
        Ok(result) => (StatusCode::OK, Json(serde_json::json!(result))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_publish_package(req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user.clone(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    if !auth_user.has_scope("packages:write") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "packages:write scope required" })),
        );
    }

    let body = match axum::body::to_bytes(req.into_body(), 2 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid body" })),
            )
        }
    };

    let publish_req: packages::PublishPackageRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    auth::log_audit(
        Some(auth_user.token_id),
        &auth_user.key_type,
        &auth_user.owner,
        "package.publish",
        Some(&format!("{}/{}", auth_user.owner, publish_req.repo)),
        Some(&publish_req.version),
        Some(&publish_req.name),
        None,
    )
    .await;

    match packages::publish(&auth_user.owner, publish_req).await {
        Ok(release) => (StatusCode::CREATED, Json(serde_json::json!(release))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_list_versions(
    Path(name): Path<String>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    if !auth_user.has_scope("packages:read") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "packages:read scope required" })),
        );
    }

    match packages::get_package(&name).await {
        Ok(summary) => (StatusCode::OK, Json(serde_json::json!(summary))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_get_release(
    Path((name, version)): Path<(String, String)>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    if !auth_user.has_scope("packages:read") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "packages:read scope required" })),
        );
    }

    match packages::get_release(&name, &version).await {
        Ok(Some(release)) => (StatusCode::OK, Json(serde_json::json!(release))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Release not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_get_latest(Path(name): Path<String>, req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    if !auth_user.has_scope("packages:read") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "packages:read scope required" })),
        );
    }

    match packages::get_latest_release(&name).await {
        Ok(Some(release)) => (StatusCode::OK, Json(serde_json::json!(release))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "No releases found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_get_docs(
    Path((name, version)): Path<(String, String)>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    if !auth_user.has_scope("packages:read") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "packages:read scope required" })),
        );
    }

    match packages::get_release_docs(&name, &version).await {
        Ok(Some(docs)) => (StatusCode::OK, Json(serde_json::json!(docs))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Release not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_get_tree(
    Path((name, version)): Path<(String, String)>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    if !auth_user.has_scope("packages:read") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "packages:read scope required" })),
        );
    }

    match packages::get_release_tree(&name, &version).await {
        Ok(Some(tree)) => (StatusCode::OK, Json(serde_json::json!(tree))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Release not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_get_blob(
    Path((name, version)): Path<(String, String)>,
    Query(query): Query<BlobQuery>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    if !auth_user.has_scope("packages:read") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "packages:read scope required" })),
        );
    }

    let path = query.path.as_deref().unwrap_or("");
    match packages::get_release_blob(&name, &version, path).await {
        Ok(Some(blob)) => (StatusCode::OK, Json(serde_json::json!(blob))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "File not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}
