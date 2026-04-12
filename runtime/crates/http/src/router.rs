use std::sync::Arc;

use axum::extract::ws::WebSocketUpgrade;
use axum::http::header::CONTENT_LENGTH;
use axum::{
    Router,
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use base64::Engine;

use crate::analytics::track_pageview;
use crate::utility_css::inject_utility_css;
use crate::websocket::{handle_hmr_websocket, handle_websocket, set_hmr_runtime_state};
use engine::{RuntimeState, execute_request_parts};

use crate::debug::http_debug_enabled;

pub fn app_router(state: Arc<RuntimeState>) -> Router {
    set_hmr_runtime_state(Arc::clone(&state));
    Router::new().fallback(handle_request).with_state(state)
}

async fn handle_request(
    State(state): State<Arc<RuntimeState>>,
    ws: Option<WebSocketUpgrade>,
    request: Request,
) -> impl IntoResponse {
    let method = request.method().as_str().to_string();
    let uri = request.uri().to_string();
    let hmr_path = request.uri().path() == "/_deka/hmr";
    if hmr_path && dev_mode_enabled() {
        if let Some(ws) = ws {
            let state_for_hmr = Arc::clone(&state);
            return ws
                .on_upgrade(move |socket| handle_hmr_websocket(socket, state_for_hmr))
                .into_response();
        }
        return Response::builder()
            .status(426)
            .body(axum::body::Body::from("WebSocket upgrade required"))
            .unwrap();
    }
    if http_debug_enabled() {
        tracing::info!("[http] request {} {}", method, uri);
    }
    let (headers, body) = if state.perf_mode {
        (Vec::new(), None)
    } else {
        let mut headers = Vec::with_capacity(request.headers().len());
        for (key, value) in request.headers().iter() {
            headers.push((
                key.as_str().to_string(),
                value.to_str().unwrap_or("").to_string(),
            ));
        }

        let content_len = request
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(usize::MAX);

        let body = if content_len == 0 {
            None
        } else {
            match axum::body::to_bytes(request.into_body(), usize::MAX).await {
                Ok(bytes) => {
                    if bytes.is_empty() {
                        None
                    } else {
                        Some(String::from_utf8_lossy(&bytes).to_string())
                    }
                }
                Err(_) => None,
            }
        };
        (headers, body)
    };

    // Keep a lightweight clone of the request headers for the pageview
    // tracker — it needs them to resolve the shop_id on the worker thread.
    // In perf mode `headers` is empty so the clone is effectively free.
    let request_headers_for_analytics = headers.clone();

    match execute_request_parts(
        Arc::clone(&state),
        format!("http://localhost{}", uri),
        method,
        headers,
        body,
    )
    .await
    {
        Ok(mut response_envelope) => {
            if http_debug_enabled() {
                tracing::info!("[http] response {} {}", response_envelope.status, uri);
            }
            // Fire-and-forget pageview tracking. Filters to 2xx + text/html
            // inside `track_pageview`, resolves shop_id on a dedicated
            // worker thread, writes to Redis out-of-band. Never blocks the
            // request path and never fails it.
            track_pageview(
                &request_headers_for_analytics,
                response_envelope.status,
                &response_envelope.headers,
            );
            if let Some(upgrade) = response_envelope.upgrade {
                if let Some(ws) = ws {
                    return ws
                        .on_upgrade(move |socket| handle_websocket(socket, state, Some(upgrade)))
                        .into_response();
                }
                return Response::builder()
                    .status(426)
                    .body(axum::body::Body::from("WebSocket upgrade required"))
                    .unwrap();
            }

            let mut response = Response::builder().status(response_envelope.status);
            let is_html = is_html_response(&response_envelope.headers);
            let inject_dev_hmr = dev_mode_enabled()
                && is_html
                && response_envelope.body_base64.is_none()
                && !response_envelope.body.is_empty();

            for (key, value) in response_envelope.headers {
                if key.eq_ignore_ascii_case("set-cookie") && value.contains('\n') {
                    for part in value.split('\n').filter(|part| !part.is_empty()) {
                        response = response.header(&key, part);
                    }
                    continue;
                }
                response = response.header(&key, value);
            }

            if let Some(body_base64) = response_envelope.body_base64 {
                let bytes: Vec<u8> = match base64::engine::general_purpose::STANDARD
                    .decode(body_base64.as_bytes())
                {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        return Response::builder()
                            .status(500)
                            .body(axum::body::Body::from(format!(
                                "Failed to decode body: {}",
                                err
                            )))
                            .unwrap();
                    }
                };
                response.body(axum::body::Body::from(bytes)).unwrap()
            } else {
                if inject_dev_hmr {
                    response_envelope.body = inject_hmr_client(&response_envelope.body);
                }
                if is_html {
                    response_envelope.body = inject_utility_css(&response_envelope.body);
                }
                response
                    .body(axum::body::Body::from(response_envelope.body))
                    .unwrap()
            }
        }
        Err(err) => {
            tracing::error!("Handler execution failed: {}", err);
            Response::builder()
                .status(500)
                .body(axum::body::Body::from(format!(
                    "Handler execution failed: {}",
                    err
                )))
                .unwrap()
        }
    }
}

fn dev_mode_enabled() -> bool {
    std::env::var("DEKA_DEV")
        .map(|value| is_truthy(&value))
        .unwrap_or(false)
}

fn is_truthy(value: &str) -> bool {
    matches!(value, "1" | "true" | "yes" | "on")
}

fn is_html_response(headers: &std::collections::HashMap<String, String>) -> bool {
    for (key, value) in headers {
        if key.eq_ignore_ascii_case("content-type")
            && value.to_ascii_lowercase().contains("text/html")
        {
            return true;
        }
    }
    false
}

fn inject_hmr_client(html: &str) -> String {
    const MARKER: &str = "__deka_hmr_client";
    if html.contains(MARKER) {
        return html.to_string();
    }
    const SCRIPT: &str = r#"<script id="__deka_hmr_client">(function(){try{var p=location.protocol==='https:'?'wss':'ws';var ws=new WebSocket(p+'://'+location.host+'/_deka/hmr');function c(s){return document.querySelector(s||'#app');}function e(v){return String(v||'').replace(/\\/g,'\\\\').replace(/"/g,'\\"');}function sf(){var a=document.activeElement;if(!a||!a.closest||!a.closest('#app')){return null;}return{id:a.id||'',name:a.getAttribute('name')||'',deka:a.getAttribute('data-deka-id')||'',start:typeof a.selectionStart==='number'?a.selectionStart:null,end:typeof a.selectionEnd==='number'?a.selectionEnd:null};}function rf(state){if(!state){return;}var el=null;if(state.id){el=document.getElementById(state.id);}if(!el&&state.deka){el=document.querySelector('#app [data-deka-id="'+e(state.deka)+'"]');}if(!el&&state.name){el=document.querySelector('#app [name="'+e(state.name)+'"]');}if(!el||typeof el.focus!=='function'){return;}el.focus();if(state.start!==null&&state.end!==null&&typeof el.setSelectionRange==='function'){try{el.setSelectionRange(state.start,state.end);}catch(_){}}}function fv(){var root=c('#app');if(!root){return [];}var out=[];var fields=root.querySelectorAll('input,textarea,select');for(var i=0;i<fields.length;i++){var f=fields[i];var id=f.id||'';var name=f.getAttribute('name')||'';var deka=f.getAttribute('data-deka-id')||'';if(!id&&!name&&!deka){continue;}var type=(f.getAttribute('type')||'').toLowerCase();var entry={id:id,name:name,deka:deka,type:type};if(type==='checkbox'||type==='radio'){entry.checked=!!f.checked;}else if(f.tagName==='SELECT'){entry.value=f.value;}else{entry.value=f.value;}out.push(entry);}return out;}function fr(list){if(!Array.isArray(list)||list.length===0){return;}for(var i=0;i<list.length;i++){var s=list[i]||{};var el=null;if(s.id){el=document.getElementById(s.id);}if(!el&&s.deka){el=document.querySelector('#app [data-deka-id="'+e(s.deka)+'"]');}if(!el&&s.name){el=document.querySelector('#app [name="'+e(s.name)+'"]');}if(!el){continue;}if((s.type==='checkbox'||s.type==='radio')&&typeof s.checked==='boolean'){el.checked=s.checked;continue;}if(typeof s.value!=='undefined'){el.value=s.value;}}}function nh(h){return String(h||'').replace(/shadowrootmode=/gi,'data-shadowrootmode=');}function ap(selector,html){var n=c(selector||'#app');if(!n){location.reload();return;}var y=window.scrollY||window.pageYOffset||0;var f=sf();var v=fv();var h=nh(html);if(n.matches&&n.matches('[data-deka-island-id],deka-island')&&n.shadowRoot){n.shadowRoot.innerHTML=h;}else{n.innerHTML=h;}if(typeof window.__dekaMountDeclarativeShadows==='function'){window.__dekaMountDeclarativeShadows(n);}if(typeof window.__dekaHydrateIslands==='function'){window.__dekaHydrateIslands(n);}window.scrollTo(0,y);fr(v);rf(f);}function sr(){var u=location.pathname+location.search;fetch(u,{headers:{Accept:'text/x-phpx-fragment'},credentials:'same-origin'}).then(function(r){if(!r.ok){return null;}return r.json();}).then(function(p){if(p&&typeof p.html==='string'){ap('#app',p.html);if(typeof p.title==='string'&&p.title!==''){document.title=p.title;}if(typeof p.head==='string'&&p.head!==''){document.head.insertAdjacentHTML('beforeend',p.head);}return;}location.reload();}).catch(function(){location.reload();});}function sub(){try{ws.send(JSON.stringify({type:'subscribe',path:location.pathname+location.search}));}catch(_){}}function a(m){if(!m||!Array.isArray(m.ops)||m.ops.length===0){sr();return;}for(var i=0;i<m.ops.length;i++){var op=m.ops[i]||{};if(op.op==='set_html'){ap(op.selector||'#app',op.html||'');continue;}sr();return;}}ws.onopen=function(){sub();};ws.onmessage=function(ev){try{var m=JSON.parse(ev.data||'{}');if(m.type==='patch'){a(m);return;}if(m.type==='reload'){sr();return;}}catch(_){sr();}};window.addEventListener('popstate',function(){sub();});ws.onclose=function(){};}catch(_){}})();</script>"#;

    if let Some(idx) = html.rfind("</body>") {
        let mut out = String::with_capacity(html.len() + SCRIPT.len());
        out.push_str(&html[..idx]);
        out.push_str(SCRIPT);
        out.push_str(&html[idx..]);
        return out;
    }
    let mut out = String::with_capacity(html.len() + SCRIPT.len());
    out.push_str(html);
    out.push_str(SCRIPT);
    out
}

#[cfg(test)]
mod tests {
    use super::{inject_hmr_client, is_truthy};

    #[test]
    fn injects_before_body_close() {
        let html = "<html><body><h1>x</h1></body></html>";
        let out = inject_hmr_client(html);
        assert!(out.contains("__deka_hmr_client"));
        assert!(out.find("__deka_hmr_client").unwrap() < out.find("</body>").unwrap());
    }

    #[test]
    fn avoids_duplicate_injection() {
        let html = "<html><body><script id=\"__deka_hmr_client\"></script></body></html>";
        let out = inject_hmr_client(html);
        assert_eq!(out.matches("__deka_hmr_client").count(), 1);
    }

    #[test]
    fn hmr_client_includes_state_preservation_hooks() {
        let html = "<html><body><div id=\"app\"><input id=\"x\" value=\"1\" /></div></body></html>";
        let out = inject_hmr_client(html);
        assert!(out.contains("selectionStart"));
        assert!(out.contains("window.scrollTo"));
        assert!(out.contains("__dekaHydrateIslands"));
        assert!(out.contains("querySelectorAll('input,textarea,select')"));
        assert!(out.contains("data-deka-id"));
        assert!(out.contains("setSelectionRange"));
    }

    #[test]
    fn truthy_parser_matches_expected_values() {
        assert!(is_truthy("1"));
        assert!(is_truthy("true"));
        assert!(is_truthy("yes"));
        assert!(is_truthy("on"));
        assert!(!is_truthy("false"));
        assert!(!is_truthy("0"));
    }
}
