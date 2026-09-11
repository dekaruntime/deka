use hyper_util::rt::TokioExecutor;
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use std::path::Path;
use std::sync::Arc;

use crate::app_router;
use crate::config::HttpConfig;
use engine::RuntimeState;

pub async fn serve_unix(
    state: Arc<RuntimeState>,
    socket_path: &str,
    config: HttpConfig,
) -> Result<(), String> {
    let app = app_router(state, config);
    let listener = bind_unix_listener(socket_path)?;
    loop {
        let (stream, _) = listener.accept().await.map_err(|err| err.to_string())?;
        let app = app.clone();
        let service = TowerToHyperService::new(app);
        tokio::spawn(async move {
            let builder = Builder::new(TokioExecutor::new());
            let io = hyper_util::rt::TokioIo::new(stream);
            if let Err(err) = builder.serve_connection_with_upgrades(io, service).await {
                tracing::warn!("unix connection failed: {}", err);
            }
        });
    }
}

fn bind_unix_listener(socket_path: &str) -> Result<tokio::net::UnixListener, String> {
    #[cfg(not(unix))]
    {
        let _ = socket_path;
        return Err("Unix sockets are not supported on this platform".to_string());
    }

    #[cfg(unix)]
    {
        if socket_path.starts_with('\0') {
            return bind_abstract_unix(socket_path);
        }

        let path = Path::new(socket_path);
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                return Err(format!(
                    "Unix socket directory does not exist: {}",
                    parent.display()
                ));
            }
        }

        if path.exists() {
            std::fs::remove_file(path).map_err(|err| {
                format!(
                    "Failed to remove existing unix socket {}: {}",
                    path.display(),
                    err
                )
            })?;
        }

        tokio::net::UnixListener::bind(path)
            .map_err(|err| format!("Failed to bind unix socket {}: {}", path.display(), err))
    }
}

#[cfg(unix)]
fn bind_abstract_unix(socket_path: &str) -> Result<tokio::net::UnixListener, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = socket_path;
        return Err("Abstract unix sockets are only supported on Linux".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        use socket2::{Domain, Socket, Type};
        use std::os::unix::net::UnixListener as StdUnixListener;

        let addr = socket2::SockAddr::unix(socket_path)
            .map_err(|err| format!("Failed to create abstract unix addr: {}", err))?;

        let socket = Socket::new(Domain::UNIX, Type::STREAM, None)
            .map_err(|err| format!("Failed to create unix socket: {}", err))?;
        socket
            .bind(&addr)
            .map_err(|err| format!("Failed to bind abstract unix socket: {}", err))?;
        socket
            .listen(1024)
            .map_err(|err| format!("Failed to listen on abstract unix socket: {}", err))?;
        socket
            .set_nonblocking(true)
            .map_err(|err| format!("Failed to set nonblocking: {}", err))?;

        let std_listener: StdUnixListener = socket.into();
        tokio::net::UnixListener::from_std(std_listener)
            .map_err(|err| format!("Failed to create tokio unix listener: {}", err))
    }
}
