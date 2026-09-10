use super::bridge_metrics::record_bridge_proto_metric;
use super::security::{enforce_net_with, security_policy_from_context};
use super::*;
use deno_core::OpState;
use rustls::{Certificate, PrivateKey, ServerName};
use std::cell::RefCell;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::Arc;

pub(super) enum NetConn {
    Tcp(TcpStream),
    TlsClient(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
    TlsServer(rustls::StreamOwned<rustls::ServerConnection, TcpStream>),
}

pub(super) enum NetListener {
    Tcp(TcpListener),
    Tls(TcpListener, Arc<rustls::ServerConfig>),
}

pub(super) struct NetHandle {
    conn: NetConn,
    // Derived from the validated connect request. Handle-only bridge actions
    // must use this immutable target for their capability check.
    target: String,
}

pub(super) struct NetListenerHandle {
    listener: NetListener,
    // Capability target derived from the bind address. Accept actions reuse
    // this target so the listener grant covers inbound connections.
    target: String,
}

/// Per-isolate socket ownership state.  This must live in Deno's `OpState`,
/// never in a process-global static: a numeric handle is only meaningful in
/// the isolate that created it.
///
/// Connections and listeners live in separate maps with separate counters to
/// avoid handle collisions and keep the lookup logic simple.
pub(super) struct NetState {
    next_conn_handle: u64,
    next_listener_handle: u64,
    handles: HashMap<u64, NetHandle>,
    listeners: HashMap<u64, NetListenerHandle>,
    /// Fixed policy for this isolate's net bridge. `None` (the
    /// production default) resolves the policy from the per-execution
    /// security context the dispatch path installed (deka#801); a missing
    /// context is an error, never an env read or a default policy. Tests
    /// seed a policy via `with_policy` so isolates stop depending on the
    /// executing thread's context (deka#537).
    policy: Option<SecurityPolicy>,
}

impl NetState {
    pub(super) fn new() -> Self {
        Self {
            next_conn_handle: 1,
            next_listener_handle: 1,
            handles: HashMap::new(),
            listeners: HashMap::new(),
            policy: None,
        }
    }

    pub(super) fn with_policy(policy: SecurityPolicy) -> Self {
        Self {
            policy: Some(policy),
            ..Self::new()
        }
    }
}

fn capability_target(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Convert a JSON bridge value into raw bytes. Arrays are treated as byte
/// sequences (mirroring the fs bridge); strings are UTF-8 encoded for
/// backwards compatibility with callers that have not migrated to `bytes`.
fn json_value_to_bytes(value: Option<&serde_json::Value>) -> Vec<u8> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Some(arr) = value.as_array() {
        arr.iter()
            .map(|v| v.as_u64().unwrap_or(0).min(255) as u8)
            .collect()
    } else if let Some(s) = value.as_str() {
        s.as_bytes().to_vec()
    } else {
        Vec::new()
    }
}

fn bytes_to_json_array(bytes: &[u8]) -> serde_json::Value {
    serde_json::Value::Array(
        bytes
            .iter()
            .map(|b| serde_json::Value::Number((*b as u64).into()))
            .collect(),
    )
}

fn handle_target(state: &NetState, handle: u64) -> Result<String, deno_core::error::CoreError> {
    state
        .handles
        .get(&handle)
        .map(|handle| handle.target.clone())
        .ok_or_else(|| core_err(format!("net: unknown handle {handle}")))
}

fn listener_target(state: &NetState, handle: u64) -> Result<String, deno_core::error::CoreError> {
    state
        .listeners
        .get(&handle)
        .map(|handle| handle.target.clone())
        .ok_or_else(|| core_err(format!("net: unknown listener handle {handle}")))
}

fn client_tls_config(
    ca_cert_pem: Option<&[u8]>,
) -> Result<Arc<rustls::ClientConfig>, rustls::Error> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| {
        rustls::OwnedTrustAnchor::from_subject_spki_name_constraints(
            ta.subject,
            ta.spki,
            ta.name_constraints,
        )
    }));

    if let Some(pem) = ca_cert_pem {
        let certs = rustls_pemfile::certs(&mut &pem[..])
            .map_err(|e| rustls::Error::General(format!("invalid ca_cert PEM: {e}")))?;
        for cert in certs {
            root_store
                .add(&rustls::Certificate(cert))
                .map_err(|e| rustls::Error::General(format!("invalid ca_cert: {e}")))?;
        }
    }

    Ok(Arc::new(
        rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth(),
    ))
}

fn server_tls_config(
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<Arc<rustls::ServerConfig>, rustls::Error> {
    let cert_chain: Vec<Certificate> = rustls_pemfile::certs(&mut &cert_pem[..])
        .map_err(|e| rustls::Error::General(format!("invalid certificate PEM: {e}")))?
        .into_iter()
        .map(Certificate)
        .collect();

    let mut keys: Vec<Vec<u8>> = rustls_pemfile::pkcs8_private_keys(&mut &key_pem[..])
        .map_err(|e| rustls::Error::General(format!("invalid PKCS8 key: {e}")))?
        .into_iter()
        .collect();
    if keys.is_empty() {
        keys = rustls_pemfile::rsa_private_keys(&mut &key_pem[..])
            .map_err(|e| rustls::Error::General(format!("invalid RSA key: {e}")))?
            .into_iter()
            .collect();
    }
    let key = keys
        .into_iter()
        .next()
        .map(PrivateKey)
        .ok_or_else(|| rustls::Error::General("no private key found".into()))?;

    Ok(Arc::new(
        rustls::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)?,
    ))
}

fn parse_server_name(name: &str) -> Result<ServerName, deno_core::error::CoreError> {
    ServerName::try_from(name).map_err(|e| core_err(format!("invalid server_name '{name}': {e}")))
}

pub(super) fn net_call_impl(
    state: &mut NetState,
    action: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let err = |msg: String| {
        deno_core::error::CoreError::from(std::io::Error::new(std::io::ErrorKind::Other, msg))
    };

    let args_obj = args.as_object().cloned().unwrap_or_default();
    match action.as_str() {
        "connect" => {
            let host = args_obj
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .trim_matches('\0')
                .to_string();
            let port = match tcp_connect_port(&args) {
                Ok(port) => port,
                Err(error) => {
                    return Ok(serde_json::json!({ "ok": false, "error": error.to_string() }));
                }
            };
            let timeout_ms = args_obj
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(5000);
            let capability_target = capability_target(&host, port);
            let addr = capability_target.clone();
            let mut addrs = addr
                .to_socket_addrs()
                .map_err(|e| err(format!("connect: resolve failed: {}", e)))?;
            let target = addrs
                .next()
                .ok_or_else(|| err("connect: no resolved address".to_string()))?;
            let stream = TcpStream::connect_timeout(&target, Duration::from_millis(timeout_ms))
                .map_err(|e| err(format!("connect: {}", e)))?;
            let handle = state.next_conn_handle;
            state.next_conn_handle += 1;
            state.handles.insert(
                handle,
                NetHandle {
                    conn: NetConn::Tcp(stream),
                    target: capability_target,
                },
            );
            Ok(serde_json::json!({ "ok": true, "handle": handle }))
        }
        "connect_tls" => {
            let host = args_obj
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .trim_matches('\0')
                .to_string();
            let port = match tcp_connect_port(&args) {
                Ok(port) => port,
                Err(error) => {
                    return Ok(serde_json::json!({ "ok": false, "error": error.to_string() }));
                }
            };
            let server_name = args_obj
                .get("server_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if server_name.is_empty() {
                return Ok(
                    serde_json::json!({ "ok": false, "error": "connect_tls: missing server_name" }),
                );
            }
            let timeout_ms = args_obj
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(5000);
            let capability_target = capability_target(&host, port);
            let addr = capability_target.clone();
            let mut addrs = addr
                .to_socket_addrs()
                .map_err(|e| err(format!("connect_tls: resolve failed: {}", e)))?;
            let target = addrs
                .next()
                .ok_or_else(|| err("connect_tls: no resolved address".to_string()))?;
            let tcp = TcpStream::connect_timeout(&target, Duration::from_millis(timeout_ms))
                .map_err(|e| err(format!("connect_tls: {}", e)))?;
            let server_name = parse_server_name(&server_name)?;
            let ca_cert = json_value_to_bytes(args_obj.get("ca_cert"));
            let ca_cert_pem = if ca_cert.is_empty() {
                None
            } else {
                Some(ca_cert.as_slice())
            };
            let client_config = client_tls_config(ca_cert_pem)
                .map_err(|e| err(format!("connect_tls: tls config failed: {e}")))?;
            let conn = rustls::ClientConnection::new(client_config, server_name)
                .map_err(|e| err(format!("connect_tls: tls init failed: {e}")))?;
            let mut stream = rustls::StreamOwned::new(conn, tcp);
            stream
                .conn
                .complete_io(&mut stream.sock)
                .map(|_| ())
                .map_err(|e| err(format!("connect_tls: handshake failed: {e}")))?;
            let handle = state.next_conn_handle;
            state.next_conn_handle += 1;
            state.handles.insert(
                handle,
                NetHandle {
                    conn: NetConn::TlsClient(stream),
                    target: capability_target,
                },
            );
            Ok(serde_json::json!({ "ok": true, "handle": handle }))
        }
        "listen" => {
            let host = args_obj
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .trim_matches('\0')
                .to_string();
            let port = match tcp_bind_port(&args) {
                Ok(port) => port,
                Err(error) => {
                    return Ok(serde_json::json!({ "ok": false, "error": error.to_string() }));
                }
            };
            let _backlog = args_obj
                .get("backlog")
                .and_then(|v| v.as_u64())
                .unwrap_or(128) as u32;
            let capability_target = capability_target(&host, port);
            let addr = capability_target.clone();
            let listener = TcpListener::bind(&addr).map_err(|e| err(format!("listen: {}", e)))?;
            listener
                .set_nonblocking(false)
                .map_err(|e| err(format!("listen: set blocking failed: {e}")))?;
            let handle = state.next_listener_handle;
            state.next_listener_handle += 1;
            state.listeners.insert(
                handle,
                NetListenerHandle {
                    listener: NetListener::Tcp(listener),
                    target: capability_target,
                },
            );
            Ok(serde_json::json!({ "ok": true, "handle": handle }))
        }
        "listen_tls" => {
            let host = args_obj
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .trim_matches('\0')
                .to_string();
            let port = match tcp_bind_port(&args) {
                Ok(port) => port,
                Err(error) => {
                    return Ok(serde_json::json!({ "ok": false, "error": error.to_string() }));
                }
            };
            let backlog = args_obj
                .get("backlog")
                .and_then(|v| v.as_u64())
                .unwrap_or(128) as u32;
            let cert = json_value_to_bytes(args_obj.get("cert"));
            let key = json_value_to_bytes(args_obj.get("key"));
            let capability_target = capability_target(&host, port);
            let addr = capability_target.clone();
            let listener = TcpListener::bind(&addr)
                .map_err(|e| err(format!("listen_tls: bind failed: {e}")))?;
            listener
                .set_nonblocking(false)
                .map_err(|e| err(format!("listen_tls: set blocking failed: {e}")))?;
            let config = server_tls_config(&cert, &key)
                .map_err(|e| err(format!("listen_tls: tls config failed: {e}")))?;
            // `backlog` is accepted for API parity but Rust's std::net::TcpListener
            // does not expose a way to set the listen backlog after binding.
            let _ = backlog;
            let handle = state.next_listener_handle;
            state.next_listener_handle += 1;
            state.listeners.insert(
                handle,
                NetListenerHandle {
                    listener: NetListener::Tls(listener, config),
                    target: capability_target,
                },
            );
            Ok(serde_json::json!({ "ok": true, "handle": handle }))
        }
        "accept" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("accept: missing handle".to_string()))?;
            let Some(listener_handle) = state.listeners.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("accept: unknown listener handle {}", handle) }),
                );
            };
            let target = listener_handle.target.clone();
            match &mut listener_handle.listener {
                NetListener::Tcp(listener) => {
                    let (stream, peer_addr) = listener
                        .accept()
                        .map_err(|e| err(format!("accept: {}", e)))?;
                    let conn_handle = state.next_conn_handle;
                    state.next_conn_handle += 1;
                    state.handles.insert(
                        conn_handle,
                        NetHandle {
                            conn: NetConn::Tcp(stream),
                            target,
                        },
                    );
                    Ok(serde_json::json!({
                        "ok": true,
                        "handle": conn_handle,
                        "peer_addr": peer_addr.to_string(),
                    }))
                }
                NetListener::Tls(listener, config) => {
                    let (tcp, peer_addr) = listener
                        .accept()
                        .map_err(|e| err(format!("accept: {}", e)))?;
                    let conn = rustls::ServerConnection::new(config.clone())
                        .map_err(|e| err(format!("accept: tls init failed: {e}")))?;
                    let mut stream = rustls::StreamOwned::new(conn, tcp);
                    stream
                        .conn
                        .complete_io(&mut stream.sock)
                        .map(|_| ())
                        .map_err(|e| err(format!("accept: handshake failed: {e}")))?;
                    let conn_handle = state.next_conn_handle;
                    state.next_conn_handle += 1;
                    state.handles.insert(
                        conn_handle,
                        NetHandle {
                            conn: NetConn::TlsServer(stream),
                            target,
                        },
                    );
                    Ok(serde_json::json!({
                        "ok": true,
                        "handle": conn_handle,
                        "peer_addr": peer_addr.to_string(),
                    }))
                }
            }
        }
        "set_deadline" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("set_deadline: missing handle".to_string()))?;
            let millis = args_obj.get("millis").and_then(|v| v.as_u64()).unwrap_or(0);
            let timeout = if millis == 0 {
                None
            } else {
                Some(Duration::from_millis(millis))
            };
            let Some(conn) = state.handles.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("set_deadline: unknown handle {}", handle) }),
                );
            };
            let result = match &mut conn.conn {
                NetConn::Tcp(stream) => stream
                    .set_read_timeout(timeout)
                    .and_then(|_| stream.set_write_timeout(timeout)),
                NetConn::TlsClient(stream) => stream
                    .get_ref()
                    .set_read_timeout(timeout)
                    .and_then(|_| stream.get_ref().set_write_timeout(timeout)),
                NetConn::TlsServer(stream) => stream
                    .get_ref()
                    .set_read_timeout(timeout)
                    .and_then(|_| stream.get_ref().set_write_timeout(timeout)),
            };
            match result {
                Ok(()) => Ok(serde_json::json!({ "ok": true })),
                Err(e) => {
                    Ok(serde_json::json!({ "ok": false, "error": format!("set_deadline: {}", e) }))
                }
            }
        }
        "read" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("read: missing handle".to_string()))?;
            let max_bytes = args_obj
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(4096) as usize;
            let mut buf = vec![0_u8; max_bytes.max(1)];
            let Some(conn) = state.handles.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("read: unknown handle {}", handle) }),
                );
            };
            let n = match &mut conn.conn {
                NetConn::Tcp(stream) => stream.read(&mut buf),
                NetConn::TlsClient(stream) => stream.read(&mut buf),
                NetConn::TlsServer(stream) => stream.read(&mut buf),
            };
            match n {
                Ok(n) => {
                    buf.truncate(n);
                    Ok(serde_json::json!({ "ok": true, "data": buf, "eof": n == 0 }))
                }
                Err(e) => Ok(serde_json::json!({ "ok": false, "error": format!("read: {}", e) })),
            }
        }
        "write" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("write: missing handle".to_string()))?;
            let data = json_value_to_bytes(args_obj.get("data"));
            let Some(conn) = state.handles.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("write: unknown handle {}", handle) }),
                );
            };
            let result = match &mut conn.conn {
                NetConn::Tcp(stream) => stream.write_all(&data),
                NetConn::TlsClient(stream) => stream.write_all(&data),
                NetConn::TlsServer(stream) => stream.write_all(&data),
            };
            match result {
                Ok(()) => Ok(serde_json::json!({ "ok": true, "written": data.len() })),
                Err(e) => Ok(serde_json::json!({ "ok": false, "error": format!("write: {}", e) })),
            }
        }
        "read_until" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("read_until: missing handle".to_string()))?;
            let delimiter = json_value_to_bytes(args_obj.get("delimiter"));
            if delimiter.is_empty() {
                return Ok(
                    serde_json::json!({ "ok": false, "error": "read_until: empty delimiter" }),
                );
            }
            let max_bytes = args_obj
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(65536) as usize;
            let Some(conn) = state.handles.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("read_until: unknown handle {}", handle) }),
                );
            };
            let mut out = Vec::new();
            let mut byte = [0_u8; 1];
            while out.len() < max_bytes {
                let n = match &mut conn.conn {
                    NetConn::Tcp(stream) => stream.read(&mut byte),
                    NetConn::TlsClient(stream) => stream.read(&mut byte),
                    NetConn::TlsServer(stream) => stream.read(&mut byte),
                };
                match n {
                    Ok(0) => {
                        return Ok(serde_json::json!({
                            "ok": true,
                            "data": out,
                            "found": false,
                            "eof": true
                        }));
                    }
                    Ok(1) => {
                        out.push(byte[0]);
                        if out.ends_with(&delimiter) {
                            out.truncate(out.len() - delimiter.len());
                            return Ok(serde_json::json!({
                                "ok": true,
                                "data": out,
                                "found": true,
                                "eof": false
                            }));
                        }
                    }
                    Ok(_) => unreachable!("single-byte read returned more than one byte"),
                    Err(e) => {
                        return Ok(
                            serde_json::json!({ "ok": false, "error": format!("read_until: {}", e) }),
                        );
                    }
                }
            }
            Ok(serde_json::json!({
                "ok": true,
                "data": out,
                "found": false,
                "eof": false
            }))
        }
        "tls_upgrade" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("tls_upgrade: missing handle".to_string()))?;
            let server_name = args_obj
                .get("server_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if server_name.is_empty() {
                return Ok(
                    serde_json::json!({ "ok": false, "error": "tls_upgrade: missing server_name" }),
                );
            }
            let Some(conn) = state.handles.remove(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("tls_upgrade: unknown handle {}", handle) }),
                );
            };
            let target = conn.target;
            let tcp = match conn.conn {
                NetConn::Tcp(stream) => stream,
                NetConn::TlsClient(stream) => {
                    let new_handle = state.next_conn_handle;
                    state.next_conn_handle += 1;
                    state.handles.insert(
                        new_handle,
                        NetHandle {
                            conn: NetConn::TlsClient(stream),
                            target,
                        },
                    );
                    return Ok(
                        serde_json::json!({ "ok": true, "handle": new_handle, "reused": true }),
                    );
                }
                NetConn::TlsServer(stream) => {
                    let new_handle = state.next_conn_handle;
                    state.next_conn_handle += 1;
                    state.handles.insert(
                        new_handle,
                        NetHandle {
                            conn: NetConn::TlsServer(stream),
                            target,
                        },
                    );
                    return Ok(
                        serde_json::json!({ "ok": true, "handle": new_handle, "reused": true }),
                    );
                }
            };
            let server_name = parse_server_name(&server_name)?;
            let client_config = client_tls_config(None)
                .map_err(|e| err(format!("tls_upgrade: tls config failed: {e}")))?;
            let conn = rustls::ClientConnection::new(client_config, server_name)
                .map_err(|e| err(format!("tls_upgrade: tls init failed: {e}")))?;
            let mut stream = rustls::StreamOwned::new(conn, tcp);
            stream
                .conn
                .complete_io(&mut stream.sock)
                .map(|_| ())
                .map_err(|e| err(format!("connect_tls: handshake failed: {e}")))?;
            let new_handle = state.next_conn_handle;
            state.next_conn_handle += 1;
            state.handles.insert(
                new_handle,
                NetHandle {
                    conn: NetConn::TlsClient(stream),
                    target,
                },
            );
            Ok(serde_json::json!({ "ok": true, "handle": new_handle }))
        }
        "close" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("close: missing handle".to_string()))?;
            if state.handles.remove(&handle).is_none() {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("close: unknown handle {}", handle) }),
                );
            }
            Ok(serde_json::json!({ "ok": true }))
        }
        _ => Ok(serde_json::json!({
            "ok": false,
            "error": format!("unknown net action '{}'", action)
        })),
    }
}

#[derive(Clone, Copy)]
pub(super) enum NetProtoActionKind {
    Connect,
    SetDeadline,
    Read,
    Write,
    TlsUpgrade,
    Close,
    ConnectTls,
    Listen,
    ListenTls,
    Accept,
    ReadUntil,
}

pub(super) fn net_action_payload_to_proto_request(
    action: &str,
    payload: &serde_json::Value,
) -> Result<proto::bridge_v1::NetRequest, deno_core::error::CoreError> {
    use proto::bridge_v1::net_request::Action;
    let args = payload.as_object().cloned().unwrap_or_default();
    let action = match action {
        "connect" => Action::Connect(proto::bridge_v1::NetConnectRequest {
            host: args
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .to_string(),
            port: args.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            timeout_ms: args
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(5000),
        }),
        "connect_tls" => Action::ConnectTls(proto::bridge_v1::NetConnectTlsRequest {
            host: args
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .to_string(),
            port: args.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            server_name: args
                .get("server_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            timeout_ms: args
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(5000),
            ca_cert: json_value_to_bytes(args.get("ca_cert")),
        }),
        "listen" => Action::Listen(proto::bridge_v1::NetListenRequest {
            host: args
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .to_string(),
            port: args.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            backlog: args.get("backlog").and_then(|v| v.as_u64()).unwrap_or(128) as u32,
        }),
        "listen_tls" => Action::ListenTls(proto::bridge_v1::NetListenTlsRequest {
            host: args
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("127.0.0.1")
                .to_string(),
            port: args.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            backlog: args.get("backlog").and_then(|v| v.as_u64()).unwrap_or(128) as u32,
            cert: json_value_to_bytes(args.get("cert")),
            key: json_value_to_bytes(args.get("key")),
        }),
        "accept" => Action::Accept(proto::bridge_v1::NetAcceptRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        "set_deadline" => Action::SetDeadline(proto::bridge_v1::NetDeadlineRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            millis: args.get("millis").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        "read" => Action::Read(proto::bridge_v1::NetReadRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            max_bytes: args
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(4096),
        }),
        "write" => Action::Write(proto::bridge_v1::NetWriteRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            data: json_value_to_bytes(args.get("data")),
        }),
        "read_until" => Action::ReadUntil(proto::bridge_v1::NetReadUntilRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            delimiter: json_value_to_bytes(args.get("delimiter")),
            max_bytes: args
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(65536),
        }),
        "tls_upgrade" => Action::TlsUpgrade(proto::bridge_v1::NetTlsUpgradeRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            server_name: args
                .get("server_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "close" => Action::Close(proto::bridge_v1::NetHandleRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        other => {
            return Err(core_err(format!(
                "unsupported net proto action '{}'",
                other
            )));
        }
    };
    Ok(proto::bridge_v1::NetRequest {
        schema_version: 1,
        action: Some(action),
    })
}

pub(super) fn net_proto_request_to_action_payload(
    req: &proto::bridge_v1::NetRequest,
) -> Result<(String, serde_json::Value, NetProtoActionKind), deno_core::error::CoreError> {
    use proto::bridge_v1::net_request::Action;
    let Some(action) = req.action.as_ref() else {
        return Err(core_err("net proto request missing action"));
    };
    match action {
        Action::Connect(connect) => Ok((
            "connect".to_string(),
            serde_json::json!({
                "host": connect.host,
                "port": connect.port,
                "timeout_ms": connect.timeout_ms,
            }),
            NetProtoActionKind::Connect,
        )),
        Action::ConnectTls(connect) => Ok((
            "connect_tls".to_string(),
            serde_json::json!({
                "host": connect.host,
                "port": connect.port,
                "server_name": connect.server_name,
                "timeout_ms": connect.timeout_ms,
                "ca_cert": bytes_to_json_array(&connect.ca_cert),
            }),
            NetProtoActionKind::ConnectTls,
        )),
        Action::Listen(listen) => Ok((
            "listen".to_string(),
            serde_json::json!({
                "host": listen.host,
                "port": listen.port,
                "backlog": listen.backlog,
            }),
            NetProtoActionKind::Listen,
        )),
        Action::ListenTls(listen) => Ok((
            "listen_tls".to_string(),
            serde_json::json!({
                "host": listen.host,
                "port": listen.port,
                "backlog": listen.backlog,
                "cert": bytes_to_json_array(&listen.cert),
                "key": bytes_to_json_array(&listen.key),
            }),
            NetProtoActionKind::ListenTls,
        )),
        Action::Accept(accept) => Ok((
            "accept".to_string(),
            serde_json::json!({
                "handle": accept.handle,
            }),
            NetProtoActionKind::Accept,
        )),
        Action::SetDeadline(deadline) => Ok((
            "set_deadline".to_string(),
            serde_json::json!({
                "handle": deadline.handle,
                "millis": deadline.millis,
            }),
            NetProtoActionKind::SetDeadline,
        )),
        Action::Read(read) => Ok((
            "read".to_string(),
            serde_json::json!({
                "handle": read.handle,
                "max_bytes": read.max_bytes,
            }),
            NetProtoActionKind::Read,
        )),
        Action::Write(write) => Ok((
            "write".to_string(),
            serde_json::json!({
                "handle": write.handle,
                "data": bytes_to_json_array(&write.data),
            }),
            NetProtoActionKind::Write,
        )),
        Action::ReadUntil(read_until) => Ok((
            "read_until".to_string(),
            serde_json::json!({
                "handle": read_until.handle,
                "delimiter": bytes_to_json_array(&read_until.delimiter),
                "max_bytes": read_until.max_bytes,
            }),
            NetProtoActionKind::ReadUntil,
        )),
        Action::TlsUpgrade(upgrade) => Ok((
            "tls_upgrade".to_string(),
            serde_json::json!({
                "handle": upgrade.handle,
                "server_name": upgrade.server_name,
            }),
            NetProtoActionKind::TlsUpgrade,
        )),
        Action::Close(close) => Ok((
            "close".to_string(),
            serde_json::json!({ "handle": close.handle }),
            NetProtoActionKind::Close,
        )),
    }
}

pub(super) fn net_json_response_to_proto(
    resp: &serde_json::Value,
    kind: NetProtoActionKind,
) -> proto::bridge_v1::NetResponse {
    use proto::bridge_v1::net_response::Action;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let error = resp
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let action = match kind {
        NetProtoActionKind::Connect => Some(Action::Connect(proto::bridge_v1::NetHandleResponse {
            handle: resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            reused: resp
                .get("reused")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })),
        NetProtoActionKind::ConnectTls => {
            Some(Action::ConnectTls(proto::bridge_v1::NetHandleResponse {
                handle: resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
                reused: resp
                    .get("reused")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            }))
        }
        NetProtoActionKind::Listen => Some(Action::Listen(proto::bridge_v1::NetListenResponse {
            handle: resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        })),
        NetProtoActionKind::ListenTls => {
            Some(Action::ListenTls(proto::bridge_v1::NetListenResponse {
                handle: resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            }))
        }
        NetProtoActionKind::Accept => Some(Action::Accept(proto::bridge_v1::NetAcceptResponse {
            handle: resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            peer_addr: resp
                .get("peer_addr")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })),
        NetProtoActionKind::SetDeadline => {
            Some(Action::SetDeadline(proto::bridge_v1::NetUnitResponse {
                ok,
            }))
        }
        NetProtoActionKind::Read => Some(Action::Read(proto::bridge_v1::NetReadResponse {
            data: json_value_to_bytes(resp.get("data")),
            eof: resp.get("eof").and_then(|v| v.as_bool()).unwrap_or(false),
        })),
        NetProtoActionKind::Write => Some(Action::Write(proto::bridge_v1::NetWriteResponse {
            written: resp.get("written").and_then(|v| v.as_u64()).unwrap_or(0),
        })),
        NetProtoActionKind::ReadUntil => {
            Some(Action::ReadUntil(proto::bridge_v1::NetReadUntilResponse {
                data: json_value_to_bytes(resp.get("data")),
                found: resp.get("found").and_then(|v| v.as_bool()).unwrap_or(false),
                eof: resp.get("eof").and_then(|v| v.as_bool()).unwrap_or(false),
            }))
        }
        NetProtoActionKind::TlsUpgrade => {
            Some(Action::TlsUpgrade(proto::bridge_v1::NetHandleResponse {
                handle: resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
                reused: resp
                    .get("reused")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            }))
        }
        NetProtoActionKind::Close => Some(Action::Close(proto::bridge_v1::NetUnitResponse { ok })),
    };

    proto::bridge_v1::NetResponse {
        schema_version: 1,
        ok,
        error,
        action,
    }
}

pub(super) fn net_proto_response_to_json(
    resp: &proto::bridge_v1::NetResponse,
) -> serde_json::Value {
    use proto::bridge_v1::net_response::Action;
    let mut out = serde_json::Map::new();
    out.insert("ok".to_string(), serde_json::Value::Bool(resp.ok));
    if !resp.error.is_empty() {
        out.insert(
            "error".to_string(),
            serde_json::Value::String(resp.error.clone()),
        );
    }

    if let Some(action) = resp.action.as_ref() {
        match action {
            Action::Connect(handle) | Action::ConnectTls(handle) | Action::TlsUpgrade(handle) => {
                out.insert(
                    "handle".to_string(),
                    serde_json::Value::Number(handle.handle.into()),
                );
                out.insert("reused".to_string(), serde_json::Value::Bool(handle.reused));
            }
            Action::Listen(handle) | Action::ListenTls(handle) => {
                out.insert(
                    "handle".to_string(),
                    serde_json::Value::Number(handle.handle.into()),
                );
            }
            Action::Accept(accept) => {
                out.insert(
                    "handle".to_string(),
                    serde_json::Value::Number(accept.handle.into()),
                );
                out.insert(
                    "peer_addr".to_string(),
                    serde_json::Value::String(accept.peer_addr.clone()),
                );
            }
            Action::SetDeadline(unit) | Action::Close(unit) => {
                out.insert("ok".to_string(), serde_json::Value::Bool(unit.ok));
            }
            Action::Read(read) => {
                out.insert("data".to_string(), bytes_to_json_array(&read.data));
                out.insert("eof".to_string(), serde_json::Value::Bool(read.eof));
            }
            Action::Write(write) => {
                out.insert(
                    "written".to_string(),
                    serde_json::Value::Number(write.written.into()),
                );
            }
            Action::ReadUntil(read_until) => {
                out.insert("data".to_string(), bytes_to_json_array(&read_until.data));
                out.insert(
                    "found".to_string(),
                    serde_json::Value::Bool(read_until.found),
                );
                out.insert("eof".to_string(), serde_json::Value::Bool(read_until.eof));
            }
        }
    }

    serde_json::Value::Object(out)
}

pub(super) fn net_call_proto_impl(
    state: &mut NetState,
    request: &[u8],
) -> Result<Vec<u8>, deno_core::error::CoreError> {
    // A seeded policy wins; otherwise resolve the per-execution security
    // context the dispatch path installed (production path, deka#801). A
    // missing context is an error — never a silent default policy.
    let policy = match state.policy.clone() {
        Some(policy) => policy,
        None => security_policy_from_context()?,
    };
    net_call_proto_impl_with(state, &policy, request)
}

/// The dispatch itself, with the policy passed in. See `NetState::policy`
/// for why this exists.
pub(super) fn net_call_proto_impl_with(
    state: &mut NetState,
    policy: &SecurityPolicy,
    request: &[u8],
) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let started = Instant::now();
    let req = proto::bridge_v1::NetRequest::decode(request)
        .map_err(|e| core_err(format!("net proto decode failed: {}", e)))?;
    let (action, payload, kind) = net_proto_request_to_action_payload(&req)?;
    validate_tcp_connect_port(&action, &payload)?;
    let net_target = match kind {
        NetProtoActionKind::Connect => net_policy_target(&payload)
            .ok_or_else(|| core_err("connect: missing capability target"))?,
        NetProtoActionKind::ConnectTls => net_policy_target(&payload)
            .ok_or_else(|| core_err("connect_tls: missing capability target"))?,
        NetProtoActionKind::Listen | NetProtoActionKind::ListenTls => {
            net_policy_target(&payload)
                .ok_or_else(|| core_err("listen: missing capability target"))?
        }
        NetProtoActionKind::Accept => {
            let handle = payload
                .get("handle")
                .and_then(|value| value.as_u64())
                .ok_or_else(|| core_err(format!("{action}: missing handle")))?;
            listener_target(state, handle)?
        }
        NetProtoActionKind::SetDeadline
        | NetProtoActionKind::Read
        | NetProtoActionKind::ReadUntil
        | NetProtoActionKind::Write
        | NetProtoActionKind::TlsUpgrade
        | NetProtoActionKind::Close => {
            let handle = payload
                .get("handle")
                .and_then(|value| value.as_u64())
                .ok_or_else(|| core_err(format!("{action}: missing handle")))?;
            handle_target(state, handle)?
        }
    };
    enforce_net_with(policy, Some(&net_target))?;
    let response_json = net_call_impl(state, action, payload)?;
    let response = net_json_response_to_proto(&response_json, kind);
    let out = response.encode_to_vec();
    record_bridge_proto_metric(
        "net",
        request.len(),
        out.len(),
        started.elapsed().as_micros() as u64,
    );
    Ok(out)
}

fn validate_tcp_connect_port(
    action: &str,
    payload: &serde_json::Value,
) -> Result<(), deno_core::error::CoreError> {
    if action == "connect" || action == "connect_tls" {
        tcp_connect_port(payload)?;
    }
    Ok(())
}

fn tcp_connect_port(payload: &serde_json::Value) -> Result<u16, deno_core::error::CoreError> {
    let port = payload
        .get("port")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| core_err("connect: port must be in range 1..=65535"))
}

fn tcp_bind_port(payload: &serde_json::Value) -> Result<u16, deno_core::error::CoreError> {
    let port = payload
        .get("port")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    u16::try_from(port)
        .ok()
        .ok_or_else(|| core_err("listen: port must be in range 0..=65535"))
}

/// Preserve the requested port when a TCP connection is checked against the
/// manifest. A `net.allow` entry may deliberately grant just one endpoint
/// (`registry.internal:443`), rather than every port on that host.
fn net_policy_target(payload: &serde_json::Value) -> Option<String> {
    let host = payload.get("host")?.as_str()?;
    match payload.get("port").and_then(|value| value.as_u64()) {
        Some(port) => Some(capability_target(host, u16::try_from(port).ok()?)),
        None => Some(host.to_string()),
    }
}

#[op2]
#[buffer]
pub(super) fn op_php_net_call_proto(
    op_state: Rc<RefCell<OpState>>,
    #[buffer] request: &[u8],
) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let mut op_state = op_state.borrow_mut();
    let state = op_state.borrow_mut::<NetState>();
    net_call_proto_impl(state, request)
}

#[op2]
#[buffer]
pub(super) fn op_php_net_proto_encode(
    #[string] action: String,
    #[serde] payload: serde_json::Value,
) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let request = net_action_payload_to_proto_request(&action, &payload)?;
    Ok(request.encode_to_vec())
}

#[op2]
#[serde]
pub(super) fn op_php_net_proto_decode(
    #[buffer] response: &[u8],
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let decoded = proto::bridge_v1::NetResponse::decode(response)
        .map_err(|e| core_err(format!("net proto decode response failed: {}", e)))?;
    Ok(net_proto_response_to_json(&decoded))
}

#[cfg(test)]
mod tests {
    use super::proto;
    use super::{
        NetState, SecurityPolicy, net_call_proto_impl_with, net_policy_target, tcp_connect_port,
    };
    use prost::Message;
    use serde_json::json;
    use std::io::{ErrorKind, Write};
    use std::net::TcpListener;

    /// Build a policy that allows exactly one net target. Tests pass this
    /// in directly instead of setting `DEKA_SECURITY_POLICY` — the env is
    /// process-global and parallel tests raced on it (deka#537).
    fn test_net_policy(allow: &str) -> SecurityPolicy {
        let document = json!({ "security": { "allow": { "net": [allow] } } });
        runtime_core::security_policy::parse_deka_security_policy(&document).policy
    }

    #[test]
    fn tcp_policy_target_preserves_manifest_port_scope() {
        assert_eq!(
            net_policy_target(&json!({ "host": "127.0.0.1", "port": 9418 })),
            Some("127.0.0.1:9418".to_string())
        );
    }

    #[test]
    fn tcp_policy_target_does_not_invent_a_port() {
        assert_eq!(
            net_policy_target(&json!({ "host": "registry.tana.gg" })),
            Some("registry.tana.gg".to_string())
        );
    }

    #[test]
    fn tcp_connect_port_rejects_overflow_values() {
        for port in [65_536, 65_537, u32::MAX as u64] {
            assert!(
                tcp_connect_port(&json!({ "port": port })).is_err(),
                "port {port}"
            );
        }
    }

    #[test]
    fn tcp_proto_rejects_overflow_before_policy_or_socket_connect() {
        let mut state = NetState::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let wrapped_port = listener.local_addr().expect("listener address").port() as u32;
        let overflow_port = wrapped_port + 65_536;

        let request = proto::bridge_v1::NetRequest {
            schema_version: 1,
            action: Some(proto::bridge_v1::net_request::Action::Connect(
                proto::bridge_v1::NetConnectRequest {
                    host: "127.0.0.1".to_string(),
                    port: overflow_port,
                    timeout_ms: 100,
                },
            )),
        };
        let policy = test_net_policy(&format!("127.0.0.1:{overflow_port}"));
        let error = net_call_proto_impl_with(&mut state, &policy, &request.encode_to_vec())
            .expect_err("overflow rejected");
        assert!(error.to_string().contains("1..=65535"));
        assert!(matches!(listener.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock));
    }

    #[test]
    fn tcp_host_port_policy_allows_only_the_manifest_endpoint() {
        let policy = test_net_policy("127.0.0.1:9418");

        assert!(super::super::security::enforce_net_with(&policy, Some("127.0.0.1:9418")).is_ok());
        assert!(super::super::security::enforce_net_with(&policy, Some("127.0.0.1:9419")).is_err());
        assert!(super::super::security::enforce_net_with(&policy, Some("127.0.0.1")).is_err());
    }

    #[test]
    fn tcp_handle_operations_retain_the_allowed_connect_target() {
        let mut state = NetState::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let address = listener.local_addr().expect("listener address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut buffer = [0_u8; 4];
            std::io::Read::read_exact(&mut stream, &mut buffer).expect("read request");
            std::io::Write::write_all(&mut stream, &buffer).expect("write response");
        });
        let allowed = format!("127.0.0.1:{}", address.port());
        let allowed_policy = test_net_policy(&allowed);

        let connect = super::net_action_payload_to_proto_request(
            "connect",
            &json!({ "host": "127.0.0.1", "port": address.port() }),
        )
        .expect("encode connect");
        let connect_response =
            net_call_proto_impl_with(&mut state, &allowed_policy, &connect.encode_to_vec())
                .expect("connect");
        let connect_response = proto::bridge_v1::NetResponse::decode(connect_response.as_slice())
            .expect("decode connect response");
        let handle = match connect_response.action.expect("connect action") {
            proto::bridge_v1::net_response::Action::Connect(response) => response.handle,
            other => panic!("unexpected connect response: {other:?}"),
        };

        for (action, payload) in [
            ("write", json!({ "handle": handle, "data": "ping" })),
            ("read", json!({ "handle": handle, "max_bytes": 4 })),
        ] {
            let request = super::net_action_payload_to_proto_request(action, &payload)
                .expect("encode handle operation");
            assert!(
                net_call_proto_impl_with(&mut state, &allowed_policy, &request.encode_to_vec())
                    .is_ok(),
                "{action} must use the allowed connect target"
            );
        }

        let denied_policy = test_net_policy("127.0.0.1:1");
        let denied_write = super::net_action_payload_to_proto_request(
            "write",
            &json!({ "handle": handle, "data": "nope" }),
        )
        .expect("encode denied write");
        assert!(
            net_call_proto_impl_with(&mut state, &denied_policy, &denied_write.encode_to_vec())
                .is_err()
        );

        let denied_connect = super::net_action_payload_to_proto_request(
            "connect",
            &json!({ "host": "127.0.0.1", "port": address.port() }),
        )
        .expect("encode denied connect");
        assert!(
            net_call_proto_impl_with(&mut state, &denied_policy, &denied_connect.encode_to_vec())
                .is_err()
        );

        let close =
            super::net_action_payload_to_proto_request("close", &json!({ "handle": handle }))
                .expect("encode close");
        assert!(
            net_call_proto_impl_with(&mut state, &allowed_policy, &close.encode_to_vec()).is_ok()
        );
        server.join().expect("server join");
    }

    #[test]
    fn tcp_proto_binary_roundtrip_preserves_non_utf8_bytes() {
        let mut state = NetState::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let address = listener.local_addr().expect("listener address");
        let payload = vec![0_u8, 1, 2, 127, 128, 200, 255];
        let server = std::thread::spawn({
            let payload = payload.clone();
            move || {
                let (mut stream, _) = listener.accept().expect("accept connection");
                let mut buf = vec![0_u8; payload.len()];
                std::io::Read::read_exact(&mut stream, &mut buf).expect("read request");
                assert_eq!(buf, payload, "server received unexpected bytes");
                std::io::Write::write_all(&mut stream, &buf).expect("write response");
            }
        });
        let allowed = format!("127.0.0.1:{}", address.port());
        let policy = test_net_policy(&allowed);

        let connect = super::net_action_payload_to_proto_request(
            "connect",
            &json!({ "host": "127.0.0.1", "port": address.port() }),
        )
        .expect("encode connect");
        let connect_response =
            net_call_proto_impl_with(&mut state, &policy, &connect.encode_to_vec())
                .expect("connect");
        let connect_response = proto::bridge_v1::NetResponse::decode(connect_response.as_slice())
            .expect("decode connect response");
        let handle = match connect_response.action.expect("connect action") {
            proto::bridge_v1::net_response::Action::Connect(response) => response.handle,
            other => panic!("unexpected connect response: {other:?}"),
        };

        let write = super::net_action_payload_to_proto_request(
            "write",
            &json!({
                "handle": handle,
                "data": payload.iter().map(|b| *b as u64).collect::<Vec<_>>()
            }),
        )
        .expect("encode write");
        let write_response =
            net_call_proto_impl_with(&mut state, &policy, &write.encode_to_vec()).expect("write");
        let write_response = proto::bridge_v1::NetResponse::decode(write_response.as_slice())
            .expect("decode write response");
        assert_eq!(
            write_response
                .action
                .and_then(|a| match a {
                    proto::bridge_v1::net_response::Action::Write(w) => Some(w.written),
                    _ => None,
                })
                .unwrap_or(0) as usize,
            payload.len()
        );

        let read = super::net_action_payload_to_proto_request(
            "read",
            &json!({ "handle": handle, "max_bytes": payload.len() as u64 }),
        )
        .expect("encode read");
        let read_response =
            net_call_proto_impl_with(&mut state, &policy, &read.encode_to_vec()).expect("read");
        let read_response = proto::bridge_v1::NetResponse::decode(read_response.as_slice())
            .expect("decode read response");
        let read_bytes = match read_response.action.expect("read action") {
            proto::bridge_v1::net_response::Action::Read(r) => r.data,
            other => panic!("unexpected read response: {other:?}"),
        };
        assert_eq!(read_bytes, payload);

        let close =
            super::net_action_payload_to_proto_request("close", &json!({ "handle": handle }))
                .expect("encode close");
        assert!(net_call_proto_impl_with(&mut state, &policy, &close.encode_to_vec()).is_ok());
        server.join().expect("server join");
    }

    #[test]
    fn tcp_listen_accept_roundtrip() {
        let mut state = NetState::new();

        let raw_listener = TcpListener::bind("127.0.0.1:0").expect("reserve listener");
        let addr = raw_listener.local_addr().expect("listener address");
        let allowed = format!("127.0.0.1:{}", addr.port());
        let policy = test_net_policy(&allowed);
        drop(raw_listener);

        let listen = super::net_action_payload_to_proto_request(
            "listen",
            &json!({
                "host": "127.0.0.1",
                "port": addr.port(),
                "backlog": 5,
            }),
        )
        .expect("encode listen");
        let listen_response =
            net_call_proto_impl_with(&mut state, &policy, &listen.encode_to_vec()).expect("listen");
        let listen_response = proto::bridge_v1::NetResponse::decode(listen_response.as_slice())
            .expect("decode listen response");
        let listen_handle = match listen_response.action.expect("listen action") {
            proto::bridge_v1::net_response::Action::Listen(response) => response.handle,
            other => panic!("unexpected listen response: {other:?}"),
        };

        let client = std::thread::spawn(move || {
            let mut stream = std::net::TcpStream::connect(addr).expect("client connect");
            stream.write_all(b"ping").expect("client write");
            let mut buf = [0_u8; 4];
            std::io::Read::read_exact(&mut stream, &mut buf).expect("client read");
            assert_eq!(&buf, b"pong");
        });

        let accept = super::net_action_payload_to_proto_request(
            "accept",
            &json!({ "handle": listen_handle }),
        )
        .expect("encode accept");
        let accept_response =
            net_call_proto_impl_with(&mut state, &policy, &accept.encode_to_vec()).expect("accept");
        let accept_response = proto::bridge_v1::NetResponse::decode(accept_response.as_slice())
            .expect("decode accept response");
        let conn_handle = match accept_response.action.expect("accept action") {
            proto::bridge_v1::net_response::Action::Accept(response) => response.handle,
            other => panic!("unexpected accept response: {other:?}"),
        };

        let read = super::net_action_payload_to_proto_request(
            "read",
            &json!({ "handle": conn_handle, "max_bytes": 4 }),
        )
        .expect("encode read");
        let read_response =
            net_call_proto_impl_with(&mut state, &policy, &read.encode_to_vec()).expect("read");
        let read_response = proto::bridge_v1::NetResponse::decode(read_response.as_slice())
            .expect("decode read response");
        let read_bytes = match read_response.action.expect("read action") {
            proto::bridge_v1::net_response::Action::Read(r) => r.data,
            other => panic!("unexpected read response: {other:?}"),
        };
        assert_eq!(read_bytes, b"ping");

        let write = super::net_action_payload_to_proto_request(
            "write",
            &json!({
                "handle": conn_handle,
                "data": b"pong".iter().map(|b| *b as u64).collect::<Vec<_>>()
            }),
        )
        .expect("encode write");
        net_call_proto_impl_with(&mut state, &policy, &write.encode_to_vec()).expect("write");

        client.join().expect("client join");
    }

    #[test]
    fn tls_listen_accept_and_connect_tls_roundtrip() {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])
            .expect("generate self-signed cert");
        let cert_pem = cert.cert.pem().into_bytes();
        let key_pem = cert.key_pair.serialize_pem().into_bytes();

        let mut state = NetState::new();

        let raw_listener = TcpListener::bind("127.0.0.1:0").expect("reserve listener");
        let addr = raw_listener.local_addr().expect("listener address");
        let allowed = format!("127.0.0.1:{}", addr.port());
        let policy = test_net_policy(&allowed);
        drop(raw_listener);

        let listen = super::net_action_payload_to_proto_request(
            "listen_tls",
            &json!({
                "host": "127.0.0.1",
                "port": addr.port(),
                "backlog": 5,
                "cert": cert_pem.iter().map(|b| *b as u64).collect::<Vec<_>>(),
                "key": key_pem.iter().map(|b| *b as u64).collect::<Vec<_>>(),
            }),
        )
        .expect("encode listen_tls");
        let listen_response =
            net_call_proto_impl_with(&mut state, &policy, &listen.encode_to_vec())
                .expect("listen_tls");
        let listen_response = proto::bridge_v1::NetResponse::decode(listen_response.as_slice())
            .expect("decode listen response");
        let listen_handle = match listen_response.action.expect("listen action") {
            proto::bridge_v1::net_response::Action::ListenTls(response) => response.handle,
            other => panic!("unexpected listen response: {other:?}"),
        };

        let client_ca_cert = cert_pem.clone();
        let client_policy = policy.clone();
        let client = std::thread::spawn(move || {
            let mut client_state = NetState::new();
            let connect_tls = super::net_action_payload_to_proto_request(
                "connect_tls",
                &json!({
                    "host": "127.0.0.1",
                    "port": addr.port(),
                    "server_name": "localhost",
                    "timeout_ms": 5000,
                    "ca_cert": client_ca_cert.iter().map(|b| *b as u64).collect::<Vec<_>>(),
                }),
            )
            .expect("encode connect_tls");
            let connect_response = net_call_proto_impl_with(
                &mut client_state,
                &client_policy,
                &connect_tls.encode_to_vec(),
            )
            .expect("connect_tls");
            let connect_response =
                proto::bridge_v1::NetResponse::decode(connect_response.as_slice())
                    .expect("decode connect response");
            let handle = match connect_response.action.expect("connect action") {
                proto::bridge_v1::net_response::Action::ConnectTls(response) => response.handle,
                other => panic!("unexpected connect response: {other:?}"),
            };

            let write = super::net_action_payload_to_proto_request(
                "write",
                &json!({
                    "handle": handle,
                    "data": b"ping".iter().map(|b| *b as u64).collect::<Vec<_>>()
                }),
            )
            .expect("encode write");
            net_call_proto_impl_with(&mut client_state, &client_policy, &write.encode_to_vec())
                .expect("write");

            let read = super::net_action_payload_to_proto_request(
                "read",
                &json!({ "handle": handle, "max_bytes": 4 }),
            )
            .expect("encode read");
            let read_response =
                net_call_proto_impl_with(&mut client_state, &client_policy, &read.encode_to_vec())
                    .expect("read");
            let read_response = proto::bridge_v1::NetResponse::decode(read_response.as_slice())
                .expect("decode read response");
            let read_bytes = match read_response.action.expect("read action") {
                proto::bridge_v1::net_response::Action::Read(r) => r.data,
                other => panic!("unexpected read response: {other:?}"),
            };
            assert_eq!(read_bytes, b"pong");

            let close =
                super::net_action_payload_to_proto_request("close", &json!({ "handle": handle }))
                    .expect("encode close");
            net_call_proto_impl_with(&mut client_state, &client_policy, &close.encode_to_vec())
                .expect("close");
        });

        let accept = super::net_action_payload_to_proto_request(
            "accept",
            &json!({ "handle": listen_handle }),
        )
        .expect("encode accept");
        let accept_response =
            net_call_proto_impl_with(&mut state, &policy, &accept.encode_to_vec()).expect("accept");
        let accept_response = proto::bridge_v1::NetResponse::decode(accept_response.as_slice())
            .expect("decode accept response");
        let (conn_handle, peer_addr) = match accept_response.action.expect("accept action") {
            proto::bridge_v1::net_response::Action::Accept(response) => {
                (response.handle, response.peer_addr)
            }
            other => panic!("unexpected accept response: {other:?}"),
        };
        assert!(!peer_addr.is_empty());

        let read = super::net_action_payload_to_proto_request(
            "read",
            &json!({ "handle": conn_handle, "max_bytes": 4 }),
        )
        .expect("encode read");
        let read_response =
            net_call_proto_impl_with(&mut state, &policy, &read.encode_to_vec()).expect("read");
        let read_response = proto::bridge_v1::NetResponse::decode(read_response.as_slice())
            .expect("decode read response");
        let read_bytes = match read_response.action.expect("read action") {
            proto::bridge_v1::net_response::Action::Read(r) => r.data,
            other => panic!("unexpected read response: {other:?}"),
        };
        assert_eq!(read_bytes, b"ping");

        let write = super::net_action_payload_to_proto_request(
            "write",
            &json!({
                "handle": conn_handle,
                "data": b"pong".iter().map(|b| *b as u64).collect::<Vec<_>>()
            }),
        )
        .expect("encode write");
        net_call_proto_impl_with(&mut state, &policy, &write.encode_to_vec()).expect("write");

        client.join().expect("client join");
    }

    #[test]
    fn tcp_proto_read_until_finds_delimiter_and_eof() {
        let mut state = NetState::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let address = listener.local_addr().expect("listener address");
        let payload = b"hello\r\nworld";
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            std::io::Write::write_all(&mut stream, payload).expect("write payload");
        });

        let policy = test_net_policy(&format!("127.0.0.1:{}", address.port()));

        let connect = super::net_action_payload_to_proto_request(
            "connect",
            &json!({ "host": "127.0.0.1", "port": address.port() }),
        )
        .expect("encode connect");
        let connect_response =
            net_call_proto_impl_with(&mut state, &policy, &connect.encode_to_vec())
                .expect("connect");
        let connect_response = proto::bridge_v1::NetResponse::decode(connect_response.as_slice())
            .expect("decode connect response");
        let handle = match connect_response.action.expect("connect action") {
            proto::bridge_v1::net_response::Action::Connect(response) => response.handle,
            other => panic!("unexpected connect response: {other:?}"),
        };

        let read_until = super::net_action_payload_to_proto_request(
            "read_until",
            &json!({
                "handle": handle,
                "delimiter": [b'\r', b'\n'],
                "max_bytes": 128
            }),
        )
        .expect("encode read_until");
        let read_response =
            net_call_proto_impl_with(&mut state, &policy, &read_until.encode_to_vec())
                .expect("read_until");
        let read_response = proto::bridge_v1::NetResponse::decode(read_response.as_slice())
            .expect("decode read_until response");
        let (data, found, eof) = match read_response.action.expect("read_until action") {
            proto::bridge_v1::net_response::Action::ReadUntil(r) => (r.data, r.found, r.eof),
            other => panic!("unexpected read_until response: {other:?}"),
        };
        assert_eq!(data, b"hello");
        assert!(found);
        assert!(!eof);

        let close =
            super::net_action_payload_to_proto_request("close", &json!({ "handle": handle }))
                .expect("encode close");
        net_call_proto_impl_with(&mut state, &policy, &close.encode_to_vec()).expect("close");
        server.join().expect("server join");
    }
}
