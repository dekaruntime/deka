//! One-shot App() execution for `deka build` static HTML.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::env::init_env;
use crate::extensions::extensions_for_mode;
use engine::{config as runtime_config, RuntimeEngine};
use pool::{ExecutionMode, HandlerKey, PoolConfig, RequestData};
use runtime_core::env::set_handler_path_with;
use runtime_core::framework::{self, static_page_routes};
use runtime_core::modules::ensure_deka_module_root_env_with;
use runtime_core::storefront_envelope::StorefrontResponse;

pub fn prerender_static_pages(project_root: &Path, dist_client: &Path) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("prerender runtime: {err}"))?;
    rt.block_on(prerender_static_pages_async(project_root, dist_client))
}

async fn prerender_static_pages_async(
    project_root: &Path,
    dist_client: &Path,
) -> Result<(), String> {
    init_env();
    unsafe {
        std::env::set_var("DEKA_SECURITY_NO_PROMPT", "1");
    }
    crate::islands::write_island_client_assets_for_project(project_root)?;
    crate::css::write_route_css_assets_for_project(project_root)?;

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
    let app_dir = project_root.join("app");
    let manifest = framework::scan_app_dir(&app_dir);
    let mut routes = static_page_routes(&manifest);
    if routes.is_empty() {
        routes.push("/".to_string());
    }

    for route in routes {
        let entry = framework::write_static_render_entry(project_root, &route)?;
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
                    request_value: serde_json::Value::Null,
                    request_parts: None,
                    mode: ExecutionMode::StaticRender,
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
        let dest = dist_path_for_route(dist_client, &route);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        std::fs::write(&dest, envelope.body.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", dest.display()))?;
    }
    Ok(())
}

fn dist_path_for_route(dist_client: &Path, route: &str) -> PathBuf {
    if route == "/" {
        dist_client.join("index.html")
    } else {
        dist_client
            .join(route.trim_start_matches('/'))
            .join("index.html")
    }
}
