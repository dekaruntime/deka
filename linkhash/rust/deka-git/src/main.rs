use axum::{
    extract::{ConnectInfo, Path, Query, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod auth;
mod config;
mod db;
mod git;
mod hooks;
mod issues;
mod packages;
mod pulls;
mod repo;

use config::Config;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "deka_git=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::load();

    tracing::info!("Linkhash (deka-git) starting on port {}", config.port);

    // Ensure data directory exists
    std::fs::create_dir_all(config.db_dir()).expect("Failed to create database directory");
    std::fs::create_dir_all(&config.repos_dir).expect("Failed to create repos directory");

    if let Err(e) = db::init(&config.db_path).await {
        tracing::error!("Failed to initialize database: {}", e);
        std::process::exit(1);
    }

    repo::storage::init_repos_root(&config.repos_dir);

    // Seed labels for the 5 project repos
    let project_repos = ["tana", "deka", "tana-website", "tana-admin", "tana-store-admin"];
    for repo_name in &project_repos {
        auth::seed_labels("tana", repo_name).await;
    }

    tracing::info!("Repositories stored in: {}", config.repos_dir);

    // Token management (no auth required for initial token creation)
    let public_routes = Router::new()
        .route("/health", get(handle_health))
        .route("/api/tokens", post(handle_create_token));

    // Authenticated routes
    let authenticated_routes = Router::new()
        // Git protocol (write only — reads are handled via optional_auth routes)
        .route("/:owner/:repo/git-receive-pack", post(handle_receive_pack))
        // Repo management
        .route("/api/repos/:repo", post(handle_create_repo))
        .route("/api/repos", get(handle_list_repos))
        .route("/api/repos/:owner/:name/init", post(handle_init_store_repo))
        // Issues
        .route("/api/repos/:owner/:repo/issues", get(handle_list_issues))
        .route("/api/repos/:owner/:repo/issues", post(handle_create_issue))
        .route("/api/repos/:owner/:repo/issues/:number", get(handle_get_issue))
        .route(
            "/api/repos/:owner/:repo/issues/:number",
            axum::routing::patch(handle_update_issue),
        )
        .route(
            "/api/repos/:owner/:repo/issues/:number/comments",
            get(handle_list_comments),
        )
        .route(
            "/api/repos/:owner/:repo/issues/:number/comments",
            post(handle_create_comment),
        )
        .route(
            "/api/repos/:owner/:repo/issues/:number/commit-refs",
            post(handle_add_commit_ref),
        )
        .route(
            "/api/repos/:owner/:repo/issues/:number/commit-refs",
            get(handle_get_commit_refs),
        )
        // Labels
        .route("/api/repos/:owner/:repo/labels", get(handle_list_labels))
        .route("/api/repos/:owner/:repo/labels", post(handle_create_label))
        .route(
            "/api/repos/:owner/:repo/issues/:number/labels/:label",
            axum::routing::put(handle_add_label),
        )
        .route(
            "/api/repos/:owner/:repo/issues/:number/labels/:label",
            axum::routing::delete(handle_remove_label),
        )
        // Pull requests
        .route("/api/repos/:owner/:repo/pulls", get(handle_list_pulls))
        .route("/api/repos/:owner/:repo/pulls", post(handle_create_pull))
        .route("/api/repos/:owner/:repo/pulls/:number", get(handle_get_pull))
        .route(
            "/api/repos/:owner/:repo/pulls/:number",
            axum::routing::patch(handle_update_pull),
        )
        .route(
            "/api/repos/:owner/:repo/pulls/:number/comments",
            get(handle_list_pull_comments),
        )
        .route(
            "/api/repos/:owner/:repo/pulls/:number/comments",
            post(handle_create_pull_comment),
        )
        // Token management
        .route("/api/tokens", get(handle_list_tokens))
        .route("/api/tokens/:id", axum::routing::delete(handle_revoke_token))
        // Audit log
        .route("/api/audit", get(handle_audit_log))
        // Convenience: flat issue list across all repos
        .route("/api/issues", get(handle_list_all_issues))
        // Package registry
        .route("/api/packages", get(handle_list_packages))
        .route("/api/packages/preflight", post(handle_preflight_publish))
        .route("/api/packages/publish", post(handle_publish_package))
        .route("/api/packages/:name/versions", get(handle_list_versions))
        .route("/api/packages/:name/:version", get(handle_get_release))
        .route("/api/packages/:name/latest", get(handle_get_latest))
        .route("/api/packages/:name/:version/docs", get(handle_get_docs))
        .route("/api/packages/:name/:version/tree", get(handle_get_tree))
        .route("/api/packages/:name/:version/blob", get(handle_get_blob))
        // Scoped packages (@scope/name)
        .route("/api/scoped-packages/:scope/:name/versions", get(handle_list_scoped_versions))
        .route("/api/scoped-packages/:scope/:name/:version", get(handle_get_scoped_release))
        .route("/api/scoped-packages/:scope/:name/latest", get(handle_get_scoped_latest))
        .route("/api/scoped-packages/:scope/:name/:version/docs", get(handle_get_scoped_docs))
        .route("/api/scoped-packages/:scope/:name/:version/tree", get(handle_get_scoped_tree))
        .route("/api/scoped-packages/:scope/:name/:version/blob", get(handle_get_scoped_blob))
        .layer(axum::middleware::from_fn(auth::require_auth));

    // Routes that use optional auth (public repos can be read without a token)
    let optional_auth_routes = Router::new()
        .route("/:owner/:repo/info/refs", get(handle_info_refs_public))
        .route("/:owner/:repo/git-upload-pack", post(handle_upload_pack_public))
        .route("/api/repos/:owner/:name/visibility", get(handle_get_visibility))
        .layer(axum::middleware::from_fn(auth::optional_auth));

    // Visibility management (requires auth)
    let visibility_routes = Router::new()
        .route(
            "/api/repos/:owner/:name/visibility",
            axum::routing::put(handle_set_visibility),
        )
        .layer(axum::middleware::from_fn(auth::require_auth));

    let app = Router::new()
        .merge(public_routes)
        .merge(optional_auth_routes)
        .merge(visibility_routes)
        .merge(authenticated_routes);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("Linkhash listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

// --- Health ---

async fn handle_health() -> &'static str {
    "OK"
}

// --- Repo visibility ---

async fn handle_get_visibility(
    Path((owner, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let visibility = auth::get_repo_visibility(&owner, &name).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "owner": owner,
            "repo": name,
            "visibility": visibility
        })),
    )
}

#[derive(Debug, Deserialize)]
struct SetVisibilityRequest {
    visibility: String,
}

async fn handle_set_visibility(
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

    // Requires: system token, repo owner, wildcard scope, or repo:write on the repo
    let is_system = auth_user.key_type == "system";
    let is_owner = auth_user.owner == owner;
    let has_wildcard = auth_user.has_scope("*");
    let has_write = auth_user.has_scope("repo:write")
        && auth_user.can_access_repo(&format!("{}/{}", owner, name));
    if !is_system && !is_owner && !has_wildcard && !has_write {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Insufficient permissions to change visibility" })),
        );
    }

    let body = match axum::body::to_bytes(req.into_body(), 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid body" })),
            )
        }
    };

    let vis_req: SetVisibilityRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    if vis_req.visibility != "public" && vis_req.visibility != "private" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "visibility must be 'public' or 'private'" })),
        );
    }

    match auth::set_repo_visibility(&owner, &name, &vis_req.visibility).await {
        Ok(()) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "repo.visibility",
                Some(&format!("{}/{}", owner, name)),
                None,
                Some(&format!("set to {}", vis_req.visibility)),
                None,
            )
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "owner": owner,
                    "repo": name,
                    "visibility": vis_req.visibility
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

// --- Git protocol handlers (public-aware) ---

/// info/refs handler that allows public repo reads without auth.
async fn handle_info_refs_public(
    Path((owner, repo)): Path<(String, String)>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    req: Request,
) -> Response {
    let repo_name = repo.strip_suffix(".git").unwrap_or(&repo);
    let service = match params.get("service") {
        Some(s) => s.as_str(),
        None => return (StatusCode::BAD_REQUEST, "Missing service parameter").into_response(),
    };

    let is_read = service == "git-upload-pack";
    let is_public = auth::is_repo_public(&owner, repo_name).await;

    // Write operations always require auth
    if !is_read {
        let auth_user = match auth::get_auth_user(&req) {
            Some(user) => user,
            None => return (StatusCode::UNAUTHORIZED, "Authentication required for push").into_response(),
        };
        if !auth_user.has_scope("repo:write") {
            return (StatusCode::FORBIDDEN, "repo:write scope required").into_response();
        }
        if !auth_user.can_access_repo(repo_name) && !auth_user.can_access_repo(&repo) {
            return (StatusCode::FORBIDDEN, "Access denied to this repository").into_response();
        }
        auth::log_audit(
            Some(auth_user.token_id), &auth_user.key_type, &auth_user.owner,
            "push.info_refs", Some(&format!("{}/{}", owner, repo)), None, None, None,
        ).await;
    } else if !is_public {
        // Private repo read requires auth
        let auth_user = match auth::get_auth_user(&req) {
            Some(user) => user,
            None => return (StatusCode::UNAUTHORIZED, "Authentication required").into_response(),
        };
        if !auth_user.has_scope("repo:read") {
            return (StatusCode::FORBIDDEN, "repo:read scope required").into_response();
        }
        if !auth_user.can_access_repo(repo_name) && !auth_user.can_access_repo(&repo) {
            return (StatusCode::FORBIDDEN, "Access denied to this repository").into_response();
        }
        auth::log_audit(
            Some(auth_user.token_id), &auth_user.key_type, &auth_user.owner,
            "fetch.info_refs", Some(&format!("{}/{}", owner, repo)), None, None, None,
        ).await;
    }
    // Public repo read: no auth needed, proceed

    match git::protocol::advertise_refs(&owner, &repo, service).await {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("Failed to advertise refs: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// upload-pack handler that allows public repo reads without auth.
async fn handle_upload_pack_public(
    Path((owner, repo)): Path<(String, String)>,
    req: Request,
) -> Response {
    let repo_name = repo.strip_suffix(".git").unwrap_or(&repo);
    let is_public = auth::is_repo_public(&owner, repo_name).await;

    if !is_public {
        let auth_user = match auth::get_auth_user(&req) {
            Some(user) => user.clone(),
            None => return (StatusCode::UNAUTHORIZED, "Authentication required").into_response(),
        };
        if !auth_user.has_scope("repo:read") {
            return (StatusCode::FORBIDDEN, "repo:read scope required").into_response();
        }
        if !auth_user.can_access_repo(repo_name) && !auth_user.can_access_repo(&repo) {
            return (StatusCode::FORBIDDEN, "Access denied to this repository").into_response();
        }
        let body = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!("Failed to read request body: {}", e);
                return (StatusCode::BAD_REQUEST, "Failed to read body").into_response();
            }
        };
        auth::log_audit(
            Some(auth_user.token_id), &auth_user.key_type, &auth_user.owner,
            "clone", Some(&format!("{}/{}", owner, repo)), None, None, None,
        ).await;
        match git::upload_pack::handle(&owner, &repo, body).await {
            Ok(response) => response,
            Err(e) => {
                tracing::error!("upload-pack failed: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    } else {
        // Public repo: no auth required for reads
        let body = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!("Failed to read request body: {}", e);
                return (StatusCode::BAD_REQUEST, "Failed to read body").into_response();
            }
        };
        match git::upload_pack::handle(&owner, &repo, body).await {
            Ok(response) => response,
            Err(e) => {
                tracing::error!("upload-pack failed: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    }
}

async fn handle_receive_pack(
    Path((owner, repo)): Path<(String, String)>,
    req: Request,
) -> Response {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user.clone(),
        None => return (StatusCode::UNAUTHORIZED, "Authentication required").into_response(),
    };

    if !auth_user.has_scope("repo:write") {
        return (StatusCode::FORBIDDEN, "repo:write scope required").into_response();
    }

    let repo_name = repo.strip_suffix(".git").unwrap_or(&repo);
    if !auth_user.can_access_repo(repo_name) && !auth_user.can_access_repo(&repo) {
        return (StatusCode::FORBIDDEN, "Access denied to this repository").into_response();
    }

    let body = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to read request body: {}", e);
            return (StatusCode::BAD_REQUEST, "Failed to read body").into_response();
        }
    };

    auth::log_audit(
        Some(auth_user.token_id),
        &auth_user.key_type,
        &auth_user.owner,
        "push",
        Some(&format!("{}/{}", owner, repo)),
        None,
        None,
        None,
    )
    .await;

    // Keep a copy of the body for post-receive hook parsing
    let body_bytes = body.to_vec();

    match git::receive_pack::handle(&owner, &repo, body).await {
        Ok(response) => {
            // Post-receive hook: trigger rebuild if main was pushed
            let hook_owner = owner.clone();
            let hook_repo = repo.clone();
            let hook_token_id = auth_user.token_id;
            let hook_key_type = auth_user.key_type.clone();
            let hook_auth_owner = auth_user.owner.clone();
            tokio::spawn(async move {
                hooks::run_post_receive(
                    &hook_owner,
                    &hook_repo,
                    &body_bytes,
                    Some(hook_token_id),
                    &hook_key_type,
                    &hook_auth_owner,
                )
                .await;
            });
            response
        }
        Err(e) => {
            tracing::error!("receive-pack failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

// --- Repo management ---

async fn handle_create_repo(Path(repo): Path<String>, req: Request) -> impl IntoResponse {
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

async fn handle_list_repos(req: Request) -> impl IntoResponse {
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

async fn handle_init_store_repo(
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

// --- Token management ---

#[derive(Debug, Deserialize)]
struct CreateTokenPayload {
    key_type: String,
    owner: String,
    scopes: Option<Vec<String>>,
    repos: Option<Vec<String>>,
    expires_in_days: Option<i64>,
}

async fn handle_create_token(Json(payload): Json<CreateTokenPayload>) -> impl IntoResponse {
    let req = auth::CreateTokenRequest {
        key_type: payload.key_type,
        owner: payload.owner,
        scopes: payload.scopes,
        repos: payload.repos,
        expires_in_days: payload.expires_in_days,
    };

    match auth::create_token(req).await {
        Ok(result) => {
            auth::log_audit(
                Some(result.id),
                &result.key_type,
                &result.owner,
                "token.create",
                None,
                None,
                Some(&format!("scopes: {:?}", result.scopes)),
                None,
            )
            .await;
            (StatusCode::CREATED, Json(serde_json::json!(result)))
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn handle_list_tokens(req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    match auth::list_tokens().await {
        Ok(tokens) => (StatusCode::OK, Json(serde_json::json!({ "tokens": tokens }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn handle_revoke_token(Path(id): Path<i64>, req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user.clone(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    match auth::revoke_token(id).await {
        Ok(true) => {
            auth::log_audit(
                Some(auth_user.token_id),
                &auth_user.key_type,
                &auth_user.owner,
                "token.revoke",
                None,
                None,
                Some(&format!("revoked token_id={}", id)),
                None,
            )
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "revoked" })),
            )
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Token not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

// --- Audit log ---

async fn handle_audit_log(
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

// --- Issues ---

async fn handle_list_issues(
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

async fn handle_list_all_issues(
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
    let project_repos = ["tana", "deka", "tana-website", "tana-admin", "tana-store-admin"];
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

async fn handle_create_issue(
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

async fn handle_get_issue(
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
            let commit_refs = issues::get_commit_refs(issue.id)
                .await
                .unwrap_or_default();
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

async fn handle_update_issue(
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

async fn handle_list_comments(
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

async fn handle_create_comment(
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

async fn handle_add_commit_ref(
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

async fn handle_get_commit_refs(
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

// --- Labels ---

async fn handle_list_labels(Path((owner, repo)): Path<(String, String)>) -> impl IntoResponse {
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

async fn handle_create_label(
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
    match issues::create_label(&owner, &repo, &label_req.name, color, label_req.description.as_deref()).await {
        Ok(label) => (StatusCode::CREATED, Json(serde_json::json!(label))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn handle_add_label(
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

async fn handle_remove_label(
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

// --- Pull requests ---

async fn handle_list_pulls(
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

async fn handle_create_pull(
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

async fn handle_get_pull(
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

async fn handle_update_pull(
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

async fn handle_list_pull_comments(
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

async fn handle_create_pull_comment(
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

#[derive(Debug, Deserialize)]
struct BlobQuery {
    path: Option<String>,
}

async fn handle_list_packages(req: Request) -> impl IntoResponse {
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

async fn handle_preflight_publish(req: Request) -> impl IntoResponse {
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

async fn handle_publish_package(req: Request) -> impl IntoResponse {
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

async fn handle_list_versions(
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

async fn handle_get_release(
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

async fn handle_get_latest(
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

async fn handle_get_docs(
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

async fn handle_get_tree(
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

async fn handle_get_blob(
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

// --- Scoped package handlers (delegates to unscoped with @scope/name) ---

async fn handle_list_scoped_versions(
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

async fn handle_get_scoped_release(
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

async fn handle_get_scoped_latest(
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

async fn handle_get_scoped_docs(
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

async fn handle_get_scoped_tree(
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

async fn handle_get_scoped_blob(
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
