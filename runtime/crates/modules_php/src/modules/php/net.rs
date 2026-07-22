use super::bridge_metrics::record_bridge_proto_metric;
use super::security::enforce_net;
use super::*;

pub(super) enum NetConn {
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
}

pub(super) struct NetState {
    next_handle: u64,
    handles: HashMap<u64, NetConn>,
}

impl NetState {
    fn new() -> Self {
        Self {
            next_handle: 1,
            handles: HashMap::new(),
        }
    }
}

static NET_STATE: OnceLock<Mutex<NetState>> = OnceLock::new();

pub(super) fn net_state() -> &'static Mutex<NetState> {
    NET_STATE.get_or_init(|| Mutex::new(NetState::new()))
}

pub(super) fn net_call_impl(
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
            let addr = format!("{}:{}", host, port);
            let mut addrs = addr
                .to_socket_addrs()
                .map_err(|e| err(format!("connect: resolve failed: {}", e)))?;
            let target = addrs
                .next()
                .ok_or_else(|| err("connect: no resolved address".to_string()))?;
            let stream = TcpStream::connect_timeout(&target, Duration::from_millis(timeout_ms))
                .map_err(|e| err(format!("connect: {}", e)))?;
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            let handle = state.next_handle;
            state.next_handle += 1;
            state.handles.insert(handle, NetConn::Tcp(stream));
            Ok(serde_json::json!({ "ok": true, "handle": handle }))
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
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            let Some(conn) = state.handles.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("set_deadline: unknown handle {}", handle) }),
                );
            };
            let result = match conn {
                NetConn::Tcp(stream) => stream
                    .set_read_timeout(timeout)
                    .and_then(|_| stream.set_write_timeout(timeout)),
                NetConn::Tls(stream) => stream
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
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            let Some(conn) = state.handles.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("read: unknown handle {}", handle) }),
                );
            };
            let n = match conn {
                NetConn::Tcp(stream) => stream.read(&mut buf),
                NetConn::Tls(stream) => stream.read(&mut buf),
            };
            match n {
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    Ok(serde_json::json!({ "ok": true, "data": data, "eof": n == 0 }))
                }
                Err(e) => Ok(serde_json::json!({ "ok": false, "error": format!("read: {}", e) })),
            }
        }
        "write" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("write: missing handle".to_string()))?;
            let data = args_obj
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .as_bytes()
                .to_vec();
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            let Some(conn) = state.handles.get_mut(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("write: unknown handle {}", handle) }),
                );
            };
            let result = match conn {
                NetConn::Tcp(stream) => stream.write_all(&data),
                NetConn::Tls(stream) => stream.write_all(&data),
            };
            match result {
                Ok(()) => Ok(serde_json::json!({ "ok": true, "written": data.len() })),
                Err(e) => Ok(serde_json::json!({ "ok": false, "error": format!("write: {}", e) })),
            }
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
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            let Some(conn) = state.handles.remove(&handle) else {
                return Ok(
                    serde_json::json!({ "ok": false, "error": format!("tls_upgrade: unknown handle {}", handle) }),
                );
            };
            let tcp = match conn {
                NetConn::Tcp(stream) => stream,
                NetConn::Tls(stream) => {
                    let new_handle = state.next_handle;
                    state.next_handle += 1;
                    state.handles.insert(new_handle, NetConn::Tls(stream));
                    return Ok(
                        serde_json::json!({ "ok": true, "handle": new_handle, "reused": true }),
                    );
                }
            };
            let connector = TlsConnector::new()
                .map_err(|e| err(format!("tls_upgrade: connector init failed: {}", e)))?;
            match connector.connect(&server_name, tcp) {
                Ok(stream) => {
                    let new_handle = state.next_handle;
                    state.next_handle += 1;
                    state.handles.insert(new_handle, NetConn::Tls(stream));
                    Ok(serde_json::json!({ "ok": true, "handle": new_handle }))
                }
                Err(e) => {
                    Ok(serde_json::json!({ "ok": false, "error": format!("tls_upgrade: {}", e) }))
                }
            }
        }
        "close" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("close: missing handle".to_string()))?;
            let mut state = net_state()
                .lock()
                .map_err(|_| err("net lock poisoned".to_string()))?;
            state.handles.remove(&handle);
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
            data: args
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
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
                "data": write.data,
            }),
            NetProtoActionKind::Write,
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
        NetProtoActionKind::SetDeadline => {
            Some(Action::SetDeadline(proto::bridge_v1::NetUnitResponse {
                ok,
            }))
        }
        NetProtoActionKind::Read => Some(Action::Read(proto::bridge_v1::NetReadResponse {
            data: resp
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            eof: resp.get("eof").and_then(|v| v.as_bool()).unwrap_or(false),
        })),
        NetProtoActionKind::Write => Some(Action::Write(proto::bridge_v1::NetWriteResponse {
            written: resp.get("written").and_then(|v| v.as_u64()).unwrap_or(0),
        })),
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
            Action::Connect(handle) | Action::TlsUpgrade(handle) => {
                out.insert(
                    "handle".to_string(),
                    serde_json::Value::Number(handle.handle.into()),
                );
                out.insert("reused".to_string(), serde_json::Value::Bool(handle.reused));
            }
            Action::SetDeadline(unit) | Action::Close(unit) => {
                out.insert("ok".to_string(), serde_json::Value::Bool(unit.ok));
            }
            Action::Read(read) => {
                out.insert(
                    "data".to_string(),
                    serde_json::Value::String(read.data.clone()),
                );
                out.insert("eof".to_string(), serde_json::Value::Bool(read.eof));
            }
            Action::Write(write) => {
                out.insert(
                    "written".to_string(),
                    serde_json::Value::Number(write.written.into()),
                );
            }
        }
    }

    serde_json::Value::Object(out)
}

pub(super) fn net_call_proto_impl(request: &[u8]) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let started = Instant::now();
    let req = proto::bridge_v1::NetRequest::decode(request)
        .map_err(|e| core_err(format!("net proto decode failed: {}", e)))?;
    let (action, payload, kind) = net_proto_request_to_action_payload(&req)?;
    validate_tcp_connect_port(&action, &payload)?;
    enforce_net(net_policy_target(&payload).as_deref())?;
    let response_json = net_call_impl(action, payload)?;
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
    if action == "connect" {
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

/// Preserve the requested port when a TCP connection is checked against the
/// manifest. A `net.allow` entry may deliberately grant just one endpoint
/// (`registry.internal:443`), rather than every port on that host.
fn net_policy_target(payload: &serde_json::Value) -> Option<String> {
    let host = payload.get("host")?.as_str()?;
    let port = payload.get("port").and_then(|value| value.as_u64());
    Some(match port {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

#[op2]
#[buffer]
pub(super) fn op_php_net_call_proto(
    #[buffer] request: &[u8],
) -> Result<Vec<u8>, deno_core::error::CoreError> {
    net_call_proto_impl(request)
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
    use super::{net_call_proto_impl, net_policy_target, tcp_connect_port};
    use prost::Message;
    use serde_json::json;
    use std::io::ErrorKind;
    use std::net::TcpListener;
    use std::sync::{Mutex, OnceLock};

    fn policy_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
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
        let _lock = policy_lock().lock().expect("policy lock");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let wrapped_port = listener.local_addr().expect("listener address").port() as u32;
        let overflow_port = wrapped_port + 65_536;
        let previous = std::env::var_os("DEKA_SECURITY_POLICY");
        unsafe {
            std::env::set_var(
                "DEKA_SECURITY_POLICY",
                format!(r#"{{"security":{{"allow":{{"net":["127.0.0.1:{overflow_port}"]}}}}}}"#),
            );
        }

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
        let error = net_call_proto_impl(&request.encode_to_vec()).expect_err("overflow rejected");
        assert!(error.to_string().contains("1..=65535"));
        assert!(matches!(listener.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock));

        unsafe {
            match previous {
                Some(value) => std::env::set_var("DEKA_SECURITY_POLICY", value),
                None => std::env::remove_var("DEKA_SECURITY_POLICY"),
            }
        }
    }

    #[test]
    fn tcp_host_port_policy_allows_only_the_manifest_endpoint() {
        let _lock = policy_lock().lock().expect("policy lock");
        let previous = std::env::var_os("DEKA_SECURITY_POLICY");
        unsafe {
            std::env::set_var(
                "DEKA_SECURITY_POLICY",
                r#"{"security":{"allow":{"net":["127.0.0.1:9418"]}}}"#,
            );
        }

        assert!(super::super::security::enforce_net(Some("127.0.0.1:9418")).is_ok());
        assert!(super::super::security::enforce_net(Some("127.0.0.1:9419")).is_err());
        assert!(super::super::security::enforce_net(Some("127.0.0.1")).is_err());

        unsafe {
            match previous {
                Some(value) => std::env::set_var("DEKA_SECURITY_POLICY", value),
                None => std::env::remove_var("DEKA_SECURITY_POLICY"),
            }
        }
    }
}
