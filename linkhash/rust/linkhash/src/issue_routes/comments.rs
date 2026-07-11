use axum::{
    extract::{Path, Request},
    http::StatusCode,
    response::IntoResponse,
};

use crate::issues;

use super::shared::{
    audit_issue_action, json_error, json_response, parse_json_body, require_auth_user,
};

pub(crate) async fn handle_list_comments(
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> impl IntoResponse {
    match issues::list_comments(&owner, &repo, number).await {
        Ok(comments) => json_response(StatusCode::OK, serde_json::json!({ "comments": comments })),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub(crate) async fn handle_create_comment(
    Path((owner, repo, number)): Path<(String, String, i64)>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match require_auth_user(&req) {
        Ok(user) => user,
        Err(response) => return response,
    };

    let comment_req: issues::CreateCommentRequest = match parse_json_body(req).await {
        Ok(request) => request,
        Err(response) => return response,
    };

    match issues::add_comment(&owner, &repo, number, &auth_user.owner, comment_req).await {
        Ok(Some(comment)) => {
            audit_issue_action(
                &auth_user,
                "issue.comment",
                &owner,
                &repo,
                format!("#{}", number),
            )
            .await;
            json_response(StatusCode::CREATED, serde_json::json!(comment))
        }
        Ok(None) => json_error(StatusCode::NOT_FOUND, "Issue not found"),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
