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
use pool::{ExecutionMode, RequestData};

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
        return build_patch_from_snapshot(path, changed_paths, "#app", &html);
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
        return build_patch_from_snapshot(path, changed_paths, "#app", &html);
    }
    if let Some(html) = extract_container_inner_html(&response.body, "body") {
        return build_patch_from_snapshot(path, changed_paths, "body", &html);
    }
    reload_payload(changed_paths)
}

fn build_patch_from_snapshot(
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
                                // (crates/deka_ui/js/server.js) with no
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

fn reload_payload(changed_paths: &[String]) -> String {
    serde_json::json!({
        "type": "reload",
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
    // Islands ship as HTML comment markers (see crates/deka_ui/js/server.js):
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
                handler_entry: None,
                request_value: payload,
                request_parts: None,
                mode: ExecutionMode::Request,
            },
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::build_patch_from_snapshot;
    use serde_json::Value;

    fn parse(payload: &str) -> Value {
        serde_json::from_str(payload).expect("valid json payload")
    }

    #[test]
    fn first_snapshot_uses_container_replace() {
        let payload = build_patch_from_snapshot(
            "/__hmr_test_first",
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Hello</div>",
        );
        let json = parse(&payload);
        assert_eq!(json["type"], "patch");
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["op"], "set_html");
        assert_eq!(json["ops"][0]["selector"], "#app");
    }

    #[test]
    fn stable_structure_produces_granular_node_patch() {
        let path = "/__hmr_test_granular";
        let first = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Hello</div><span data-deka-id=\"b\">World</span>",
        );
        let first_json = parse(&first);
        assert_eq!(first_json["ops"][0]["selector"], "#app");

        let second = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Hello 2</div><span data-deka-id=\"b\">World</span>",
        );
        let second_json = parse(&second);
        assert_eq!(second_json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(second_json["ops"][0]["selector"], "[data-deka-id=\"a\"]");
        assert_eq!(second_json["ops"][0]["html"], "Hello 2");
    }

    fn b64(value: &str) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
    }

    // Builds an island exactly the way crates/deka_ui/js/server.js:545 emits
    // it: comment markers carrying the b64-encoded component name, directive,
    // and props around the server-rendered body.
    fn island(name: &str, directive: &str, props: &str, body: &str) -> String {
        format!(
            "<!--deka-island start:{} directive:{} props:{}-->{}<!--deka-island end:{}-->",
            b64(name),
            b64(directive),
            b64(props),
            body,
            b64(name)
        )
    }

    #[test]
    fn structural_changes_fallback_to_island_patch_when_possible() {
        let path = "/__hmr_test_island";
        let _ = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &island("Widget", "load", "{}", "<div data-deka-id=\"n1\">A</div>"),
        );
        let payload = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &island("Widget", "load", "{}", "<section data-deka-id=\"n2\">B</section>"),
        );
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["island"], "Widget");
        assert_eq!(json["ops"][0]["occurrence"], 1);
        assert_eq!(
            json["ops"][0]["html"],
            "<section data-deka-id=\"n2\">B</section>"
        );
    }

    #[test]
    fn multiple_islands_patch_only_the_changed_island() {
        let path = "/__hmr_test_island_multi";
        let _ = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &format!(
                "{}{}",
                island("WidgetA", "load", "{}", "<div data-deka-id=\"a\">A</div>"),
                island("WidgetB", "load", "{}", "<div data-deka-id=\"b\">B</div>")
            ),
        );
        let payload = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &format!(
                "{}{}",
                island("WidgetA", "load", "{}", "<div data-deka-id=\"a\">A</div>"),
                island("WidgetB", "load", "{}", "<section data-deka-id=\"b2\">B2</section>")
            ),
        );
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["island"], "WidgetB");
        assert_eq!(json["ops"][0]["occurrence"], 1);
        assert_eq!(
            json["ops"][0]["html"],
            "<section data-deka-id=\"b2\">B2</section>"
        );
    }

    #[test]
    fn island_name_change_forces_container_fallback_patch() {
        let path = "/__hmr_test_island_id_change";
        let _ = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &island("WidgetA", "load", "{}", "<div data-deka-id=\"n1\">A</div>"),
        );
        let payload = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &island("WidgetB", "load", "{}", "<div data-deka-id=\"n1\">A</div>"),
        );
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["selector"], "#app");
    }

    #[test]
    fn island_patch_works_with_real_rendered_marker_shape() {
        // Regression test for the marker/element drift: snapshots must be
        // shaped like actual server.js output (padded b64, real props JSON),
        // not like synthetic wrapper elements.
        let path = "/__hmr_test_island_real_shape";
        let first = format!(
            "{}<p>static</p>",
            island("Counter", "load", "{\"count\":1}", "<button data-deka-id=\"test:Counter/i0\">0</button>")
        );
        let _ = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &first);
        let second = format!(
            "{}<p>static</p>",
            island("Counter", "load", "{\"count\":1}", "<button data-deka-id=\"test:Counter/i1\">1</button>")
        );
        let payload = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &second);
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["island"], "Counter");
        assert_eq!(json["ops"][0]["occurrence"], 1);
        assert_eq!(
            json["ops"][0]["html"],
            "<button data-deka-id=\"test:Counter/i1\">1</button>"
        );
    }

    #[test]
    fn defer_shaped_markers_with_extra_fields_are_collected() {
        // wrapDeferred (server.js:433) emits markers with id/enc/cache fields
        // and no props; those must be located too.
        let path = "/__hmr_test_island_defer_shape";
        let marker_a = format!(
            "<!--deka-island start:{} directive:{} id:{} cache:{}--><span data-deka-defer=\"D:1\"><span data-deka-id=\"test:_/i0/i0\" slot=\"fallback\">.</span></span><!--deka-island end:{}-->",
            b64("Badge"),
            b64("defer"),
            b64("D:1"),
            b64("60s"),
            b64("Badge")
        );
        let _ = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &marker_a);
        let marker_b = format!(
            "<!--deka-island start:{} directive:{} id:{} cache:{}--><span data-deka-defer=\"D:1\"><span data-deka-id=\"test:_/i0/i1\" slot=\"fallback\">!</span></span><!--deka-island end:{}-->",
            b64("Badge"),
            b64("defer"),
            b64("D:1"),
            b64("60s"),
            b64("Badge")
        );
        let payload = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &marker_b);
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["island"], "Badge");
        assert_eq!(json["ops"][0]["occurrence"], 1);
        assert_eq!(
            json["ops"][0]["html"],
            "<span data-deka-defer=\"D:1\"><span data-deka-id=\"test:_/i0/i1\" slot=\"fallback\">!</span></span>"
        );
    }

    #[test]
    fn repeated_island_names_patch_by_occurrence() {
        let path = "/__hmr_test_island_duplicate_names";
        let _ = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &format!(
                "{}{}",
                island("Counter", "load", "{}", "<i data-deka-id=\"a1\">0</i>"),
                island("Counter", "load", "{}", "<i data-deka-id=\"b1\">10</i>")
            ),
        );
        let payload = build_patch_from_snapshot(
            path,
            &["main.phpx".to_string()],
            "#app",
            &format!(
                "{}{}",
                island("Counter", "load", "{}", "<i data-deka-id=\"a1\">0</i>"),
                island("Counter", "load", "{}", "<i data-deka-id=\"b2\">11</i>")
            ),
        );
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["island"], "Counter");
        assert_eq!(json["ops"][0]["occurrence"], 2);
        assert_eq!(json["ops"][0]["html"], "<i data-deka-id=\"b2\">11</i>");
    }

    // Mirrors the client-side resolution in the HMR client (crates/http/src/
    // router.rs): find the `occurrence`-th start marker for `name`, walk to
    // the end marker that closes its depth, and return the body between the
    // two. Kept separate from `collect_islands` on purpose: the test must
    // resolve the op the way the browser will, not the way the producer
    // parses.
    fn resolve_island_body(html: &str, name: &str, occurrence: usize) -> Option<String> {
        let name_b64 = b64(name);
        let start_needle = "<!--deka-island start:";
        let marker_needle = "<!--deka-island ";
        let mut seen = 0usize;
        let mut offset = 0usize;
        while let Some(pos) = html[offset..].find(start_needle) {
            let abs = offset + pos;
            let header_start = abs + start_needle.len();
            let header_end = header_start + html[header_start..].find("-->")?;
            if html[header_start..header_end].split(' ').next() == Some(name_b64.as_str()) {
                seen += 1;
                if seen == occurrence {
                    let body_start = header_end + 3;
                    let mut depth = 1usize;
                    let mut cursor = body_start;
                    while let Some(rel) = html[cursor..].find(marker_needle) {
                        let marker_abs = cursor + rel;
                        let after = marker_abs + marker_needle.len();
                        if html[after..].starts_with("start:") {
                            depth += 1;
                        } else if html[after..].starts_with("end:") {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                return Some(html[body_start..marker_abs].to_string());
                            }
                        }
                        cursor = after;
                    }
                    return None;
                }
            }
            offset = header_end + 3;
        }
        None
    }

    #[test]
    fn island_op_resolves_to_body_between_server_markers() {
        // Asserts the EFFECT, not the op encoding: given the island HTML
        // server.js actually emits (comment markers around a fragment body
        // with no wrapper element), the op's island identity must resolve to
        // the exact range the client will replace. The old
        // `[data-deka-island-id=...]` selector could never be checked this
        // way, which is how the drift shipped.
        let path = "/__hmr_test_island_effect";
        let first = format!(
            "<main>{}<p>static</p></main>",
            island("Widget", "load", "{}", "<b data-deka-id=\"w1\">old</b><i>body</i>")
        );
        let _ = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &first);
        let second = format!(
            "<main>{}<p>static</p></main>",
            island("Widget", "load", "{}", "<b data-deka-id=\"w2\">new</b><i>body</i>")
        );
        let payload = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &second);
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        let op = &json["ops"][0];
        assert_eq!(op["island"], "Widget");
        assert_eq!(op["occurrence"], 1);
        // The identity must resolve to the island range in the live document
        // (modelled here by the previous render) ...
        let name = op["island"].as_str().unwrap_or_default().to_string();
        let occurrence = op["occurrence"].as_u64().unwrap_or(1) as usize;
        assert_eq!(
            resolve_island_body(&first, &name, occurrence).as_deref(),
            Some("<b data-deka-id=\"w1\">old</b><i>body</i>")
        );
        // ... and the op payload must be exactly the new body between the
        // same markers.
        assert_eq!(
            resolve_island_body(&second, &name, occurrence).as_deref(),
            Some(op["html"].as_str().unwrap_or_default())
        );
    }

    #[test]
    fn island_op_occurrence_resolves_to_second_instance() {
        // Effect check for repeated components: occurrence 2 must resolve to
        // the SECOND island's range, not the first.
        let path = "/__hmr_test_island_effect_occurrence";
        let first = format!(
            "{}{}",
            island("Counter", "load", "{}", "<i data-deka-id=\"a1\">0</i>"),
            island("Counter", "load", "{}", "<i data-deka-id=\"b1\">10</i>")
        );
        let _ = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &first);
        let second = format!(
            "{}{}",
            island("Counter", "load", "{}", "<i data-deka-id=\"a1\">0</i>"),
            island("Counter", "load", "{}", "<i data-deka-id=\"b2\">11</i>")
        );
        let payload = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &second);
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        let op = &json["ops"][0];
        assert_eq!(op["island"], "Counter");
        assert_eq!(op["occurrence"], 2);
        let name = op["island"].as_str().unwrap_or_default().to_string();
        let occurrence = op["occurrence"].as_u64().unwrap_or(1) as usize;
        assert_eq!(
            resolve_island_body(&first, &name, occurrence).as_deref(),
            Some("<i data-deka-id=\"b1\">10</i>")
        );
        assert_eq!(
            resolve_island_body(&second, &name, occurrence).as_deref(),
            Some(op["html"].as_str().unwrap_or_default())
        );
    }

    #[test]
    fn unterminated_island_marker_falls_back_to_container_patch() {
        let path = "/__hmr_test_island_unterminated";
        let first = format!(
            "{}<p>tail</p>",
            island("Widget", "load", "{}", "<div data-deka-id=\"n1\">A</div>")
        );
        let _ = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", &first);
        // No end marker: the island cannot be bounded, so no island op may be
        // emitted for it.
        let second = "<!--deka-island start:V2lkZ2V0 directive:bG9hZA== props:e30=--><div data-deka-id=\"n2\">B</div><p>tail</p>";
        let payload = build_patch_from_snapshot(path, &["main.phpx".to_string()], "#app", second);
        let json = parse(&payload);
        assert_eq!(json["ops"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["ops"][0]["selector"], "#app");
    }

    #[test]
    fn granular_patch_payload_is_smaller_than_full_replace() {
        let full_path = "/__hmr_test_size_full";
        let full_payload = build_patch_from_snapshot(
            full_path,
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Alpha</div><div data-deka-id=\"b\">Bravo</div>",
        );

        let patch_path = "/__hmr_test_size_patch";
        let _ = build_patch_from_snapshot(
            patch_path,
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Alpha</div><div data-deka-id=\"b\">Bravo</div>",
        );
        let patch_payload = build_patch_from_snapshot(
            patch_path,
            &["main.phpx".to_string()],
            "#app",
            "<div data-deka-id=\"a\">Alpha 2</div><div data-deka-id=\"b\">Bravo</div>",
        );
        assert!(patch_payload.len() < full_payload.len());
    }
}
