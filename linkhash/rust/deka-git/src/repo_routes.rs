use axum::{
    extract::{Path, Request},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::{auth, config::Config, repo};

pub(crate) async fn handle_create_repo(
    Path(repo): Path<String>,
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

    if !auth_user.has_scope("repo:write") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "repo:write scope required" })),
        );
    }

    match repo::storage::create_bare_repo(&auth_user.owner, &repo) {
        Ok(path) => {
            // Seed labels for the new repo
            auth::seed_labels(&auth_user.owner, &repo).await;

            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "repo.create",
                Some(&format!("{}/{}", auth_user.owner, repo)),
                None,
                None,
                None,
            )
            .await;

            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "status": "created",
                    "owner": auth_user.owner,
                    "repo": repo,
                    "path": path.display().to_string()
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
        ),
    }
}

pub(crate) async fn handle_list_repos(req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    // List repos for the "tana" owner (all store repos live under tana/)
    match repo::storage::list_repos("tana") {
        Ok(repos) => (
            StatusCode::OK,
            Json(serde_json::json!({ "owner": "tana", "repos": repos })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

// --- Store repo initialization ---

pub(crate) async fn handle_init_store_repo(
    Path((owner, name)): Path<(String, String)>,
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

    if !auth_user.has_scope("repo:write") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "repo:write scope required" })),
        );
    }

    // Resolve the default store template directory.
    // Convention: TANA_STORE_ROOT env var points to tana/store/, or we derive
    // it from the repos directory (repos is at store/repos/, template is at store/default/).
    let template_dir = {
        let config = Config::load();
        let repos_path = std::path::PathBuf::from(&config.repos_dir);
        // repos_dir is typically .../store/repos — go up one level for store root
        repos_path
            .parent()
            .map(|p| p.join("default"))
            .unwrap_or_else(|| std::path::PathBuf::from("store/default"))
    };

    match repo::storage::create_and_seed_repo(&owner, &name, &template_dir) {
        Ok(path) => {
            // Seed labels for the new repo
            auth::seed_labels(&owner, &name).await;

            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "repo.init",
                Some(&format!("{}/{}", owner, name)),
                None,
                Some("seeded from default template"),
                None,
            )
            .await;

            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "status": "created",
                    "owner": owner,
                    "repo": name,
                    "path": path.display().to_string(),
                    "seeded": template_dir.exists()
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
        ),
    }
}
