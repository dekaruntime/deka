//! One-shot App() execution for `deka build` static HTML.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::env::init_env;
use crate::extensions::extensions_for_mode;
use engine::{config as runtime_config, RuntimeEngine};
use pool::{ExecutionMode, HandlerKey, PoolConfig, RequestData};
use runtime_core::env::set_handler_path_with;
use runtime_core::framework;
use runtime_core::modules::ensure_deka_module_root_env_with;
use runtime_core::storefront_envelope::StorefrontResponse;

/// One static render the build must produce: the page's route template, the
/// CONCRETE route to write HTML for, and the literal params for the template's
/// `[param]` segments (empty for plain static pages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticRenderTask {
    pub template: String,
    pub route: String,
    pub params: BTreeMap<String, String>,
}

pub fn prerender_static_pages(
    project_root: &Path,
    dist_client: &Path,
    tasks: &[StaticRenderTask],
) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("prerender runtime: {err}"))?;
    rt.block_on(prerender_static_pages_async(project_root, dist_client, tasks))
}

async fn prerender_static_pages_async(
    project_root: &Path,
    dist_client: &Path,
    tasks: &[StaticRenderTask],
) -> Result<(), String> {
    init_env();
    unsafe {
        std::env::set_var("DEKA_SECURITY_NO_PROMPT", "1");
    }
    crate::islands::write_island_client_assets_for_project(project_root)?;
    crate::css::write_route_css_assets_for_project(project_root)?;

    // Explicit render plan: every static route and staticParams instance the
    // build manifest planned. Empty means no app-router pages were planned;
    // render `/` for minimal projects as before.
    let mut planned: Vec<StaticRenderTask> = tasks.to_vec();
    if planned.is_empty() {
        planned.push(StaticRenderTask {
            template: "/".to_string(),
            route: "/".to_string(),
            params: BTreeMap::new(),
        });
    }

    let mut pool_config = PoolConfig::default();
    pool_config.num_workers = 1;
    pool_config.request_timeout_ms = 30_000;
    let runtime_cfg = runtime_config::RuntimeConfig::load();
    let serve_mode = runtime_config::ServeMode::Php;
    let extensions_provider = Arc::new(move || extensions_for_mode(&serve_mode));
    let engine = Arc::new(RuntimeEngine::new(
        pool_config.clone(),
        pool_config,
        &runtime_cfg,
        extensions_provider,
    ));

    for task in &planned {
        let route = &task.route;
        let entry =
            framework::write_static_render_entry(project_root, &task.template, &task.params)?;
        let handler_path = entry.to_string_lossy().to_string();
        let mut env_set = |key: &str, value: &str| unsafe { std::env::set_var(key, value) };
        let env_get = |key: &str| std::env::var(key).ok();
        set_handler_path_with(&handler_path, &env_get, &mut env_set);
        ensure_deka_module_root_env_with(
            &handler_path,
            &|path| path.exists(),
            &|| std::env::current_exe().ok(),
            &env_get,
            &mut env_set,
        );
        let response = engine
            .execute(
                HandlerKey::new(format!("prerender:{route}")),
                RequestData {
                    handler_code: String::new(),
                    handler_entry: Some(handler_path.clone()),
                    module_root: Some(project_root.to_string_lossy().into_owned()),
                    request_value: serde_json::Value::Null,
                    request_parts: None,
                    mode: ExecutionMode::StaticRender,
                    security: None,
                },
            )
            .await
            .map_err(|err| format!("prerender {route}: {err}"))?;
        if !response.success {
            return Err(format!(
                "prerender {route}: {}",
                response.error.unwrap_or_else(|| "unknown error".to_string())
            ));
        }
        let result = response
            .result
            .ok_or_else(|| format!("prerender {route}: empty result"))?;
        let envelope = StorefrontResponse::from_value(result)
            .map_err(|err| format!("prerender {route}: {err}"))?;
        if envelope.status >= 400 {
            return Err(format!(
                "prerender {route}: status {}",
                envelope.status
            ));
        }
        let dest = dist_path_for_route(dist_client, route)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        std::fs::write(&dest, envelope.body.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", dest.display()))?;
    }
    Ok(())
}

/// Map a concrete route to its published HTML path. Segment validation here
/// is defense in depth (deka#719 review, codex): dot segments would be
/// normalized by the filesystem and could write outside `dist_client`, so
/// they are rejected at the boundary even though the build manifest already
/// refuses them.
fn dist_path_for_route(dist_client: &Path, route: &str) -> Result<PathBuf, String> {
    if route == "/" {
        return Ok(dist_client.join("index.html"));
    }
    let mut path = dist_client.to_path_buf();
    for segment in route.trim_start_matches('/').split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(format!(
                "prerender {route}: unsafe path segment `{segment}`"
            ));
        }
        path.push(segment);
    }
    Ok(path.join("index.html"))
}
