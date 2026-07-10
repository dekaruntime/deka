use axum::{
    extract::{Path, Request},
    http::StatusCode,
    response::IntoResponse,
};

use crate::issues;

use super::shared::{json_error, json_response, parse_json_body, require_auth_user};

pub(crate) async fn handle_add_commit_ref(
    Path((owner, repo, number)): Path<(String, String, i64)>,
    req: Request,
) -> impl IntoResponse {
    if let Err(response) = require_auth_user(&req) {
        return response;
    }

    let cr_req: issues::CreateCommitRefRequest = match parse_json_body(req).await {
        Ok(request) => request,
        Err(response) => return response,
    };

    let issue = match issues::get_issue(&owner, &repo, number).await {
        Ok(Some(issue)) => issue,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "Issue not found"),
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };

    match issues::add_commit_ref(issue.id, &cr_req.commit_hash, &cr_req.repo).await {
        Ok(cr) => json_response(StatusCode::CREATED, serde_json::json!(cr)),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub(crate) async fn handle_get_commit_refs(
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> impl IntoResponse {
    let issue = match issues::get_issue(&owner, &repo, number).await {
        Ok(Some(issue)) => issue,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "Issue not found"),
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };

    match issues::get_commit_refs(issue.id).await {
        Ok(refs) => json_response(StatusCode::OK, serde_json::json!({ "commit_refs": refs })),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
