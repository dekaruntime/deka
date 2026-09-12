pub mod dns;
pub mod tcp;
pub mod udp;
pub mod ws;

use std::sync::Arc;

pub use engine::RuntimeState;

pub struct HttpOptions {
    pub port: u16,
    pub listeners: usize,
    pub perf_mode: bool,
    /// Caller-supplied HTTP configuration (deka#801) — replaces the
    /// `DEKA_RATE_LIMIT_*` / `DEKA_HTTP_DEBUG` environment reads.
    pub http: deka_http::HttpConfig,
}

pub struct UnixOptions {
    pub path: String,
    pub http: deka_http::HttpConfig,
}

pub struct WsOptions {
    pub port: u16,
}

pub struct TcpOptions {
    pub addr: String,
}

pub struct UdpOptions {
    pub addr: String,
}

pub struct DnsOptions {
    pub addr: String,
}

pub enum ListenConfig {
    Http(HttpOptions),
    Unix(UnixOptions),
    Ws(WsOptions),
    Tcp(TcpOptions),
    Udp(UdpOptions),
    Dns(DnsOptions),
}

pub fn notify_hmr_changed(paths: &[String]) {
    deka_http::websocket::broadcast_hmr_changed(paths);
}

pub async fn serve(state: Arc<RuntimeState>, target: ListenConfig) -> Result<(), String> {
    match target {
        ListenConfig::Http(options) => {
            deka_http::serve_http(
                state,
                options.port,
                options.listeners,
                options.perf_mode,
                options.http,
            )
            .await
        }
        ListenConfig::Unix(options) => {
            deka_http::unix::serve_unix(state, &options.path, options.http).await
        }
        ListenConfig::Ws(options) => ws::serve_ws(state, options).await,
        ListenConfig::Tcp(options) => tcp::serve_tcp(state, options).await,
        ListenConfig::Udp(options) => udp::serve_udp(state, options).await,
        ListenConfig::Dns(options) => dns::serve_dns(state, options).await,
    }
}
