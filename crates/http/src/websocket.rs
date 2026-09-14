use axum::extract::ws::{CloseCode, CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use tokio::sync::mpsc;

use engine::{RuntimeState, execute_request_parts};
use pool::RequestData;
static NEXT_WS_ID: AtomicU64 = AtomicU64::new(1);
struct WsEntry {
    sender: mpsc::UnboundedSender<Message>,
    pending: Arc<AtomicUsize>,
}

static WS_REGISTRY: OnceLock<Mutex<HashMap<u64, WsEntry>>> = OnceLock::new();
struct HmrEntry {
    sender: mpsc::UnboundedSender<Message>,
    path: String,
}
struct HmrSnapshot {
    selector: String,
    container_html: String,
    node_html: HashMap<String, String>,
    island_html: HashMap<String, String>,
}
static HMR_REGISTRY: OnceLock<Mutex<HashMap<u64, HmrEntry>>> = OnceLock::new();
static HMR_RUNTIME_STATE: OnceLock<Arc<RuntimeState>> = OnceLock::new();
static HMR_SNAPSHOTS: OnceLock<Mutex<HashMap<String, HmrSnapshot>>> = OnceLock::new();

pub fn set_hmr_runtime_state(state: Arc<RuntimeState>) {
    let _ = HMR_RUNTIME_STATE.set(state);
}

pub fn register_sender(sender: mpsc::UnboundedSender<Message>, pending: Arc<AtomicUsize>) -> u64 {
    let id = NEXT_WS_ID.fetch_add(1, Ordering::Relaxed);
    let registry = WS_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = registry.lock() {
        guard.insert(id, WsEntry { sender, pending });
    }
    id
}

pub fn unregister_sender(id: u64) {
    if let Some(registry) = WS_REGISTRY.get() {
        if let Ok(mut guard) = registry.lock() {
            guard.remove(&id);
        }
    }
}

pub async fn handle_hmr_websocket(socket: WebSocket, _state: Arc<RuntimeState>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let id = NEXT_WS_ID.fetch_add(1, Ordering::Relaxed);
    let registry = HMR_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = registry.lock() {
        guard.insert(
            id,
            HmrEntry {
                sender: tx,
                path: "/".to_string(),
            },
        );
    }

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let write_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if ws_sender.send(message).await.is_err() {
                break;
            }
        }
    });

    while let Some(message) = ws_receiver.next().await {
        match message {
            Ok(Message::Text(text)) => {
                update_hmr_path(id, &text);
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }

    if let Some(registry) = HMR_REGISTRY.get() {
        if let Ok(mut guard) = registry.lock() {
            guard.remove(&id);
        }
    }
    write_task.abort();
}

pub fn broadcast_hmr_text(payload: String) {
    let Some(registry) = HMR_REGISTRY.get() else {
        return;
    };
    let mut clients = Vec::new();
    if let Ok(guard) = registry.lock() {
        for (id, entry) in guard.iter() {
            clients.push((*id, entry.sender.clone()));
        }
    }
    let mut dead = Vec::new();
    for (id, sender) in clients {
        if sender.send(Message::Text(payload.clone())).is_err() {
            dead.push(id);
        }
    }
    if dead.is_empty() {
        return;
    }
    if let Ok(mut guard) = registry.lock() {
        for id in dead {
            guard.remove(&id);
        }
    }
}

pub fn broadcast_hmr_changed(paths: &[String]) {
    let Some(state) = HMR_RUNTIME_STATE.get().cloned() else {
        return;
    };
    let Some(registry) = HMR_REGISTRY.get() else {
        return;
    };
    let mut clients = Vec::new();
    if let Ok(guard) = registry.lock() {
        for (id, entry) in guard.iter() {
            clients.push((*id, entry.path.clone(), entry.sender.clone()));
        }
    }
    if clients.is_empty() {
        return;
    }
    let changed = paths.to_vec();
    tokio::spawn(async move {
        let mut dead = Vec::new();
        for (id, path, sender) in clients {
            let payload = render_hmr_payload(Arc::clone(&state), &path, &changed).await;
            if sender.send(Message::Text(payload)).is_err() {
                dead.push(id);
            }
        }
        if dead.is_empty() {
            return;
        }
        if let Some(registry) = HMR_REGISTRY.get() {
            if let Ok(mut guard) = registry.lock() {
                for id in dead {
                    guard.remove(&id);
                }
            }
        }
    });
}

fn update_hmr_path(id: u64, text: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    if value.get("type").and_then(|v| v.as_str()) != Some("subscribe") {
        return;
    }
    let Some(path) = value.get("path").and_then(|v| v.as_str()) else {
        return;
    };
    if path.is_empty() {
        return;
    }
    if let Some(registry) = HMR_REGISTRY.get() {
        if let Ok(mut guard) = registry.lock() {
            if let Some(entry) = guard.get_mut(&id) {
                entry.path = path.to_string();
            }
        }
    }
}

async fn render_hmr_payload(
    state: Arc<RuntimeState>,
    path: &str,
    changed_paths: &[String],
) -> String {
    let mut headers = Vec::new();
    headers.push(("accept".to_string(), "text/x-deka-fragment".to_string()));
    let uri = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };
    let response = execute_request_parts(
        Arc::clone(&state),
        format!("http://localhost{}", uri),
        "GET".to_string(),
        headers,
        None,
    )
    .await;
    let Ok(response) = response else {
        return reload_payload(changed_paths);
    };
    if response.status >= 400 || response.body_base64.is_some() {
        return reload_payload(changed_paths);
    }
    if let Some(html) = partial_html_from_response(&response) {
        let _ = build_patch_from_snapshot(path, changed_paths, "#app", &html);
        return html_update_payload(changed_paths, "#app", &html);
    }

    let response = execute_request_parts(
        Arc::clone(&state),
        format!("http://localhost{}", uri),
        "GET".to_string(),
        Vec::new(),
        None,
    )
    .await;
    let Ok(response) = response else {
        return reload_payload(changed_paths);
    };
    if response.status >= 400 || response.body_base64.is_some() {
        return reload_payload(changed_paths);
    }
    if let Some(html) = extract_container_inner_html(&response.body, "app") {
        let _ = build_patch_from_snapshot(path, changed_paths, "#app", &html);
        return html_update_payload(changed_paths, "#app", &html);
    }
    if let Some(html) = extract_container_inner_html(&response.body, "body") {
        let _ = build_patch_from_snapshot(path, changed_paths, "body", &html);
        return html_update_payload(changed_paths, "body", &html);
    }
    reload_payload(changed_paths)
}

pub(crate) fn html_update_payload(changed_paths: &[String], selector: &str, html: &str) -> String {
    serde_json::json!({
        "type": "html-update",
        "schema": 1,
        "paths": changed_paths,
        "selector": selector,
        "html": html,
    })
    .to_string()
}

pub(crate) fn build_patch_from_snapshot(
    path: &str,
    changed_paths: &[String],
    selector: &str,
    html: &str,
) -> String {
    let snapshots = HMR_SNAPSHOTS.get_or_init(|| Mutex::new(HashMap::new()));
    let new_map = collect_deka_nodes(html);
    let new_islands = collect_islands(html);
    let mut ops = Vec::new();
    if let Ok(mut guard) = snapshots.lock() {
        if let Some(prev) = guard.get(path) {
            if prev.selector == selector
                && prev.container_html != html
                && !prev.node_html.is_empty()
                && !new_map.is_empty()
            {
                let mut changed_ids = Vec::new();
                let mut structure_changed = false;
                for id in prev.node_html.keys() {
                    if !new_map.contains_key(id) {
                        structure_changed = true;
                        break;
                    }
                }
                if !structure_changed {
                    for (id, next_html) in &new_map {
                        match prev.node_html.get(id) {
                            Some(prev_html) if prev_html == next_html => {}
                            Some(_) => changed_ids.push(id.clone()),
                            None => {
                                structure_changed = true;
                                break;
                            }
                        }
                    }
                }
                if !structure_changed && !changed_ids.is_empty() && changed_ids.len() <= 32 {
                    changed_ids.sort();
                    for id in changed_ids {
                        if let Some(next_html) = new_map.get(&id) {
                            ops.push(serde_json::json!({
                                "op": "set_html",
                                "selector": format!("[data-deka-id=\"{}\"]", id),
                                "html": next_html,
                            }));
                        }
                    }
                } else if structure_changed
                    && !prev.island_html.is_empty()
                    && !new_islands.is_empty()
                {
                    let mut changed_islands = Vec::new();
                    let mut islands_stable = true;
                    for id in prev.island_html.keys() {
                        if !new_islands.contains_key(id) {
                            islands_stable = false;
                            break;
                        }
                    }
                    if islands_stable {
                        for (id, next_html) in &new_islands {
                            match prev.island_html.get(id) {
                                Some(prev_html) if prev_html == next_html => {}
                                Some(_) => changed_islands.push(id.clone()),
                                None => {
                                    islands_stable = false;
                                    break;
                                }
                            }
                        }
                    }
                    if islands_stable && !changed_islands.is_empty() && changed_islands.len() <= 16
                    {
                        changed_islands.sort();
                        for id in changed_islands {
                            if let Some(next_html) = new_islands.get(&id) {
                                let (name, occurrence) = split_island_key(&id);
                                // Keyed by marker identity, not a selector:
                                // islands render as comment markers
                                // (the paused ui/server runtime) with no
                                // wrapper element to select, so the client
                                // locates the `occurrence`-th start marker
                                // for `name` and replaces the range between
                                // it and the matching end marker.
                                ops.push(serde_json::json!({
                                    "op": "set_html",
                                    "island": name,
                                    "occurrence": occurrence,
                                    "html": next_html,
                                }));
                            }
                        }
                    }
                }
            }
        }
        guard.insert(
            path.to_string(),
            HmrSnapshot {
                selector: selector.to_string(),
                container_html: html.to_string(),
                node_html: new_map,
                island_html: new_islands,
            },
        );
    }
    if ops.is_empty() {
        ops.push(serde_json::json!({
            "op": "set_html",
            "selector": selector,
            "html": html,
        }));
    }
    serde_json::json!({
        "type": "patch",
        "schema": 1,
        "paths": changed_paths,
        "ops": ops,
    })
    .to_string()
}

fn partial_html_from_response(response: &engine::ResponseEnvelope) -> Option<String> {
    let content_type = response
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_ascii_lowercase())
        .unwrap_or_default();
    let looks_json = content_type.contains("json")
        || content_type.contains("x-deka-fragment")
        || content_type.contains("x-phpx-fragment")
        || response.body.trim_start().starts_with('{');
    if !looks_json {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(&response.body).ok()?;
    value
        .get("html")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

pub(crate) fn reload_payload(changed_paths: &[String]) -> String {
    serde_json::json!({
        "type": "reload",
        "paths": changed_paths,
    })
    .to_string()
}

pub(crate) fn island_reload_payload(changed_paths: &[String]) -> String {
    serde_json::json!({
        "type": "reload",
        "schema": 1,
        "reason": "island-source",
        "paths": changed_paths,
    })
    .to_string()
}

/// Full reload for a root `index.html` document-shell edit (deka#1048). The
/// shell (title, head, wrapper markup, <script> tags) lives outside the
/// `#app` container the html-update path morphs, so it cannot be reflected
/// by patching in place — a real navigation is the only way to pick it up.
pub(crate) fn document_reload_payload(changed_paths: &[String]) -> String {
    serde_json::json!({
        "type": "reload",
        "schema": 1,
        "reason": "document-source",
        "paths": changed_paths,
    })
    .to_string()
}

fn extract_container_inner_html(html: &str, id: &str) -> Option<String> {
    let needle_a = format!("id=\"{}\"", id);
    let needle_b = format!("id='{}'", id);
    let id_pos = html.find(&needle_a).or_else(|| html.find(&needle_b))?;
    let start_tag = html[..id_pos].rfind('<')?;
    let open_end_rel = html[start_tag..].find('>')?;
    let open_end = start_tag + open_end_rel;
    let tag_name = read_tag_name(&html[start_tag + 1..open_end])?;
    let close_tag = format!("</{}>", tag_name);
    let open_tag_prefix = format!("<{}", tag_name);
    let mut depth = 1usize;
    let mut cursor = open_end + 1;
    while cursor < html.len() {
        let next_open = html[cursor..].find(&open_tag_prefix).map(|v| cursor + v);
        let next_close = html[cursor..].find(&close_tag).map(|v| cursor + v);
        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                cursor = o + open_tag_prefix.len();
            }
            (_, Some(c)) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(html[open_end + 1..c].to_string());
                }
                cursor = c + close_tag.len();
            }
            _ => break,
        }
    }
    None
}

fn read_tag_name(fragment: &str) -> Option<String> {
    let trimmed = fragment.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = String::new();
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else {
            break;
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn collect_deka_nodes(container_html: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut offset = 0usize;
    let needle = "data-deka-id=\"";
    while let Some(pos) = container_html[offset..].find(needle) {
        let abs = offset + pos;
        let id_start = abs + needle.len();
        let Some(id_end_rel) = container_html[id_start..].find('"') else {
            break;
        };
        let id_end = id_start + id_end_rel;
        let id = &container_html[id_start..id_end];
        if !id.is_empty() {
            if let Some(inner) =
                extract_element_inner_by_attr(container_html, "data-deka-id", id, abs)
            {
                out.insert(id.to_string(), inner);
            }
        }
        offset = id_end + 1;
    }
    out
}

fn collect_islands(container_html: &str) -> HashMap<String, String> {
    // Islands ship as HTML comment markers. The grammar is defined once in
    // the paused ui runtime's island-marker module (producer and consumer
    // both derive from it); this scanner only matches the literal
    // `deka-island start:` / `deka-island end:` prefixes, so keep
    // ISLAND_MARKER_TAG stable there:
    //   <!--deka-island start:<b64 name> directive:<b64> [props:<b64>] [id:<b64>] ...-->
    //   ...island body...
    //   <!--deka-island end:<b64 name>-->
    // The island's stable identity is the decoded component name from the start
    // marker; the body is the markup between the start and end markers.
    let mut out = HashMap::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut offset = 0usize;
    let start_needle = "<!--deka-island start:";
    let marker_needle = "<!--deka-island ";
    while let Some(pos) = container_html[offset..].find(start_needle) {
        let abs = offset + pos;
        let header_start = abs + start_needle.len();
        let Some(header_end_rel) = container_html[header_start..].find("-->") else {
            break;
        };
        let header_end = header_start + header_end_rel;
        let header = &container_html[header_start..header_end];
        let Some(name) = header
            .split(' ')
            .next()
            .and_then(base64_decode)
            .filter(|name| !name.is_empty())
        else {
            offset = header_end + 3;
            continue;
        };
        // Bound the island body at the end marker that closes the start
        // marker's depth, so nested islands stay intact.
        let body_start = header_end + 3;
        let mut depth = 1usize;
        let mut cursor = body_start;
        let mut body_end = None;
        while let Some(rel) = container_html[cursor..].find(marker_needle) {
            let marker_abs = cursor + rel;
            let after = marker_abs + marker_needle.len();
            if container_html[after..].starts_with("start:") {
                depth += 1;
            } else if container_html[after..].starts_with("end:") {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    body_end = Some(marker_abs);
                    break;
                }
            }
            cursor = after;
        }
        let Some(body_end) = body_end else {
            offset = body_start;
            continue;
        };
        // Disambiguate repeated instances of the same component by document
        // order so each instance keeps its own entry.
        let occurrence = counts.entry(name.clone()).or_insert(0);
        *occurrence += 1;
        let key = if *occurrence == 1 {
            name
        } else {
            format!("{}#{}", name, occurrence)
        };
        out.insert(key, container_html[body_start..body_end].to_string());
        offset = body_end;
    }
    out
}

fn base64_decode(value: &str) -> Option<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value.as_bytes())
        .ok()?;
    String::from_utf8(bytes).ok()
}

// Inverse of the occurrence keys built in `collect_islands`: "Name" is the
// first instance of a component, "Name#2" the second, and so on.
fn split_island_key(key: &str) -> (String, usize) {
    match key.rsplit_once('#') {
        Some((name, occ)) if !occ.is_empty() && occ.bytes().all(|b| b.is_ascii_digit()) => {
            (name.to_string(), occ.parse::<usize>().unwrap_or(1).max(1))
        }
        _ => (key.to_string(), 1),
    }
}

fn extract_element_inner_by_attr(
    html: &str,
    attr_name: &str,
    attr_value: &str,
    hint_pos: usize,
) -> Option<String> {
    let needle = format!("{}=\"{}\"", attr_name, attr_value);
    let attr_pos = if hint_pos < html.len() && html[hint_pos..].starts_with(&needle) {
        hint_pos
    } else {
        html.find(&needle)?
    };
    let start_tag = html[..attr_pos].rfind('<')?;
    let open_end_rel = html[start_tag..].find('>')?;
    let open_end = start_tag + open_end_rel;
    let tag_name = read_tag_name(&html[start_tag + 1..open_end])?;
    let close_tag = format!("</{}>", tag_name);
    let open_tag_prefix = format!("<{}", tag_name);
    let mut depth = 1usize;
    let mut cursor = open_end + 1;
    while cursor < html.len() {
        let next_open = html[cursor..].find(&open_tag_prefix).map(|v| cursor + v);
        let next_close = html[cursor..].find(&close_tag).map(|v| cursor + v);
        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                cursor = o + open_tag_prefix.len();
            }
            (_, Some(c)) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(html[open_end + 1..c].to_string());
                }
                cursor = c + close_tag.len();
            }
            _ => break,
        }
    }
    None
}

pub fn send_text(id: u64, message: String) -> Result<(), String> {
    if let Some(registry) = WS_REGISTRY.get() {
        if let Ok(guard) = registry.lock() {
            if let Some(entry) = guard.get(&id) {
                entry
                    .sender
                    .send(Message::Text(message))
                    .map_err(|err| err.to_string())?;
                entry.pending.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        }
    }
    Err(format!("WebSocket {} not found", id))
}

pub fn send_binary(id: u64, data: &[u8]) -> Result<(), String> {
    if let Some(registry) = WS_REGISTRY.get() {
        if let Ok(guard) = registry.lock() {
            if let Some(entry) = guard.get(&id) {
                entry
                    .sender
                    .send(Message::Binary(data.to_vec()))
                    .map_err(|err| err.to_string())?;
                entry.pending.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        }
    }
    Err(format!("WebSocket {} not found", id))
}

pub fn close_socket(id: u64, code: u16, reason: String) -> Result<(), String> {
    if let Some(registry) = WS_REGISTRY.get() {
        if let Ok(guard) = registry.lock() {
            if let Some(entry) = guard.get(&id) {
                let frame = CloseFrame {
                    code: CloseCode::from(code),
                    reason: reason.into(),
                };
                entry
                    .sender
                    .send(Message::Close(Some(frame)))
                    .map_err(|err| err.to_string())?;
                return Ok(());
            }
        }
    }
    Err(format!("WebSocket {} not found", id))
}

pub async fn handle_websocket(socket: WebSocket, state: Arc<RuntimeState>, upgrade: Option<Value>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let pending = Arc::new(AtomicUsize::new(0));
    let id = register_sender(tx, Arc::clone(&pending));

    let upgrade_data = upgrade
        .and_then(|value| value.get("data").cloned())
        .unwrap_or(Value::Null);

    let engine = Arc::clone(&state.engine);
    let handler_key = state.handler_key.clone();
    let handler_code = state.handler_code.clone();

    emit_event(
        Arc::clone(&engine),
        handler_key.clone(),
        handler_code.clone(),
        serde_json::json!({
            "__dekaWsEvent": "open",
            "__dekaWsId": id,
            "__dekaWsData": upgrade_data,
        }),
    )
    .await;

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let drain_engine = Arc::clone(&engine);
    let drain_handler_key = handler_key.clone();
    let drain_handler_code = handler_code.clone();
    let drain_pending = Arc::clone(&pending);
    let write_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if ws_sender.send(message).await.is_err() {
                break;
            }
            if drain_pending.fetch_sub(1, Ordering::Relaxed) == 1 {
                let _ = emit_event(
                    Arc::clone(&drain_engine),
                    drain_handler_key.clone(),
                    drain_handler_code.clone(),
                    serde_json::json!({
                        "__dekaWsEvent": "drain",
                        "__dekaWsId": id,
                    }),
                )
                .await;
            }
        }
    });

    while let Some(message) = ws_receiver.next().await {
        match message {
            Ok(Message::Text(text)) => {
                emit_event(
                    Arc::clone(&engine),
                    handler_key.clone(),
                    handler_code.clone(),
                    serde_json::json!({
                        "__dekaWsEvent": "message",
                        "__dekaWsId": id,
                        "__dekaWsMessage": text,
                    }),
                )
                .await;
            }
            Ok(Message::Binary(bytes)) => {
                let data = bytes.into_iter().collect::<Vec<u8>>();
                emit_event(
                    Arc::clone(&engine),
                    handler_key.clone(),
                    handler_code.clone(),
                    serde_json::json!({
                        "__dekaWsEvent": "message",
                        "__dekaWsId": id,
                        "__dekaWsBinary": true,
                        "__dekaWsMessage": data,
                    }),
                )
                .await;
            }
            Ok(Message::Close(frame)) => {
                emit_event(
                    Arc::clone(&engine),
                    handler_key.clone(),
                    handler_code.clone(),
                    serde_json::json!({
                        "__dekaWsEvent": "close",
                        "__dekaWsId": id,
                        "__dekaWsCode": frame.as_ref().map(|f| u16::from(f.code)),
                        "__dekaWsReason": frame.as_ref().map(|f| f.reason.as_ref()),
                    }),
                )
                .await;
                break;
            }
            Err(_) => break,
            _ => {}
        }
    }

    unregister_sender(id);
    write_task.abort();
}

async fn emit_event(
    engine: Arc<engine::RuntimeEngine>,
    handler_key: pool::HandlerKey,
    handler_code: String,
    payload: Value,
) {
    let _ = engine
        .execute(
            handler_key,
            RequestData {
                handler_code,
                request_value: payload,
                ..Default::default()
            },
        )
        .await;
}
