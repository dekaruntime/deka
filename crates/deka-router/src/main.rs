//! Router-only Deka platform binary (stubbed).
//!
//! The original shard-routing proxy was archived with the `deka-shard` crate.
//! This binary now starts a minimal HTTP server that reports readiness via
//! `/healthz` and returns 501 for all other routes.

use std::net::TcpListener;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};

const DEFAULT_LISTEN_PORT: u16 = 8531;

#[tokio::main]
async fn main() {
    let port = parse_port();
    let bind_addr = std::env::var("DEKA_ROUTER_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let listener = match TcpListener::bind(format!("{bind_addr}:{port}")) {
        Ok(listener) => listener,
        Err(err) => {
            log_error("router", &format!("failed to bind {bind_addr}:{port}: {err}"));
            std::process::exit(1);
        }
    };
    listener.set_nonblocking(true).ok();
    log("listen", &format!("http://{bind_addr}:{port}"));

    let app = Router::new()
        .route("/healthz", axum::routing::get(healthz))
        .fallback(not_implemented);

    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn healthz() -> impl IntoResponse {
    Response::builder()
        .status(200)
        .body(Body::from("ok"))
        .unwrap()
}

async fn not_implemented(_request: Request) -> impl IntoResponse {
    Response::builder()
        .status(501)
        .body(Body::from("Not Implemented: shard routing has been removed"))
        .unwrap()
}

fn parse_port() -> u16 {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--port" {
            if let Some(value) = args.next()
                && let Ok(port) = value.parse()
            {
                return port;
            }
        } else if let Some(value) = arg.strip_prefix("--port=")
            && let Ok(port) = value.parse()
        {
            return port;
        }
    }
    env_u16("PORT").unwrap_or(DEFAULT_LISTEN_PORT)
}

fn env_u16(key: &str) -> Option<u16> {
    std::env::var(key).ok()?.parse().ok()
}

fn log(category: &str, message: &str) {
    eprintln!("[{category}] {message}");
}

fn log_error(category: &str, message: &str) {
    eprintln!("[{category}] ERROR: {message}");
}
