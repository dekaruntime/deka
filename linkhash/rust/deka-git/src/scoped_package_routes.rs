use axum::{
    extract::{Path, Query, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::package_routes::BlobQuery;
use crate::{auth, packages};

pub(crate) async fn handle_list_scoped_versions(
    Path((scope, name)): Path<(String, String)>,
    req: Request,
) -> impl IntoResponse {
    let pkg_name = format!("@{}/{}", scope, name);
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
    match packages::get_package(&pkg_name).await {
        Ok(summary) => (StatusCode::OK, Json(serde_json::json!(summary))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_get_scoped_release(
    Path((scope, name, version)): Path<(String, String, String)>,
    req: Request,
) -> impl IntoResponse {
    let pkg_name = format!("@{}/{}", scope, name);
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
    match packages::get_release(&pkg_name, &version).await {
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

pub(crate) async fn handle_get_scoped_latest(
    Path((scope, name)): Path<(String, String)>,
    req: Request,
) -> impl IntoResponse {
    let pkg_name = format!("@{}/{}", scope, name);
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
    match packages::get_latest_release(&pkg_name).await {
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

pub(crate) async fn handle_get_scoped_docs(
    Path((scope, name, version)): Path<(String, String, String)>,
    req: Request,
) -> impl IntoResponse {
    let pkg_name = format!("@{}/{}", scope, name);
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
    match packages::get_release_docs(&pkg_name, &version).await {
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

pub(crate) async fn handle_get_scoped_tree(
    Path((scope, name, version)): Path<(String, String, String)>,
    req: Request,
) -> impl IntoResponse {
    let pkg_name = format!("@{}/{}", scope, name);
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
    match packages::get_release_tree(&pkg_name, &version).await {
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

pub(crate) async fn handle_get_scoped_blob(
    Path((scope, name, version)): Path<(String, String, String)>,
    Query(query): Query<BlobQuery>,
    req: Request,
) -> impl IntoResponse {
    let pkg_name = format!("@{}/{}", scope, name);
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
    match packages::get_release_blob(&pkg_name, &version, path).await {
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
