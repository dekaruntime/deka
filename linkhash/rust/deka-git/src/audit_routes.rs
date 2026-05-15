use axum::{
    extract::{Query, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::auth;

pub(crate) async fn handle_audit_log(
    Query(query): Query<auth::AuditQuery>,
    req: Request,
) -> impl IntoResponse {
    let _auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    match auth::query_audit_log(&query).await {
        Ok(entries) => (
            StatusCode::OK,
            Json(serde_json::json!({ "entries": entries })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}
