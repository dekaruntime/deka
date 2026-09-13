//! React Fast Refresh for `deka dev`: vendor files, per-module compile, WS push.

mod compile;
mod transform;
mod vendor;

use axum::http::header::CONTENT_TYPE;
use axum::response::Response;
use engine::RuntimeState;
use std::sync::Arc;

pub use compile::{compile_abs, compile_relative, dsc_supports_dev, install, RefreshContext};
pub use transform::{detect_components, is_refreshable_path, wrap_module, Component};

const MODULE_PREFIX: &str = "/_deka/hmr/module/";
const VENDOR_PREFIX: &str = "/_deka/react/";

pub fn try_response(state: &Arc<RuntimeState>, path: &str) -> Option<Response> {
    if !state.dev_mode {
        return None;
    }
    if let Some(rel) = path.strip_prefix(VENDOR_PREFIX) {
        return vendor_response(rel);
    }
    if let Some(rel) = path.strip_prefix(MODULE_PREFIX) {
        return module_response(rel);
    }
    None
}

pub fn js_update_payload(changed: &[String]) -> Option<String> {
    let refreshable: Vec<&String> = changed
        .iter()
        .filter(|path| is_refreshable_path(path))
        .collect();
    if refreshable.is_empty() {
        return None;
    }
    let ctx = compile::context();
    if ctx.project_root.as_os_str().is_empty() {
        return None;
    }
    let mut modules = Vec::new();
    for path in &refreshable {
        let abs = std::path::Path::new(path.as_str());
        let compiled = if abs.is_absolute() || abs.exists() {
            compile_abs(abs)
        } else {
            compile_relative(&path.replace('\\', "/")).map(|js| (path.replace('\\', "/"), js))
        };
        match compiled {
            Ok((rel, js)) => {
                if !js.contains("__dekaRefreshBoundary = true") {
                    continue;
                }
                modules.push(serde_json::json!({
                    "id": rel,
                    "url": format!("{MODULE_PREFIX}{rel}"),
                }));
            }
            Err(err) => {
                tracing::warn!("fast refresh compile failed for {path}: {err}");
                return Some(
                    serde_json::json!({
                        "type": "reload",
                        "paths": changed,
                        "reason": err,
                    })
                    .to_string(),
                );
            }
        }
    }
    if modules.is_empty() {
        return None;
    }
    Some(
        serde_json::json!({
            "type": "js-update",
            "schema": 1,
            "paths": changed,
            "modules": modules,
        })
        .to_string(),
    )
}

pub fn broadcast_changed(changed: &[String]) -> bool {
    let Some(payload) = js_update_payload(changed) else {
        return false;
    };
    tracing::info!("fast refresh {payload}");
    crate::websocket::broadcast_hmr_text(payload);
    true
}

fn vendor_response(rel: &str) -> Option<Response> {
    let file = vendor::get(rel)?;
    Response::builder()
        .status(200)
        .header(CONTENT_TYPE, vendor::content_type(rel))
        .header("cache-control", "no-store")
        .body(axum::body::Body::from(file.body))
        .ok()
}

fn module_response(rel: &str) -> Option<Response> {
    match compile_relative(rel) {
        Ok(js) => Response::builder()
            .status(200)
            .header(CONTENT_TYPE, "text/javascript; charset=utf-8")
            .header("cache-control", "no-store")
            .body(axum::body::Body::from(js))
            .ok(),
        Err(err) => {
            let status = if err.contains("no such module") || err.contains("escaped") {
                404
            } else {
                500
            };
            Response::builder()
                .status(status)
                .header(CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(axum::body::Body::from(err))
                .ok()
        }
    }
}

pub fn import_map_json() -> &'static str {
    r#"{
  "imports": {
    "react": "/_deka/react/react.js",
    "react/jsx-runtime": "/_deka/react/jsx-runtime.js",
    "react/jsx-dev-runtime": "/_deka/react/jsx-dev-runtime.js",
    "react-dom": "/_deka/react/react-dom.js",
    "react-dom/client": "/_deka/react/react-dom-client.js",
    "react-refresh/runtime": "/_deka/react/refresh-runtime.js",
    "@js/react": "/_deka/react/react.js",
    "@js/react/jsx-runtime": "/_deka/react/jsx-runtime.js",
    "@js/react/jsx-dev-runtime": "/_deka/react/jsx-dev-runtime.js"
  }
}"#
}

#[cfg(test)]
mod tests {
    use super::js_update_payload;
    use crate::react_refresh::{install, RefreshContext};
    use std::fs;
    use std::path::PathBuf;

    fn project(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/fast-refresh-it")
            .join(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("Label.js"),
            "export function Label() { return 'hello'; }\n",
        )
        .unwrap();
        install(RefreshContext {
            project_root: root.clone(),
            dsc: None,
        });
        root
    }

    #[test]
    fn js_component_edit_emits_js_update() {
        let _lock = crate::react_refresh::compile::TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = project("js-update");
        let path = root.join("Label.js").to_string_lossy().into_owned();
        let payload = js_update_payload(&[path.clone()]).expect("js-update");
        let json: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(json["type"], "js-update");
        assert_eq!(json["modules"][0]["id"], "Label.js");
        assert_eq!(json["modules"][0]["url"], "/_deka/hmr/module/Label.js");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn server_handler_edit_does_not_claim_fast_refresh() {
        assert!(js_update_payload(&["/proj/index.html".to_string()]).is_none());
        let _lock = crate::react_refresh::compile::TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = project("handler-js");
        fs::write(
            root.join("index.js"),
            "globalThis.app = { async fetch() { return new Response('ok'); } };\n",
        )
        .unwrap();
        let path = root.join("index.js").to_string_lossy().into_owned();
        assert!(
            js_update_payload(&[path]).is_none(),
            "a WinterTC handler is not a React refresh boundary"
        );
        let _ = fs::remove_dir_all(root);
    }
}
