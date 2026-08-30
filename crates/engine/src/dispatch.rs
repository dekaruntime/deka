use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::RuntimeState;
use crate::envelope::{RequestEnvelope, ResponseEnvelope};
use pool::RequestParts;
use pool::{ExecutionMode, RequestData};
use runtime_core::framework::{
    self, matcher_hits, parse_middleware_matcher, public_file_exists, skip_middleware_path,
    trailing_slash_redirect, MIDDLEWARE_NEXT_STATUS,
};
use runtime_core::storefront_envelope::StorefrontResponse;

async fn execute_request_data(
    state: Arc<RuntimeState>,
    request_data: RequestData,
) -> Result<ResponseEnvelope, String> {
    let pool_response = state
        .engine
        .execute(state.handler_key.clone(), request_data)
        .await
        .map_err(|err| format!("handler execution failed: {}", err))?;

    if !pool_response.success {
        let error_msg = pool_response
            .error
            .unwrap_or_else(|| "Unknown error".to_string());
        return Err(format!("handler execution failed: {}", error_msg));
    }

    tracing::debug!(
        "Request completed - warm: {}µs, total: {}µs, cache_hit: {}",
        pool_response.warm_time_us,
        pool_response.total_time_us,
        pool_response.cache_hit
    );

    let result = pool_response
        .result
        .ok_or_else(|| "handler returned no result".to_string())?;

    ResponseEnvelope::from_value(result)
        .map_err(|err| format!("handler returned invalid response: {}", err))
}

pub async fn execute_request(
    state: Arc<RuntimeState>,
    request: RequestEnvelope,
) -> Result<ResponseEnvelope, String> {
    let request_parts = pool::RequestParts {
        url: request.url.clone(),
        method: request.method.clone(),
        headers: request.headers.into_iter().collect(),
        body: request.body.clone(),
    };

    let request_data = RequestData {
        handler_code: state.handler_code.clone(),
        handler_entry: state.handler_entry.clone(),
        request_value: serde_json::Value::Null,
        request_parts: Some(request_parts),
        mode: ExecutionMode::Request,
    };

    execute_request_data(state, request_data).await
}

pub async fn execute_request_parts(
    state: Arc<RuntimeState>,
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
) -> Result<ResponseEnvelope, String> {
    let project_root = state
        .handler_entry
        .as_deref()
        .and_then(project_root_from_generated_entry);
    let want_trailing = project_root
        .as_deref()
        .map(read_trailing_slash)
        .unwrap_or(false);
    if let Some(location) = trailing_slash_redirect(&url, want_trailing) {
        let mut location_headers = HashMap::new();
        location_headers.insert("location".to_string(), location);
        return Ok(StorefrontResponse {
            status: 301,
            headers: location_headers,
            body: String::new(),
            body_base64: None,
            upgrade: None,
        });
    }

    let path = framework::request_path_from_url(&url);
    if let Some(mw_resp) = run_middleware_if_needed(
        Arc::clone(&state),
        project_root.as_deref(),
        &path,
        url.clone(),
        method.clone(),
        headers.clone(),
        body.clone(),
    )
    .await?
    {
        return Ok(mw_resp);
    }

    let handler_entry = api_entry_override(state.handler_entry.clone(), &url);
    let is_head = method.eq_ignore_ascii_case("HEAD");
    let request_parts = RequestParts {
        url,
        method,
        headers,
        body,
    };

    let request_data = RequestData {
        handler_code: state.handler_code.clone(),
        handler_entry,
        request_value: serde_json::Value::Null,
        request_parts: Some(request_parts),
        mode: ExecutionMode::Request,
    };

    let mut response = execute_request_data(state, request_data).await?;
    if is_head {
        response.body.clear();
        response.body_base64 = None;
    }
    Ok(response)
}

async fn run_middleware_if_needed(
    state: Arc<RuntimeState>,
    project_root: Option<&Path>,
    path: &str,
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
) -> Result<Option<ResponseEnvelope>, String> {
    let Some(page_entry) = state.handler_entry.as_ref() else {
        return Ok(None);
    };
    if skip_middleware_path(path) {
        return Ok(None);
    }
    if let Some(root) = project_root {
        if public_file_exists(root, path) {
            return Ok(None);
        }
        if let Some(src) = std::fs::read_to_string(root.join(framework::MIDDLEWARE_FILE)).ok() {
            match parse_middleware_matcher(&src) {
                Some(patterns) if !matcher_hits(&patterns, path) => return Ok(None),
                _ => {}
            }
        }
    }
    let mw_entry = Path::new(page_entry).with_file_name("middleware-entry.ds");
    if !mw_entry.is_file() {
        return Ok(None);
    }
    let request_data = RequestData {
        handler_code: state.handler_code.clone(),
        handler_entry: Some(mw_entry.to_string_lossy().into_owned()),
        request_value: serde_json::Value::Null,
        request_parts: Some(RequestParts {
            url,
            method,
            headers,
            body,
        }),
        mode: ExecutionMode::Request,
    };
    let response = execute_request_data(state, request_data).await?;
    if response.status == MIDDLEWARE_NEXT_STATUS {
        Ok(None)
    } else {
        Ok(Some(response))
    }
}

fn project_root_from_generated_entry(entry: &str) -> Option<PathBuf> {
    let mut current = Path::new(entry).parent()?;
    loop {
        if current.join("deka.json").is_file() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

fn read_trailing_slash(project_root: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(project_root.join("deka.json")) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value
        .get("serve")
        .and_then(|serve| serve.get("trailingSlash").or_else(|| serve.get("trailing_slash")))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn api_entry_override(page_entry: Option<String>, url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    let path = path.split("://").nth(1).unwrap_or(path);
    let path = path.find('/').map(|i| &path[i..]).unwrap_or("/");
    if path == "/_deka/defer" || path.starts_with("/_deka/defer?") {
        let Some(page_entry) = page_entry else {
            return None;
        };
        let defer_entry = std::path::Path::new(&page_entry).with_file_name("defer-entry.dsx");
        if defer_entry.is_file() {
            return Some(defer_entry.to_string_lossy().into_owned());
        }
        return Some(page_entry);
    }
    if !(path == "/api" || path.starts_with("/api/")) {
        return page_entry;
    }
    let Some(page_entry) = page_entry else {
        return None;
    };
    let api_entry = std::path::Path::new(&page_entry).with_file_name("api-entry.ds");
    if api_entry.is_file() {
        Some(api_entry.to_string_lossy().into_owned())
    } else {
        Some(page_entry)
    }
}

pub async fn execute_request_value(
    state: Arc<RuntimeState>,
    request_value: serde_json::Value,
) -> Result<ResponseEnvelope, String> {
    let request_data = RequestData {
        handler_code: state.handler_code.clone(),
        handler_entry: state.handler_entry.clone(),
        request_value,
        request_parts: None,
        mode: ExecutionMode::Request,
    };

    execute_request_data(state, request_data).await
}
