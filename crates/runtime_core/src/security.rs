#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Read,
    Write,
    Net,
    Env,
    Run,
    Db,
    Dynamic,
    Wasm,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationCapability {
    pub op_id: &'static str,
    pub capability: Capability,
    pub notes: &'static str,
}

// Single source of truth for operation -> capability classification.
// New runtime operations must be added here first.
pub const OPERATION_CAPABILITY_MATRIX: &[OperationCapability] = &[
    // Registered php host ops (crates/deka_host/src/modules/php/mod.rs).
    OperationCapability {
        op_id: "php.op_php_parse_phpx_types",
        capability: Capability::Unknown,
        notes: "Compiler/type parser op; no direct host side effects expected",
    },
    OperationCapability {
        op_id: "php.op_php_read_file_sync",
        capability: Capability::Read,
        notes: "Read file from host FS",
    },
    OperationCapability {
        op_id: "php.op_php_write_file_sync",
        capability: Capability::Write,
        notes: "Write file to host FS",
    },
    OperationCapability {
        op_id: "php.op_php_mkdirs",
        capability: Capability::Write,
        notes: "Create directories on host FS",
    },
    OperationCapability {
        op_id: "php.op_php_sha256",
        capability: Capability::Dynamic,
        notes: "Pure compute utility op (non-IO); kept distinct from read/write/net/db/run",
    },
    OperationCapability {
        op_id: "php.op_php_random_bytes",
        capability: Capability::Dynamic,
        notes: "Entropy/crypto utility op",
    },
    OperationCapability {
        op_id: "php.op_php_digest",
        capability: Capability::Dynamic,
        notes: "Pure hash (SHA-2/SHA-3/BLAKE3); not I/O",
    },
    OperationCapability {
        op_id: "php.op_php_hmac",
        capability: Capability::Dynamic,
        notes: "HMAC-SHA-2; not I/O",
    },
    OperationCapability {
        op_id: "php.op_php_secure_compare",
        capability: Capability::Dynamic,
        notes: "Constant-time equality; not I/O",
    },
    OperationCapability {
        op_id: "php.op_php_read_env",
        capability: Capability::Env,
        notes: "Read process environment map",
    },
    OperationCapability {
        op_id: "php.op_php_db_call_proto",
        capability: Capability::Db,
        notes: "Dispatch DB bridge request",
    },
    OperationCapability {
        op_id: "php.op_php_db_proto_encode",
        capability: Capability::Db,
        notes: "Encode DB bridge payload",
    },
    OperationCapability {
        op_id: "php.op_php_db_proto_decode",
        capability: Capability::Db,
        notes: "Decode DB bridge payload",
    },
    OperationCapability {
        op_id: "php.op_php_net_call_proto",
        capability: Capability::Net,
        notes: "Dispatch net bridge request",
    },
    OperationCapability {
        op_id: "php.op_php_net_proto_encode",
        capability: Capability::Net,
        notes: "Encode net bridge payload",
    },
    OperationCapability {
        op_id: "php.op_php_net_proto_decode",
        capability: Capability::Net,
        notes: "Decode net bridge payload",
    },
    OperationCapability {
        op_id: "php.op_php_fs_call_proto",
        capability: Capability::Unknown,
        notes: "Dispatch FS bridge request; child action decides read/write",
    },
    OperationCapability {
        op_id: "php.op_php_fs_call_proto_async",
        capability: Capability::Unknown,
        notes: "Dispatch FS bridge request without blocking the isolate (deka#578); child action decides read/write",
    },
    OperationCapability {
        op_id: "php.op_php_fs_proto_encode",
        capability: Capability::Unknown,
        notes: "Encode FS bridge payload",
    },
    OperationCapability {
        op_id: "php.op_php_fs_proto_decode",
        capability: Capability::Unknown,
        notes: "Decode FS bridge payload",
    },
    OperationCapability {
        op_id: "php.op_php_bridge_proto_stats",
        capability: Capability::Read,
        notes: "Read bridge stats/metrics",
    },
    OperationCapability {
        op_id: "php.op_php_cwd",
        capability: Capability::Read,
        notes: "Read current working directory",
    },
    OperationCapability {
        op_id: "php.op_php_canonicalize",
        capability: Capability::Read,
        notes: "Resolve symlinks to canonical path (used for confinement checks)",
    },
    OperationCapability {
        op_id: "php.op_php_file_exists",
        capability: Capability::Read,
        notes: "Check file existence",
    },
    OperationCapability {
        op_id: "php.op_php_path_resolve",
        capability: Capability::Read,
        notes: "Resolve path against base",
    },
    OperationCapability {
        op_id: "php.op_php_read_dir",
        capability: Capability::Read,
        notes: "List directory entries",
    },
    OperationCapability {
        op_id: "php.op_php_parse_wit",
        capability: Capability::Wasm,
        notes: "Parse WIT schema for wasm module interop",
    },
    // Filesystem bridge ops.
    OperationCapability {
        op_id: "bridge.fs.open",
        capability: Capability::Unknown,
        notes: "Mode-sensitive; classify as read/write at runtime from FsOpenRequest.mode",
    },
    OperationCapability {
        op_id: "bridge.fs.read",
        capability: Capability::Read,
        notes: "Read bytes from opened handle",
    },
    OperationCapability {
        op_id: "bridge.fs.write",
        capability: Capability::Write,
        notes: "Write bytes to opened handle",
    },
    OperationCapability {
        op_id: "bridge.fs.close",
        capability: Capability::Read,
        notes: "Handle cleanup only",
    },
    OperationCapability {
        op_id: "bridge.fs.read_file",
        capability: Capability::Read,
        notes: "Read full file path",
    },
    OperationCapability {
        op_id: "bridge.fs.write_file",
        capability: Capability::Write,
        notes: "Write full file path",
    },
    // Network bridge ops.
    OperationCapability {
        op_id: "bridge.net.connect",
        capability: Capability::Net,
        notes: "TCP outbound connect",
    },
    OperationCapability {
        op_id: "bridge.net.set_deadline",
        capability: Capability::Net,
        notes: "Socket deadline update",
    },
    OperationCapability {
        op_id: "bridge.net.read",
        capability: Capability::Net,
        notes: "Read from socket handle",
    },
    OperationCapability {
        op_id: "bridge.net.write",
        capability: Capability::Net,
        notes: "Write to socket handle",
    },
    OperationCapability {
        op_id: "bridge.net.tls_upgrade",
        capability: Capability::Net,
        notes: "Promote TCP socket to TLS",
    },
    OperationCapability {
        op_id: "bridge.net.close",
        capability: Capability::Net,
        notes: "Socket close",
    },
    // Database bridge ops. Engine-level allowlisting is a qualifier on this capability.
    OperationCapability {
        op_id: "bridge.db.open",
        capability: Capability::Db,
        notes: "Open DB handle; enforce engine qualifier postgres/mysql/sqlite",
    },
    OperationCapability {
        op_id: "bridge.db.query",
        capability: Capability::Db,
        notes: "Run query on DB handle",
    },
    OperationCapability {
        op_id: "bridge.db.query_one",
        capability: Capability::Db,
        notes: "Run single-row query on DB handle",
    },
    OperationCapability {
        op_id: "bridge.db.exec",
        capability: Capability::Db,
        notes: "Run exec statement on DB handle",
    },
    OperationCapability {
        op_id: "bridge.db.begin",
        capability: Capability::Db,
        notes: "Begin transaction",
    },
    OperationCapability {
        op_id: "bridge.db.commit",
        capability: Capability::Db,
        notes: "Commit transaction",
    },
    OperationCapability {
        op_id: "bridge.db.rollback",
        capability: Capability::Db,
        notes: "Rollback transaction",
    },
    OperationCapability {
        op_id: "bridge.db.close",
        capability: Capability::Db,
        notes: "Close DB handle",
    },
    OperationCapability {
        op_id: "bridge.db.stats",
        capability: Capability::Db,
        notes: "Read DB stats for active handles and metrics",
    },
    // Host/env operations used by php wasm bridge.
    // Dynamic execution.
    OperationCapability {
        op_id: "runtime.dynamic.eval",
        capability: Capability::Dynamic,
        notes: "String/code eval path",
    },
    OperationCapability {
        op_id: "runtime.dynamic.import",
        capability: Capability::Dynamic,
        notes: "Dynamic module import from runtime expression",
    },
    OperationCapability {
        op_id: "runtime.dynamic.fetch_exec",
        capability: Capability::Dynamic,
        notes: "Fetch and execute remote code",
    },
    // WASM loading/execution.
    OperationCapability {
        op_id: "runtime.wasm.import",
        capability: Capability::Wasm,
        notes: "import ... as wasm",
    },
    OperationCapability {
        op_id: "runtime.wasm.compile",
        capability: Capability::Wasm,
        notes: "WebAssembly.compile / module compilation",
    },
    OperationCapability {
        op_id: "runtime.wasm.instantiate",
        capability: Capability::Wasm,
        notes: "WebAssembly.instantiate",
    },
    OperationCapability {
        op_id: "runtime.wasm.host_import",
        capability: Capability::Wasm,
        notes: "Host capability surface exposed into wasm import object",
    },
    // Subprocess execution.
    OperationCapability {
        op_id: "runtime.process.spawn",
        capability: Capability::Run,
        notes: "Spawn subprocess",
    },
    OperationCapability {
        op_id: "runtime.process.exec",
        capability: Capability::Run,
        notes: "Exec subprocess",
    },
];

pub fn capability_for_operation(op_id: &str) -> Capability {
    OPERATION_CAPABILITY_MATRIX
        .iter()
        .find(|entry| entry.op_id == op_id)
        .map(|entry| entry.capability)
        .unwrap_or(Capability::Unknown)
}

pub const KNOWN_RUNTIME_OPERATION_IDS: &[&str] = &[
    // Registered php ops.
    "php.op_php_parse_phpx_types",
    "php.op_php_read_file_sync",
    "php.op_php_write_file_sync",
    "php.op_php_mkdirs",
    "php.op_php_sha256",
    "php.op_php_random_bytes",
    "php.op_php_digest",
    "php.op_php_hmac",
    "php.op_php_secure_compare",
    "php.op_php_read_env",
    "php.op_php_db_call_proto",
    "php.op_php_db_proto_encode",
    "php.op_php_db_proto_decode",
    "php.op_php_net_call_proto",
    "php.op_php_net_proto_encode",
    "php.op_php_net_proto_decode",
    "php.op_php_fs_call_proto",
    "php.op_php_fs_call_proto_async",
    "php.op_php_fs_proto_encode",
    "php.op_php_fs_proto_decode",
    "php.op_php_bridge_proto_stats",
    "php.op_php_cwd",
    "php.op_php_canonicalize",
    "php.op_php_file_exists",
    "php.op_php_path_resolve",
    "php.op_php_read_dir",
    "php.op_php_parse_wit",
    // Bridge action ops.
    "bridge.fs.open",
    "bridge.fs.read",
    "bridge.fs.write",
    "bridge.fs.close",
    "bridge.fs.read_file",
    "bridge.fs.write_file",
    "bridge.net.connect",
    "bridge.net.set_deadline",
    "bridge.net.read",
    "bridge.net.write",
    "bridge.net.tls_upgrade",
    "bridge.net.close",
    "bridge.db.open",
    "bridge.db.query",
    "bridge.db.query_one",
    "bridge.db.exec",
    "bridge.db.begin",
    "bridge.db.commit",
    "bridge.db.rollback",
    "bridge.db.close",
    "bridge.db.stats",
    // Runtime guard points.
    "runtime.dynamic.eval",
    "runtime.dynamic.import",
    "runtime.dynamic.fetch_exec",
    "runtime.wasm.import",
    "runtime.wasm.compile",
    "runtime.wasm.instantiate",
    "runtime.wasm.host_import",
    "runtime.process.spawn",
    "runtime.process.exec",
];

#[cfg(test)]
mod tests {
    use super::{
        Capability, KNOWN_RUNTIME_OPERATION_IDS, OPERATION_CAPABILITY_MATRIX,
        capability_for_operation,
    };
    use std::collections::HashSet;

    #[test]
    fn unknown_when_not_registered() {
        assert_eq!(
            capability_for_operation("runtime.unmapped.operation"),
            Capability::Unknown
        );
    }

    #[test]
    fn matrix_has_unique_operation_ids() {
        let mut seen = HashSet::new();
        for entry in OPERATION_CAPABILITY_MATRIX {
            assert!(
                seen.insert(entry.op_id),
                "duplicate operation id in capability matrix: {}",
                entry.op_id
            );
        }
    }

    #[test]
    fn known_ops_are_present_in_matrix() {
        let mapped: HashSet<&str> = OPERATION_CAPABILITY_MATRIX
            .iter()
            .map(|e| e.op_id)
            .collect();
        for op_id in KNOWN_RUNTIME_OPERATION_IDS {
            assert!(
                mapped.contains(op_id),
                "known operation id is not mapped in matrix: {}",
                op_id
            );
        }
    }
}
