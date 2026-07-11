use axum::{
    extract::{Path, Query, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::{issues, repo};

use super::shared::{
    audit_issue_action, json_error, json_response, parse_json_body, require_auth_user,
};

pub(crate) async fn handle_list_issues(
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<issues::ListIssuesQuery>,
) -> impl IntoResponse {
    match issues::list_issues(&owner, &repo, &query).await {
        Ok(list) => json_response(StatusCode::OK, serde_json::json!({ "issues": list })),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub(crate) async fn handle_list_all_issues(
    Query(query): Query<issues::ListIssuesQuery>,
    req: Request,
) -> impl IntoResponse {
    if let Err(response) = require_auth_user(&req) {
        return response;
    }

    let mut all_issues = Vec::new();
    let project_repos = [
        "tana",
        "deka",
        "tana-website",
        "tana-admin",
        "tana-store-admin",
    ];

    for repo_name in &project_repos {
        if let Ok(mut list) = issues::list_issues("tana", repo_name, &query).await {
            all_issues.append(&mut list);
        }
    }

    if let Ok(repos) = repo::storage::list_repos("tana") {
        for repo_name in &repos {
            if !project_repos.contains(&repo_name.as_str()) {
                if let Ok(mut list) = issues::list_issues("tana", repo_name, &query).await {
                    all_issues.append(&mut list);
                }
            }
        }
    }

    all_issues.sort_by(|a, b| b.number.cmp(&a.number));

    json_response(StatusCode::OK, serde_json::json!({ "issues": all_issues }))
}

pub(crate) async fn handle_create_issue(
    Path((owner, repo)): Path<(String, String)>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match require_auth_user(&req) {
        Ok(user) => user,
        Err(response) => return response,
    };

    if !auth_user.has_scope("issues:write") {
        return json_error(StatusCode::FORBIDDEN, "issues:write scope required");
    }

    let create_req: issues::CreateIssueRequest = match parse_json_body(req).await {
        Ok(request) => request,
        Err(response) => return response,
    };

    match issues::create_issue(&owner, &repo, &auth_user.owner, create_req).await {
        Ok(issue) => {
            audit_issue_action(
                &auth_user,
                "issue.create",
                &owner,
                &repo,
                format!("#{}: {}", issue.number, issue.title),
            )
            .await;
            json_response(StatusCode::CREATED, serde_json::json!(issue))
        }
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub(crate) async fn handle_get_issue(
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> impl IntoResponse {
    match issues::get_issue(&owner, &repo, number).await {
        Ok(Some(issue)) => {
            let comments = issues::list_comments(&owner, &repo, number)
                .await
                .unwrap_or_default();
            let labels = issues::get_issue_labels(&owner, &repo, number)
                .await
                .unwrap_or_default();
            let commit_refs = issues::get_commit_refs(issue.id).await.unwrap_or_default();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "issue": issue,
                    "comments": comments,
                    "labels": labels,
                    "commit_refs": commit_refs
                })),
            )
        }
        Ok(None) => json_error(StatusCode::NOT_FOUND, "Issue not found"),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub(crate) async fn handle_update_issue(
    Path((owner, repo, number)): Path<(String, String, i64)>,
    req: Request,
) -> impl IntoResponse {
    let auth_user = match require_auth_user(&req) {
        Ok(user) => user,
        Err(response) => return response,
    };

    let update_req: issues::UpdateIssueRequest = match parse_json_body(req).await {
        Ok(request) => request,
        Err(response) => return response,
    };

    match issues::update_issue(&owner, &repo, number, &auth_user.owner, update_req).await {
        Ok(Some(issue)) => {
            audit_issue_action(
                &auth_user,
                "issue.update",
                &owner,
                &repo,
                format!("#{} -> {}", number, issue.state),
            )
            .await;
            json_response(StatusCode::OK, serde_json::json!(issue))
        }
        Ok(None) => json_error(StatusCode::NOT_FOUND, "Issue not found"),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
