use axum::{
    extract::{Path, Query, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod auth;
mod config;
mod db;
mod git;
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
                .unwrap_or_else(|_| "gild_vcs=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    tracing::info!("Deka Git starting on port {}", config.port);

    if let Err(e) = db::init_with_url(&config.database_url).await {
        tracing::error!("Failed to initialize database: {}", e);
        std::process::exit(1);
    }

    if let Err(e) =
        db::ensure_bootstrap_identity(&config.bootstrap_username, &config.bootstrap_token).await
    {
        tracing::error!("Failed to ensure bootstrap identity: {}", e);
        std::process::exit(1);
    }

    repo::storage::init_repos_root(&config.repos_path);
    std::fs::create_dir_all(&config.repos_path).expect("Failed to create repos directory");

    tracing::info!("Repositories stored in: {}", config.repos_path);
    tracing::info!(
        "Auth enabled with bootstrap user '{}' (token from config)",
        config.bootstrap_username
    );

    let authenticated_routes = Router::new()
        .route("/:owner/:repo/info/refs", get(handle_info_refs))
        .route("/:owner/:repo/git-receive-pack", post(handle_receive_pack))
        .route("/:owner/:repo/git-upload-pack", post(handle_upload_pack))
        .route("/api/repos/:repo", post(handle_create_repo))
        .route("/api/repos", get(handle_list_repos))
        .route("/api/repos/:owner/:repo/fork", post(handle_fork_repo))
        .route("/api/auth/me", get(handle_auth_me))
        .route("/api/packages/preflight", post(handle_preflight_package))
        .route("/api/packages/publish", post(handle_publish_package))
        .route("/api/user/ssh-keys", get(handle_list_ssh_keys))
        .route("/api/user/ssh-keys", post(handle_add_ssh_key))
        .route(
            "/api/user/ssh-keys/:fingerprint",
            axum::routing::delete(handle_delete_ssh_key),
        )
        .route("/api/repos/:owner/:repo/issues", get(handle_list_issues))
        .route("/api/repos/:owner/:repo/issues", post(handle_create_issue))
        .route(
            "/api/repos/:owner/:repo/issues/:number",
            get(handle_get_issue),
        )
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
        .route("/api/repos/:owner/:repo/pulls", get(handle_list_pulls))
        .route("/api/repos/:owner/:repo/pulls", post(handle_create_pull))
        .route(
            "/api/repos/:owner/:repo/pulls/:number",
            get(handle_get_pull),
        )
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
        .layer(axum::middleware::from_fn(auth::require_auth));

    let app = Router::new()
        .merge(authenticated_routes)
        .route(
            "/api/public/repos/:owner/:repo/resolve",
            get(handle_resolve_repo_ref),
        )
        .route("/api/auth/signup", post(handle_auth_signup))
        .route("/api/auth/login", post(handle_auth_login))
        .route("/api/packages/:name", get(handle_get_package))
        .route("/api/packages/:name/:version", get(handle_get_release))
        .route(
            "/api/packages/:name/:version/docs",
            get(handle_get_release_docs),
        )
        .route(
            "/api/packages/:name/:version/tree",
            get(handle_get_release_tree),
        )
        .route(
            "/api/packages/:name/:version/blob",
            get(handle_get_release_blob),
        )
        .route(
            "/api/scoped-packages/:scope/:name",
            get(handle_get_package_scoped),
        )
        .route(
            "/api/scoped-packages/:scope/:name/:version",
            get(handle_get_release_scoped),
        )
        .route(
            "/api/scoped-packages/:scope/:name/:version/docs",
            get(handle_get_release_docs_scoped),
        )
        .route(
            "/api/scoped-packages/:scope/:name/:version/tree",
            get(handle_get_release_tree_scoped),
        )
        .route(
            "/api/scoped-packages/:scope/:name/:version/blob",
            get(handle_get_release_blob_scoped),
        )
        .route(
            "/api/packages/:name/:version/download",
            get(handle_download_release),
        )
        .route(
            "/api/scoped-packages/:scope/:name/:version/download",
            get(handle_download_release_scoped),
        )
        .route("/health", get(handle_health));

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("gild-vcs listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handle_info_refs(
    Path((owner, repo)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
    req: Request,
) -> Response {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => return (StatusCode::UNAUTHORIZED, "Authentication required").into_response(),
    };

    if owner != auth_user.username {
        return (
            StatusCode::FORBIDDEN,
            "Cannot access another user repository",
        )
            .into_response();
    }

    let service = match params.get("service") {
        Some(s) => s.as_str(),
        None => return (StatusCode::BAD_REQUEST, "Missing service parameter").into_response(),
    };

    match service {
        "git-receive-pack" | "git-upload-pack" => {
            match git::protocol::advertise_refs(&owner, &repo, service).await {
                Ok(response) => response,
                Err(e) => {
                    tracing::error!("Failed to advertise refs: {}", e);
                    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
                }
            }
        }
        _ => (StatusCode::BAD_REQUEST, "Invalid service").into_response(),
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

    if owner != auth_user.username {
        return (
            StatusCode::FORBIDDEN,
            "Cannot push to another user repository",
        )
            .into_response();
    }

    let body = match axum::body::to_bytes(req.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to read request body: {}", e);
            return (StatusCode::BAD_REQUEST, "Failed to read body").into_response();
        }
    };

    match git::receive_pack::handle(&owner, &repo, body).await {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("receive-pack failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

async fn handle_upload_pack(Path((owner, repo)): Path<(String, String)>, req: Request) -> Response {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => return (StatusCode::UNAUTHORIZED, "Authentication required").into_response(),
    };

    if owner != auth_user.username {
        return (
            StatusCode::FORBIDDEN,
            "Cannot fetch another user repository",
        )
            .into_response();
    }

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

async fn handle_create_repo(Path(repo): Path<String>, req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    match repo::storage::create_bare_repo(&auth_user.username, &repo) {
        Ok(path) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "status": "created",
                "owner": auth_user.username,
                "repo": repo,
                "path": path.display().to_string()
            })),
        ),
        Err(e) => {
            tracing::error!("create repo failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
            )
        }
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

    match repo::storage::list_repos(&auth_user.username) {
        Ok(repos) => (
            StatusCode::OK,
            Json(serde_json::json!({ "owner": auth_user.username, "repos": repos })),
        ),
        Err(e) => {
            tracing::error!("list repos failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct ResolveRefQuery {
    #[serde(rename = "ref")]
    reference: Option<String>,
}

async fn handle_resolve_repo_ref(
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<ResolveRefQuery>,
) -> impl IntoResponse {
    let requested = query.reference.unwrap_or_else(|| "HEAD".to_string());
    match repo::storage::resolve_ref(&owner, &repo, &requested) {
        Ok(resolved) => {
            let short = resolved.commit.chars().take(12).collect::<String>();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "owner": owner,
                    "repo": repo,
                    "requestedRef": resolved.requested_ref,
                    "normalizedRef": resolved.normalized_ref,
                    "commit": resolved.commit,
                    "shortCommit": short
                })),
            )
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        ),
    }
}

#[derive(Debug, Deserialize, Default)]
struct ForkRepoRequest {
    target_owner: Option<String>,
    target_repo: Option<String>,
}

async fn handle_fork_repo(
    Path((source_owner, source_repo)): Path<(String, String)>,
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
    let fork_req: ForkRepoRequest = if body.is_empty() {
        ForkRepoRequest::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
            }
        }
    };

    let target_owner = fork_req
        .target_owner
        .unwrap_or_else(|| auth_user.username.clone());
    if target_owner != auth_user.username {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "fork target owner must match authenticated user" })),
        );
    }
    let target_repo = fork_req
        .target_repo
        .unwrap_or_else(|| format!("{}-fork", source_repo));

    let forked_path = match repo::storage::fork_bare_repo(
        &source_owner,
        &source_repo,
        &target_owner,
        &target_repo,
    ) {
        Ok(path) => path,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
            )
        }
    };

    let resolved = repo::storage::resolve_ref(&target_owner, &target_repo, "HEAD");
    let (commit, short_commit) = match resolved {
        Ok(value) => {
            let short = value.commit.chars().take(12).collect::<String>();
            (value.commit, short)
        }
        Err(_) => (String::new(), String::new()),
    };

    tracing::info!(
        event = "repo.fork",
        actor = %auth_user.username,
        source_owner = %source_owner,
        source_repo = %source_repo,
        target_owner = %target_owner,
        target_repo = %target_repo,
        "repository forked"
    );

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "ok": true,
            "sourceOwner": source_owner,
            "sourceRepo": source_repo,
            "targetOwner": target_owner,
            "targetRepo": target_repo,
            "path": forked_path.display().to_string(),
            "commit": commit,
            "shortCommit": short_commit
        })),
    )
}

#[derive(Debug, Deserialize)]
struct SignupRequest {
    username: String,
    email: String,
    password: String,
}

async fn handle_auth_signup(Json(payload): Json<SignupRequest>) -> impl IntoResponse {
    let cfg = match Config::load() {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("config load failed: {}", e) })),
            )
        }
    };

    match auth::signup(
        &payload.username,
        &payload.email,
        &payload.password,
        cfg.auto_verify_signups,
    )
    .await
    {
        Ok(result) => (StatusCode::CREATED, Json(serde_json::json!(result))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username_or_email: String,
    password: String,
}

async fn handle_auth_login(Json(payload): Json<LoginRequest>) -> impl IntoResponse {
    match auth::login(&payload.username_or_email, &payload.password).await {
        Ok(result) => (StatusCode::OK, Json(serde_json::json!(result))),
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn handle_auth_me(req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(v) => v,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    match auth::auth_me(auth_user.user_id).await {
        Ok(Some(profile)) => (StatusCode::OK, Json(serde_json::json!({ "user": profile }))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "User not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct AddSshKeyRequest {
    name: String,
    public_key: String,
}

async fn handle_list_ssh_keys(req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    match auth::list_ssh_keys(auth_user.user_id).await {
        Ok(keys) => (StatusCode::OK, Json(serde_json::json!({ "keys": keys }))),
        Err(e) => {
            tracing::error!("list ssh keys failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn handle_add_ssh_key(req: Request) -> impl IntoResponse {
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

    let payload: AddSshKeyRequest = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    match auth::add_ssh_key(auth_user.user_id, &payload.name, &payload.public_key).await {
        Ok(key) => (StatusCode::CREATED, Json(serde_json::json!({ "key": key }))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn handle_delete_ssh_key(Path(fingerprint): Path<String>, req: Request) -> impl IntoResponse {
    let auth_user = match auth::get_auth_user(&req) {
        Some(user) => user,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Authentication required" })),
            )
        }
    };

    match auth::delete_ssh_key(auth_user.user_id, &fingerprint).await {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "deleted" })),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "SSH key not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
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

    let body = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid body" })),
            )
        }
    };

    let payload: packages::PublishPackageRequest = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    match packages::publish(&auth_user.username, payload).await {
        Ok(release) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "release": release })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn handle_preflight_package(req: Request) -> impl IntoResponse {
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

    let payload: packages::PublishPackageRequest = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    match packages::preflight_publish(&auth_user.username, &payload).await {
        Ok(report) => (
            StatusCode::OK,
            Json(serde_json::json!({ "preflight": report })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn handle_get_package(Path(name): Path<String>) -> impl IntoResponse {
    match packages::get_package(&name).await {
        Ok(summary) => (StatusCode::OK, Json(serde_json::json!(summary))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn handle_get_package_scoped(
    Path((scope, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let full_name = format!("@{}/{}", scope.trim_start_matches('@'), name);
    handle_get_package(Path(full_name)).await
}

async fn handle_get_release(Path((name, version)): Path<(String, String)>) -> impl IntoResponse {
    match packages::get_release(&name, &version).await {
        Ok(Some(release)) => (StatusCode::OK, Json(serde_json::json!(release))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Release not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseDocsQuery {
    symbol: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReleaseBlobQuery {
    path: String,
}

async fn handle_get_release_docs(
    Path((name, version)): Path<(String, String)>,
    Query(query): Query<ReleaseDocsQuery>,
) -> impl IntoResponse {
    if let Some(symbol) = query.symbol.as_deref() {
        return match packages::get_release_doc_symbol(&name, &version, symbol).await {
            Ok(Some(doc)) => (StatusCode::OK, Json(serde_json::json!(doc))).into_response(),
            Ok(None) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Doc symbol not found" })),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        };
    }

    match packages::get_release_docs(&name, &version).await {
        Ok(Some(docs)) => (StatusCode::OK, Json(serde_json::json!(docs))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Release docs not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_get_release_scoped(
    Path((scope, name, version)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let full_name = format!("@{}/{}", scope.trim_start_matches('@'), name);
    handle_get_release(Path((full_name, version))).await
}

async fn handle_get_release_docs_scoped(
    Path((scope, name, version)): Path<(String, String, String)>,
    Query(query): Query<ReleaseDocsQuery>,
) -> impl IntoResponse {
    let full_name = format!("@{}/{}", scope.trim_start_matches('@'), name);
    handle_get_release_docs(Path((full_name, version)), Query(query)).await
}

async fn handle_get_release_tree(
    Path((name, version)): Path<(String, String)>,
) -> impl IntoResponse {
    match packages::get_release_tree(&name, &version).await {
        Ok(Some(tree)) => (StatusCode::OK, Json(serde_json::json!(tree))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Release tree not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_get_release_tree_scoped(
    Path((scope, name, version)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let full_name = format!("@{}/{}", scope.trim_start_matches('@'), name);
    handle_get_release_tree(Path((full_name, version))).await
}

async fn handle_get_release_blob(
    Path((name, version)): Path<(String, String)>,
    Query(query): Query<ReleaseBlobQuery>,
) -> impl IntoResponse {
    match packages::get_release_blob(&name, &version, &query.path).await {
        Ok(Some(blob)) => (StatusCode::OK, Json(serde_json::json!(blob))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Blob not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_get_release_blob_scoped(
    Path((scope, name, version)): Path<(String, String, String)>,
    Query(query): Query<ReleaseBlobQuery>,
) -> impl IntoResponse {
    let full_name = format!("@{}/{}", scope.trim_start_matches('@'), name);
    handle_get_release_blob(Path((full_name, version)), Query(query)).await
}

async fn handle_download_release(Path((name, version)): Path<(String, String)>) -> Response {
    let release = match packages::get_release(&name, &version).await {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "Release not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to load release {}@{}: {}", name, version, e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load release").into_response();
        }
    };

    match packages::build_release_tarball(&release) {
        Ok(bytes) => {
            let filename = format!("{}-{}.tar.gz", name, version);
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/gzip")
                .header(
                    "content-disposition",
                    format!("attachment; filename=\"{}\"", filename),
                )
                .body(axum::body::Body::from(bytes))
                .unwrap()
        }
        Err(e) => {
            tracing::error!("Failed to build tarball for {}@{}: {}", name, version, e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to build tarball").into_response()
        }
    }
}

async fn handle_download_release_scoped(
    Path((scope, name, version)): Path<(String, String, String)>,
) -> Response {
    let full_name = format!("@{}/{}", scope.trim_start_matches('@'), name);
    handle_download_release(Path((full_name, version))).await
}

async fn handle_health() -> &'static str {
    "OK"
}

async fn handle_list_issues(
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<issues::ListIssuesQuery>,
) -> impl IntoResponse {
    match issues::list_issues(&owner, &repo, &query).await {
        Ok(list) => (StatusCode::OK, Json(serde_json::json!({ "issues": list }))),
        Err(e) => {
            tracing::error!("list issues failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
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

    match issues::create_issue(&owner, &repo, &auth_user.username, create_req).await {
        Ok(issue) => (StatusCode::CREATED, Json(serde_json::json!(issue))),
        Err(e) => {
            tracing::error!("create issue failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn handle_get_issue(
    Path((owner, repo, number)): Path<(String, String, i32)>,
) -> impl IntoResponse {
    match issues::get_issue(&owner, &repo, number).await {
        Ok(Some(issue)) => {
            let comments = issues::list_comments(&owner, &repo, number)
                .await
                .unwrap_or_default();
            let labels = issues::get_issue_labels(&owner, &repo, number)
                .await
                .unwrap_or_default();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "issue": issue,
                    "comments": comments,
                    "labels": labels
                })),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Issue not found" })),
        ),
        Err(e) => {
            tracing::error!("get issue failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn handle_update_issue(
    Path((owner, repo, number)): Path<(String, String, i32)>,
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
        Ok(Some(issue)) => (StatusCode::OK, Json(serde_json::json!(issue))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Issue not found" })),
        ),
        Err(e) => {
            tracing::error!("update issue failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn handle_list_comments(
    Path((owner, repo, number)): Path<(String, String, i32)>,
) -> impl IntoResponse {
    match issues::list_comments(&owner, &repo, number).await {
        Ok(comments) => (
            StatusCode::OK,
            Json(serde_json::json!({ "comments": comments })),
        ),
        Err(e) => {
            tracing::error!("list comments failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn handle_create_comment(
    Path((owner, repo, number)): Path<(String, String, i32)>,
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

    match issues::add_comment(&owner, &repo, number, &auth_user.username, comment_req).await {
        Ok(Some(comment)) => (StatusCode::CREATED, Json(serde_json::json!(comment))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Issue not found" })),
        ),
        Err(e) => {
            tracing::error!("create comment failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn handle_list_labels(Path((owner, repo)): Path<(String, String)>) -> impl IntoResponse {
    match issues::list_labels(&owner, &repo).await {
        Ok(labels) => (
            StatusCode::OK,
            Json(serde_json::json!({ "labels": labels })),
        ),
        Err(e) => {
            tracing::error!("list labels failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
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
        Err(e) => {
            tracing::error!("create label failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn handle_add_label(
    Path((owner, repo, number, label)): Path<(String, String, i32, String)>,
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
        Err(e) => {
            tracing::error!("add label failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

async fn handle_remove_label(
    Path((owner, repo, number, label)): Path<(String, String, i32, String)>,
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
        Err(e) => {
            tracing::error!("remove label failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    }
}

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

    match pulls::create_pull(&owner, &repo, &auth_user.username, create_req).await {
        Ok(pr) => (StatusCode::CREATED, Json(serde_json::json!(pr))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn handle_get_pull(
    Path((owner, repo, number)): Path<(String, String, i32)>,
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
    Path((owner, repo, number)): Path<(String, String, i32)>,
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
        Ok(Some(pr)) => (StatusCode::OK, Json(serde_json::json!(pr))),
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
    Path((owner, repo, number)): Path<(String, String, i32)>,
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
    Path((owner, repo, number)): Path<(String, String, i32)>,
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
    match pulls::add_pull_comment(&owner, &repo, number, &auth_user.username, create_req).await {
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
