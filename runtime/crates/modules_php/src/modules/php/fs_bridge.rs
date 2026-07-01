use super::bridge_metrics::record_bridge_proto_metric;
use super::security::{enforce_read, enforce_write};
use super::*;

pub(super) struct FsState {
    next_handle: u64,
    handles: HashMap<u64, StdFile>,
}

impl FsState {
    fn new() -> Self {
        Self {
            next_handle: 1,
            handles: HashMap::new(),
        }
    }
}

static FS_STATE: OnceLock<Mutex<FsState>> = OnceLock::new();

pub(super) fn fs_state() -> &'static Mutex<FsState> {
    FS_STATE.get_or_init(|| Mutex::new(FsState::new()))
}

pub(super) fn fs_call_impl(
    action: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let err = |msg: String| {
        deno_core::error::CoreError::from(std::io::Error::new(std::io::ErrorKind::Other, msg))
    };

    let to_bytes = |value: Option<&serde_json::Value>| -> Vec<u8> {
        let Some(value) = value else {
            return Vec::new();
        };
        if let Some(arr) = value.as_array() {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                let byte = item.as_u64().unwrap_or(0).min(255) as u8;
                out.push(byte);
            }
            return out;
        }
        if let Some(s) = value.as_str() {
            return s.as_bytes().to_vec();
        }
        Vec::new()
    };

    let args_obj = args.as_object().cloned().unwrap_or_default();
    match action.as_str() {
        "open" => {
            let path = args_obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if path.is_empty() {
                return Ok(serde_json::json!({ "ok": false, "error": "open: missing path" }));
            }

            let mode = args_obj.get("mode").and_then(|v| v.as_str()).unwrap_or("r");
            let mut opts = OpenOptions::new();
            let read = mode.contains('r') || mode.contains('+');
            let write = mode.contains('w')
                || mode.contains('a')
                || mode.contains('x')
                || mode.contains('c')
                || mode.contains('+');
            let append = mode.contains('a');
            let truncate = mode.contains('w');
            let create = mode.contains('w')
                || mode.contains('a')
                || mode.contains('x')
                || mode.contains('c');
            let create_new = mode.contains('x');

            opts.read(read)
                .write(write)
                .append(append)
                .truncate(truncate)
                .create(create)
                .create_new(create_new);

            let file = match opts.open(&path) {
                Ok(file) => file,
                Err(e) => {
                    return Ok(serde_json::json!({
                        "ok": false,
                        "error": format!("open: {}", e)
                    }));
                }
            };

            let mut state = fs_state()
                .lock()
                .map_err(|_| err("fs lock poisoned".to_string()))?;
            let handle = state.next_handle;
            state.next_handle += 1;
            state.handles.insert(handle, file);
            Ok(serde_json::json!({ "ok": true, "handle": handle }))
        }
        "read" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("read: missing handle".to_string()))?;
            let max_bytes = args_obj
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(65536) as usize;
            let mut buf = vec![0_u8; max_bytes.max(1)];

            let mut state = fs_state()
                .lock()
                .map_err(|_| err("fs lock poisoned".to_string()))?;
            let Some(file) = state.handles.get_mut(&handle) else {
                return Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("read: unknown handle {}", handle)
                }));
            };

            match file.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    Ok(serde_json::json!({
                        "ok": true,
                        "data": buf,
                        "eof": n == 0
                    }))
                }
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("read: {}", e)
                })),
            }
        }
        "write" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("write: missing handle".to_string()))?;
            let data = to_bytes(args_obj.get("data"));

            let mut state = fs_state()
                .lock()
                .map_err(|_| err("fs lock poisoned".to_string()))?;
            let Some(file) = state.handles.get_mut(&handle) else {
                return Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("write: unknown handle {}", handle)
                }));
            };

            match file.write_all(&data) {
                Ok(()) => Ok(serde_json::json!({ "ok": true, "written": data.len() })),
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("write: {}", e)
                })),
            }
        }
        "close" => {
            let handle = args_obj
                .get("handle")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| err("close: missing handle".to_string()))?;
            let mut state = fs_state()
                .lock()
                .map_err(|_| err("fs lock poisoned".to_string()))?;
            state.handles.remove(&handle);
            Ok(serde_json::json!({ "ok": true }))
        }
        "read_file" => {
            let path = args_obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if path.is_empty() {
                return Ok(serde_json::json!({ "ok": false, "error": "read_file: missing path" }));
            }
            match std::fs::read(&path) {
                Ok(data) => Ok(serde_json::json!({ "ok": true, "data": data })),
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("read_file: {}", e)
                })),
            }
        }
        "write_file" => {
            let path = args_obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if path.is_empty() {
                return Ok(serde_json::json!({ "ok": false, "error": "write_file: missing path" }));
            }
            let data = to_bytes(args_obj.get("data"));
            match std::fs::write(&path, &data) {
                Ok(()) => Ok(serde_json::json!({ "ok": true, "written": data.len() })),
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("write_file: {}", e)
                })),
            }
        }
        "read_dir" => {
            let path = args_obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if path.is_empty() {
                return Ok(serde_json::json!({ "ok": false, "error": "read_dir: missing path" }));
            }
            let entries = std::fs::read_dir(&path).map_err(|e| {
                deno_core::error::CoreError::from(std::io::Error::new(
                    e.kind(),
                    format!("read_dir: {}", e),
                ))
            })?;
            let mut out = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|e| {
                    deno_core::error::CoreError::from(std::io::Error::new(
                        e.kind(),
                        format!("read_dir: {}", e),
                    ))
                })?;
                let file_type = entry.file_type().map_err(|e| {
                    deno_core::error::CoreError::from(std::io::Error::new(
                        e.kind(),
                        format!("read_dir: {}", e),
                    ))
                })?;
                out.push(serde_json::json!({
                    "name": entry.file_name().to_string_lossy().to_string(),
                    "is_dir": file_type.is_dir(),
                    "is_file": file_type.is_file(),
                }));
            }
            Ok(serde_json::json!({ "ok": true, "entries": out }))
        }
        "mkdirs" => {
            let path = args_obj
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('\0')
                .to_string();
            if path.is_empty() {
                return Ok(serde_json::json!({ "ok": false, "error": "mkdirs: missing path" }));
            }
            match std::fs::create_dir_all(&path) {
                Ok(()) => Ok(serde_json::json!({ "ok": true })),
                Err(e) => Ok(serde_json::json!({
                    "ok": false,
                    "error": format!("mkdirs: {}", e)
                })),
            }
        }
        _ => Ok(serde_json::json!({
            "ok": false,
            "error": format!("unknown fs action '{}'", action)
        })),
    }
}

#[derive(Clone, Copy)]
pub(super) enum FsProtoActionKind {
    Open,
    Read,
    Write,
    Close,
    ReadFile,
    WriteFile,
    ReadDir,
    Mkdirs,
}

pub(super) fn fs_action_payload_to_proto_request(
    action: &str,
    payload: &serde_json::Value,
) -> Result<proto::bridge_v1::FsRequest, deno_core::error::CoreError> {
    use proto::bridge_v1::fs_request::Action;
    let args = payload.as_object().cloned().unwrap_or_default();
    let action = match action {
        "open" => Action::Open(proto::bridge_v1::FsOpenRequest {
            path: args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            mode: args
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("r")
                .to_string(),
        }),
        "read" => Action::Read(proto::bridge_v1::FsReadRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
            max_bytes: args
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(65536),
        }),
        "write" => {
            let data = args
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|x| x.as_u64().unwrap_or(0).min(255) as u8)
                        .collect::<Vec<u8>>()
                })
                .or_else(|| {
                    args.get("data")
                        .and_then(|v| v.as_str())
                        .map(|s| s.as_bytes().to_vec())
                })
                .unwrap_or_default();
            Action::Write(proto::bridge_v1::FsWriteRequest {
                handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
                data,
            })
        }
        "close" => Action::Close(proto::bridge_v1::FsCloseRequest {
            handle: args.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        }),
        "read_file" => Action::ReadFile(proto::bridge_v1::FsReadFileRequest {
            path: args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "write_file" => {
            let data = args
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|x| x.as_u64().unwrap_or(0).min(255) as u8)
                        .collect::<Vec<u8>>()
                })
                .or_else(|| {
                    args.get("data")
                        .and_then(|v| v.as_str())
                        .map(|s| s.as_bytes().to_vec())
                })
                .unwrap_or_default();
            Action::WriteFile(proto::bridge_v1::FsWriteFileRequest {
                path: args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                data,
            })
        }
        "read_dir" => Action::ReadDir(proto::bridge_v1::FsReadDirRequest {
            path: args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            with_types: args
                .get("with_types")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        }),
        "mkdirs" => Action::Mkdirs(proto::bridge_v1::FsMkdirsRequest {
            path: args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        other => return Err(core_err(format!("unsupported fs proto action '{}'", other))),
    };
    Ok(proto::bridge_v1::FsRequest {
        schema_version: 1,
        action: Some(action),
    })
}

pub(super) fn fs_proto_request_to_action_payload(
    req: &proto::bridge_v1::FsRequest,
) -> Result<(String, serde_json::Value, FsProtoActionKind), deno_core::error::CoreError> {
    use proto::bridge_v1::fs_request::Action;
    let Some(action) = req.action.as_ref() else {
        return Err(core_err("fs proto request missing action"));
    };
    match action {
        Action::Open(open) => Ok((
            "open".to_string(),
            serde_json::json!({ "path": open.path, "mode": open.mode }),
            FsProtoActionKind::Open,
        )),
        Action::Read(read) => Ok((
            "read".to_string(),
            serde_json::json!({ "handle": read.handle, "max_bytes": read.max_bytes }),
            FsProtoActionKind::Read,
        )),
        Action::Write(write) => Ok((
            "write".to_string(),
            serde_json::json!({
                "handle": write.handle,
                "data": write.data.iter().map(|b| serde_json::Value::Number((*b as u64).into())).collect::<Vec<_>>()
            }),
            FsProtoActionKind::Write,
        )),
        Action::Close(close) => Ok((
            "close".to_string(),
            serde_json::json!({ "handle": close.handle }),
            FsProtoActionKind::Close,
        )),
        Action::ReadFile(read_file) => Ok((
            "read_file".to_string(),
            serde_json::json!({ "path": read_file.path }),
            FsProtoActionKind::ReadFile,
        )),
        Action::WriteFile(write_file) => Ok((
            "write_file".to_string(),
            serde_json::json!({
                "path": write_file.path,
                "data": write_file.data.iter().map(|b| serde_json::Value::Number((*b as u64).into())).collect::<Vec<_>>()
            }),
            FsProtoActionKind::WriteFile,
        )),
        Action::ReadDir(read_dir) => Ok((
            "read_dir".to_string(),
            serde_json::json!({ "path": read_dir.path, "with_types": read_dir.with_types }),
            FsProtoActionKind::ReadDir,
        )),
        Action::Mkdirs(mkdirs) => Ok((
            "mkdirs".to_string(),
            serde_json::json!({ "path": mkdirs.path }),
            FsProtoActionKind::Mkdirs,
        )),
    }
}

pub(super) fn fs_json_response_to_proto(
    resp: &serde_json::Value,
    kind: FsProtoActionKind,
) -> proto::bridge_v1::FsResponse {
    use proto::bridge_v1::fs_response::Action;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let error = resp
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let action = match kind {
        FsProtoActionKind::Open => Some(Action::Open(proto::bridge_v1::FsOpenResponse {
            handle: resp.get("handle").and_then(|v| v.as_u64()).unwrap_or(0),
        })),
        FsProtoActionKind::Read => {
            let data = resp
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|x| x.as_u64().unwrap_or(0).min(255) as u8)
                        .collect::<Vec<u8>>()
                })
                .unwrap_or_default();
            Some(Action::Read(proto::bridge_v1::FsReadResponse {
                data,
                eof: resp.get("eof").and_then(|v| v.as_bool()).unwrap_or(false),
            }))
        }
        FsProtoActionKind::Write => Some(Action::Write(proto::bridge_v1::FsWriteResponse {
            written: resp.get("written").and_then(|v| v.as_u64()).unwrap_or(0),
        })),
        FsProtoActionKind::Close => Some(Action::Close(proto::bridge_v1::FsUnitResponse { ok })),
        FsProtoActionKind::ReadFile => {
            let data = resp
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|x| x.as_u64().unwrap_or(0).min(255) as u8)
                        .collect::<Vec<u8>>()
                })
                .unwrap_or_default();
            Some(Action::ReadFile(proto::bridge_v1::FsReadResponse {
                data,
                eof: true,
            }))
        }
        FsProtoActionKind::WriteFile => {
            Some(Action::WriteFile(proto::bridge_v1::FsWriteResponse {
                written: resp.get("written").and_then(|v| v.as_u64()).unwrap_or(0),
            }))
        }
        FsProtoActionKind::ReadDir => {
            let entries = resp
                .get("entries")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|entry| entry.as_object())
                        .map(|entry| proto::bridge_v1::FsDirEntry {
                            name: entry
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            is_dir: entry
                                .get("is_dir")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                            is_file: entry
                                .get("is_file")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some(Action::ReadDir(proto::bridge_v1::FsReadDirResponse {
                entries,
            }))
        }
        FsProtoActionKind::Mkdirs => Some(Action::Mkdirs(proto::bridge_v1::FsUnitResponse { ok })),
    };
    proto::bridge_v1::FsResponse {
        schema_version: 1,
        ok,
        error,
        action,
    }
}

pub(super) fn fs_proto_response_to_json(resp: &proto::bridge_v1::FsResponse) -> serde_json::Value {
    use proto::bridge_v1::fs_response::Action;
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
            Action::Open(open) => {
                out.insert(
                    "handle".to_string(),
                    serde_json::Value::Number(open.handle.into()),
                );
            }
            Action::Read(read) | Action::ReadFile(read) => {
                out.insert(
                    "data".to_string(),
                    serde_json::Value::Array(
                        read.data
                            .iter()
                            .map(|b| serde_json::Value::Number((*b as u64).into()))
                            .collect(),
                    ),
                );
                out.insert("eof".to_string(), serde_json::Value::Bool(read.eof));
            }
            Action::Write(write) | Action::WriteFile(write) => {
                out.insert(
                    "written".to_string(),
                    serde_json::Value::Number(write.written.into()),
                );
            }
            Action::Close(unit) => {
                out.insert("ok".to_string(), serde_json::Value::Bool(unit.ok));
            }
            Action::ReadDir(read_dir) => {
                out.insert(
                    "entries".to_string(),
                    serde_json::Value::Array(
                        read_dir
                            .entries
                            .iter()
                            .map(|entry| {
                                serde_json::json!({
                                    "name": entry.name,
                                    "is_dir": entry.is_dir,
                                    "is_file": entry.is_file,
                                })
                            })
                            .collect(),
                    ),
                );
            }
            Action::Mkdirs(unit) => {
                out.insert("ok".to_string(), serde_json::Value::Bool(unit.ok));
            }
        }
    }
    serde_json::Value::Object(out)
}

pub(super) fn fs_call_proto_impl(request: &[u8]) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let started = Instant::now();
    let req = proto::bridge_v1::FsRequest::decode(request)
        .map_err(|e| core_err(format!("fs proto decode failed: {}", e)))?;
    let (action, payload, kind) = fs_proto_request_to_action_payload(&req)?;
    let fs_target = payload.get("path").and_then(|v| v.as_str()).or(Some("*"));
    match action.as_str() {
        "read" | "read_file" => enforce_read(fs_target)?,
        "write" | "write_file" | "mkdirs" => enforce_write(fs_target)?,
        "read_dir" => enforce_read(fs_target)?,
        "open" => {
            let mode = payload.get("mode").and_then(|v| v.as_str()).unwrap_or("r");
            if mode.contains('w')
                || mode.contains('a')
                || mode.contains('x')
                || mode.contains('c')
                || mode.contains('+')
            {
                enforce_write(fs_target)?;
            } else {
                enforce_read(fs_target)?;
            }
        }
        _ => {}
    }
    let response_json = fs_call_impl(action, payload)?;
    let response = fs_json_response_to_proto(&response_json, kind);
    let out = response.encode_to_vec();
    record_bridge_proto_metric(
        "fs",
        request.len(),
        out.len(),
        started.elapsed().as_micros() as u64,
    );
    Ok(out)
}

#[op2]
#[buffer]
pub(super) fn op_php_fs_call_proto(
    #[buffer] request: &[u8],
) -> Result<Vec<u8>, deno_core::error::CoreError> {
    fs_call_proto_impl(request)
}

#[op2]
#[buffer]
pub(super) fn op_php_fs_proto_encode(
    #[string] action: String,
    #[serde] payload: serde_json::Value,
) -> Result<Vec<u8>, deno_core::error::CoreError> {
    let request = fs_action_payload_to_proto_request(&action, &payload)?;
    Ok(request.encode_to_vec())
}

#[op2]
#[serde]
pub(super) fn op_php_fs_proto_decode(
    #[buffer] response: &[u8],
) -> Result<serde_json::Value, deno_core::error::CoreError> {
    let decoded = proto::bridge_v1::FsResponse::decode(response)
        .map_err(|e| core_err(format!("fs proto decode response failed: {}", e)))?;
    Ok(fs_proto_response_to_json(&decoded))
}
