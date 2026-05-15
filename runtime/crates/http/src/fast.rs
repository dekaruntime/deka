use std::sync::Arc;

use base64::Engine;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HyperBuilder;

use engine::{RuntimeState, execute_request_parts};

use crate::debug::http_debug_enabled;

pub async fn serve_http_fast(listener: tokio::net::TcpListener, state: Arc<RuntimeState>) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!("HTTP accept failed: {}", err);
                continue;
            }
        };
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| handle_request_fast(Arc::clone(&state), req));
            let builder = HyperBuilder::new(TokioExecutor::new());
            if let Err(err) = builder.serve_connection(io, service).await {
                tracing::warn!("HTTP connection failed: {}", err);
            }
        });
    }
}

async fn handle_request_fast(
    state: Arc<RuntimeState>,
    request: hyper::Request<Incoming>,
) -> Result<hyper::Response<Full<Bytes>>, hyper::Error> {
    let method = request.method().as_str().to_string();
    let uri = request.uri().to_string();
    if http_debug_enabled() {
        tracing::info!("[http-fast] request {} {}", method, uri);
    }
    let _ = request.into_body();

    let response = match execute_request_parts(
        Arc::clone(&state),
        format!("http://localhost{}", uri),
        method,
        Vec::new(),
        None,
    )
    .await
    {
        Ok(response_envelope) => response_envelope,
        Err(err) => {
            tracing::error!("Handler execution failed: {}", err);
            let response = hyper::Response::builder().status(500);
            let body = Full::new(Bytes::from(format!("Handler execution failed: {}", err)));
            return Ok(response.body(body).unwrap());
        }
    };
    if http_debug_enabled() {
        tracing::info!("[http-fast] response {} {}", response.status, uri);
    }

    let mut builder = hyper::Response::builder().status(response.status);
    for (key, value) in response.headers {
        if key.eq_ignore_ascii_case("set-cookie") && value.contains('\n') {
            for part in value.split('\n').filter(|part| !part.is_empty()) {
                builder = builder.header(&key, part);
            }
            continue;
        }
        builder = builder.header(&key, value);
    }

    let body = if let Some(body_base64) = response.body_base64 {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(body_base64.as_bytes())
            .unwrap_or_default();
        Full::new(Bytes::from(bytes))
    } else {
        Full::new(Bytes::from(response.body))
    };

    Ok(builder.body(body).unwrap())
}
