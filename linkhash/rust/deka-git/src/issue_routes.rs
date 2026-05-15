use axum::{
    extract::{Path, Query, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::{auth, issues, repo};

pub(crate) async fn handle_list_issues(
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<issues::ListIssuesQuery>,
) -> impl IntoResponse {
    match issues::list_issues(&owner, &repo, &query).await {
        Ok(list) => (StatusCode::OK, Json(serde_json::json!({ "issues": list }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_list_all_issues(
    Query(query): Query<issues::ListIssuesQuery>,
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

    // Query issues across all repos under "tana" owner
    // For now, search across all known project repos
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
    // Also check store repos
    if let Ok(repos) = repo::storage::list_repos("tana") {
        for repo_name in &repos {
            if !project_repos.contains(&repo_name.as_str()) {
                if let Ok(mut list) = issues::list_issues("tana", repo_name, &query).await {
                    all_issues.append(&mut list);
                }
            }
        }
    }

    // Sort by number descending
    all_issues.sort_by(|a, b| b.number.cmp(&a.number));

    (
        StatusCode::OK,
        Json(serde_json::json!({ "issues": all_issues })),
    )
}

pub(crate) async fn handle_create_issue(
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

    if !auth_user.has_scope("issues:write") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "issues:write scope required" })),
        );
    }

    let body = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid body" })),
            )
        }
    };

    let create_req: issues::CreateIssueRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    match issues::create_issue(&owner, &repo, &auth_user.owner, create_req).await {
        Ok(issue) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "issue.create",
                Some(&format!("{}/{}", owner, repo)),
                None,
                Some(&format!("#{}: {}", issue.number, issue.title)),
                None,
            )
            .await;
            (StatusCode::CREATED, Json(serde_json::json!(issue)))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
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
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Issue not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_update_issue(
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

    let update_req: issues::UpdateIssueRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    match issues::update_issue(&owner, &repo, number, update_req).await {
        Ok(Some(issue)) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "issue.update",
                Some(&format!("{}/{}", owner, repo)),
                None,
                Some(&format!("#{} -> {}", number, issue.state)),
                None,
            )
            .await;
            (StatusCode::OK, Json(serde_json::json!(issue)))
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Issue not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_list_comments(
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> impl IntoResponse {
    match issues::list_comments(&owner, &repo, number).await {
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

pub(crate) async fn handle_create_comment(
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

    let comment_req: issues::CreateCommentRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    match issues::add_comment(&owner, &repo, number, &auth_user.owner, comment_req).await {
        Ok(Some(comment)) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "issue.comment",
                Some(&format!("{}/{}", owner, repo)),
                None,
                Some(&format!("#{}", number)),
                None,
            )
            .await;
            (StatusCode::CREATED, Json(serde_json::json!(comment)))
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Issue not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_add_commit_ref(
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

    let cr_req: issues::CreateCommitRefRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    // Get the issue first
    let issue = match issues::get_issue(&owner, &repo, number).await {
        Ok(Some(i)) => i,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Issue not found" })),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    match issues::add_commit_ref(issue.id, &cr_req.commit_hash, &cr_req.repo).await {
        Ok(cr) => (StatusCode::CREATED, Json(serde_json::json!(cr))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_get_commit_refs(
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> impl IntoResponse {
    let issue = match issues::get_issue(&owner, &repo, number).await {
        Ok(Some(i)) => i,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Issue not found" })),
            )
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    match issues::get_commit_refs(issue.id).await {
        Ok(refs) => (
            StatusCode::OK,
            Json(serde_json::json!({ "commit_refs": refs })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}
