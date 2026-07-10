#![allow(
    clippy::unnecessary_sort_by,
    clippy::while_immutable_condition,
    dead_code,
    unused_variables
)]

use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod account_routes;
mod audit_routes;
mod auth;
mod authz;
mod config;
mod db;
mod git;
mod git_routes;
mod hooks;
mod issue_routes;
mod issues;
mod label_routes;
mod package_routes;
mod packages;
mod pipeline_yaml;
mod pull_routes;
mod pulls;
mod redaction;
mod repo;
mod repo_routes;
mod scoped_package_routes;
mod token_routes;
mod visibility_routes;

use account_routes::*;
use audit_routes::*;
use config::Config;
use git_routes::*;
use issue_routes::*;
use label_routes::*;
use package_routes::*;
use pull_routes::*;
use repo_routes::*;
use scoped_package_routes::*;
use token_routes::*;
use visibility_routes::*;

const DEFAULT_GILD_VAULT_SOCKET_PATH: &str = "/run/gild-vault/sock";

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "linkhash=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    if let Some(command) = std::env::args().nth(1) {
        if command == "migrate-tokens-to-vault" {
            if let Err(err) = run_migrate_tokens_to_vault().await {
                tracing::error!("token vault migration failed: {}", err);
                std::process::exit(1);
            }
            return;
        }
    }

    let config = Config::load();

    tracing::info!("Linkhash (gild-vcs) starting on port {}", config.port);

    // Ensure data directory exists
    std::fs::create_dir_all(config.db_dir()).expect("Failed to create database directory");
    std::fs::create_dir_all(&config.repos_dir).expect("Failed to create repos directory");

    if let Err(e) = db::init(&config.db_path).await {
        tracing::error!("Failed to initialize database: {}", e);
        std::process::exit(1);
    }

    repo::storage::init_repos_root(&config.repos_dir);

    // Seed labels for the 5 project repos
    let project_repos = [
        "tana",
        "deka",
        "tana-website",
        "tana-admin",
        "tana-store-admin",
    ];
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
        // Token management
        .route("/api/auth/me", get(handle_auth_me))
        .route("/api/tokens", get(handle_list_tokens))
        .route(
            "/api/tokens/:id",
            axum::routing::delete(handle_revoke_token),
        )
        // Account and ACL management
        .route("/api/accounts", get(handle_list_accounts))
        .route(
            "/api/accounts/agents/bootstrap",
            post(handle_bootstrap_agents),
        )
        .route(
            "/api/accounts/:account/ssh-keys",
            post(handle_register_ssh_key),
        )
        .route(
            "/api/repos/:owner/:repo/acl",
            axum::routing::put(handle_grant_repo_acl),
        )
        .route(
            "/api/repos/:owner/:repo/secrets/acl",
            axum::routing::put(handle_grant_secret_acl),
        )
        .route("/api/repos/:owner/:repo/env", get(handle_env_acl))
        .route(
            "/api/repos/:owner/:repo/webhooks",
            get(handle_list_webhooks),
        )
        .route(
            "/api/repos/:owner/:repo/webhooks",
            post(handle_create_webhook),
        )
        .route("/api/deploy-watchers", get(handle_list_deploy_watchers))
        .route("/api/deploy-watchers", post(handle_upsert_deploy_watcher))
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
        .route(
            "/api/scoped-packages/:scope/:name/versions",
            get(handle_list_scoped_versions),
        )
        .route(
            "/api/scoped-packages/:scope/:name/:version",
            get(handle_get_scoped_release),
        )
        .route(
            "/api/scoped-packages/:scope/:name/latest",
            get(handle_get_scoped_latest),
        )
        .route(
            "/api/scoped-packages/:scope/:name/:version/docs",
            get(handle_get_scoped_docs),
        )
        .route(
            "/api/scoped-packages/:scope/:name/:version/tree",
            get(handle_get_scoped_tree),
        )
        .route(
            "/api/scoped-packages/:scope/:name/:version/blob",
            get(handle_get_scoped_blob),
        )
        .layer(axum::middleware::from_fn(auth::require_auth));

    // Routes that use optional auth (public repos can be read without a token)
    let optional_auth_routes = Router::new()
        .route("/:owner/:repo/info/refs", get(handle_info_refs_public))
        .route(
            "/:owner/:repo/git-upload-pack",
            post(handle_upload_pack_public),
        )
        .route(
            "/api/repos/:owner/:name/visibility",
            get(handle_get_visibility),
        )
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
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

// --- Health ---

async fn handle_health() -> &'static str {
    "OK"
}

async fn run_migrate_tokens_to_vault() -> anyhow::Result<()> {
    let config = Config::load();
    std::fs::create_dir_all(config.db_dir())?;
    db::init(&config.db_path).await?;

    let socket_path = migrate_tokens_socket_path(std::env::args().skip(2))?;
    let summary = auth::migrate_tokens_to_vault(&socket_path).await?;
    tracing::info!(
        migrated = summary.migrated,
        skipped = summary.skipped,
        failed = summary.failed,
        socket = %socket_path.display(),
        "migrated gild-vcs tokens to vault"
    );
    println!(
        "migrated {}, skipped {}, failed {}",
        summary.migrated, summary.skipped, summary.failed
    );
    Ok(())
}

fn migrate_tokens_socket_path(mut args: impl Iterator<Item = String>) -> anyhow::Result<PathBuf> {
    let mut socket_path = std::env::var("GILD_VAULT_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_GILD_VAULT_SOCKET_PATH));

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => {
                let Some(value) = args.next() else {
                    anyhow::bail!("--socket requires a path");
                };
                socket_path = PathBuf::from(value);
            }
            other => anyhow::bail!("unknown migrate-tokens-to-vault option: {other}"),
        }
    }

    Ok(socket_path)
}
