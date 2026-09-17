//! RFD 27 host bridge — the authoritative operation catalog, host-grant
//! types + resolution rules, and the shared `PermissionDenied` error type.
//!
//! This module is the single enforcement boundary for `bridge kind.action(args)`
//! in DekaScript. The external `dsc` compiler (pinned in `scripts/dsc-version`,
//! currently 0.8.1) performs **no** catalog or grant validation: it typechecks
//! unknown kinds/actions as generic sync `Result<infer, infer>` and emits
//! `__deka_to_result(__deka_host(kind, action, args))` for them. Every runtime
//! gate decision therefore comes from [`HOST_CATALOG`] here, never from a
//! hand-maintained JS list (deka#620 drift).
//!
//! Contract (RFD 27):
//!
//! - The catalog is authoritative. [`HostAction::r#async`] MUST match the
//!   pinned dsc exactly: precisely `fs.{read_file,write_file,read_dir,mkdirs}`
//!   are async; their `_sync` twins and every other action are sync. dsc emits
//!   `__deka_host(kind, action, args).then(__deka_to_result)` for async
//!   entries and `__deka_to_result(__deka_host(kind, action, args))` for sync
//!   ones; a flag mismatch means the DS-level `await` resolves the wrong
//!   wrapper and the call breaks at runtime.
//! - Bytes on the wire are a `Uint8Array` on the DS surface and a
//!   `#[buffer]` zero-copy value at the Rust op boundary — never
//!   `Array<number>`.
//! - A denied call returns `Err(PermissionDenied { capability, target })`,
//!   encoded with [`PERMISSION_DENIED_MARKER`] so it survives the CoreError /
//!   thrown-message boundary. It is never a throw into DekaScript.
//!
//! Kinds with no host implementation yet (vault) are
//! deliberately absent — the catalog lists only genuinely host-implemented
//! actions, zero stubs.

use std::fmt::Write;

// ---- Operation catalog -----------------------------------------------------

/// Wire representation of one bridge argument / result value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireType {
    /// UTF-8 string.
    Str,
    /// `f64` / integer number.
    Num,
    /// Boolean.
    Bool,
    /// Byte payload: `Uint8Array` on the DS surface, `#[buffer]` at the op.
    Bytes,
    /// Opaque `u64` resource id (connection/file/db handle).
    Handle,
    /// Free-form JSON value (query params, stats objects, ...).
    Json,
}

/// Where an action can be served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hosts {
    /// Implemented by the native host only.
    NativeOnly,
    /// Implemented natively and in the browser shim (WebCrypto / timers).
    NativeAndBrowser,
}

/// Shape of the success value returned to DekaScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultShape {
    Bytes,
    Num,
    Bool,
    Unit,
    Handle,
    /// Directory listing (array of entry names).
    Entries,
    Json,
}

/// One positional argument of a host action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostArg {
    pub name: &'static str,
    pub wire: WireType,
}

const fn arg(name: &'static str, wire: WireType) -> HostArg {
    HostArg { name, wire }
}

/// A single host operation: `bridge <kind>.<name>(args...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostAction {
    pub name: &'static str,
    pub args: &'static [HostArg],
    pub result: ResultShape,
    /// Whether dsc compiles the call with `await` + `.then(__deka_to_result)`.
    /// Exactly `fs.{read_file,write_file,read_dir,mkdirs}` are async; ALL
    /// other actions are sync (dsc 0.8.1 contract — a flag change needs a
    /// dsc catalog PR first).
    pub r#async: bool,
    /// I/O capability this action exercises: `"read"`, `"write"`, `"net"`,
    /// or `"db"`. `None` = not I/O (crypto / time / concurrency) and never
    /// permission-denied. `fs.open` is catalogued as `"read"`; the Rust op
    /// already splits read/write handles internally.
    pub capability: Option<&'static str>,
    pub hosts: Hosts,
}

/// A bridge kind and its host-implemented actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostKind {
    pub name: &'static str,
    /// Grant-owner package (`"@deka/crypto"`), matched against manifest and
    /// grant-table entries.
    pub grant_owner: &'static str,
    pub actions: &'static [HostAction],
}

const CRYPTO_ACTIONS: &[HostAction] = &[
    HostAction {
        name: "random_bytes",
        args: &[arg("len", WireType::Num)],
        result: ResultShape::Bytes,
        r#async: false,
        capability: None,
        hosts: Hosts::NativeAndBrowser,
    },
    HostAction {
        name: "digest",
        args: &[
            arg("algorithm", WireType::Str),
            arg("data", WireType::Bytes),
        ],
        result: ResultShape::Bytes,
        r#async: false,
        capability: None,
        hosts: Hosts::NativeAndBrowser,
    },
    HostAction {
        name: "hmac",
        args: &[
            arg("algorithm", WireType::Str),
            arg("key", WireType::Bytes),
            arg("data", WireType::Bytes),
        ],
        result: ResultShape::Bytes,
        r#async: false,
        capability: None,
        hosts: Hosts::NativeAndBrowser,
    },
    HostAction {
        name: "secure_compare",
        args: &[arg("a", WireType::Bytes), arg("b", WireType::Bytes)],
        result: ResultShape::Bool,
        r#async: false,
        capability: None,
        hosts: Hosts::NativeAndBrowser,
    },
    HostAction {
        name: "aes_256_gcm_encrypt",
        args: &[
            arg("key", WireType::Bytes),
            arg("nonce", WireType::Bytes),
            arg("plaintext", WireType::Bytes),
            arg("aad", WireType::Bytes),
        ],
        result: ResultShape::Bytes,
        r#async: false,
        capability: None,
        hosts: Hosts::NativeAndBrowser,
    },
    HostAction {
        name: "aes_256_gcm_decrypt",
        args: &[
            arg("key", WireType::Bytes),
            arg("nonce", WireType::Bytes),
            arg("ciphertext", WireType::Bytes),
            arg("aad", WireType::Bytes),
        ],
        result: ResultShape::Bytes,
        r#async: false,
        capability: None,
        hosts: Hosts::NativeAndBrowser,
    },
    // bcrypt has no WebCrypto equivalent; native host only.
    HostAction {
        name: "bcrypt_verify",
        args: &[arg("password", WireType::Str), arg("hash", WireType::Str)],
        result: ResultShape::Bool,
        r#async: false,
        capability: None,
        hosts: Hosts::NativeOnly,
    },
];

const FS_ACTIONS: &[HostAction] = &[
    HostAction {
        name: "read_file",
        args: &[arg("path", WireType::Str)],
        result: ResultShape::Bytes,
        r#async: true,
        capability: Some("read"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "write_file",
        args: &[arg("path", WireType::Str), arg("data", WireType::Bytes)],
        result: ResultShape::Num,
        r#async: true,
        capability: Some("write"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "read_dir",
        args: &[arg("path", WireType::Str)],
        result: ResultShape::Entries,
        r#async: true,
        capability: Some("read"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "mkdirs",
        args: &[arg("path", WireType::Str)],
        result: ResultShape::Unit,
        r#async: true,
        capability: Some("write"),
        hosts: Hosts::NativeOnly,
    },
    // These are distinct catalog actions, not package-level Promise shims.
    // The bootstrap routes them to the synchronous implementation of the same
    // host operation (deka#758).
    HostAction {
        name: "read_file_sync",
        args: &[arg("path", WireType::Str)],
        result: ResultShape::Bytes,
        r#async: false,
        capability: Some("read"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "write_file_sync",
        args: &[arg("path", WireType::Str), arg("data", WireType::Bytes)],
        result: ResultShape::Num,
        r#async: false,
        capability: Some("write"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "read_dir_sync",
        args: &[arg("path", WireType::Str)],
        result: ResultShape::Entries,
        r#async: false,
        capability: Some("read"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "mkdirs_sync",
        args: &[arg("path", WireType::Str)],
        result: ResultShape::Unit,
        r#async: false,
        capability: Some("write"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "open",
        args: &[arg("path", WireType::Str), arg("write", WireType::Bool)],
        result: ResultShape::Handle,
        r#async: false,
        capability: Some("read"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "read",
        args: &[
            arg("handle", WireType::Handle),
            arg("max_bytes", WireType::Num),
        ],
        result: ResultShape::Bytes,
        r#async: false,
        capability: Some("read"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "write",
        args: &[
            arg("handle", WireType::Handle),
            arg("data", WireType::Bytes),
        ],
        result: ResultShape::Num,
        r#async: false,
        capability: Some("write"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "close",
        args: &[arg("handle", WireType::Handle)],
        result: ResultShape::Unit,
        r#async: false,
        capability: None,
        hosts: Hosts::NativeOnly,
    },
];

const NET_ACTIONS: &[HostAction] = &[
    HostAction {
        name: "connect",
        args: &[arg("host", WireType::Str), arg("port", WireType::Num)],
        result: ResultShape::Handle,
        r#async: false,
        capability: Some("net"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "listen",
        args: &[arg("host", WireType::Str), arg("port", WireType::Num)],
        result: ResultShape::Handle,
        r#async: false,
        capability: Some("net"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "accept",
        args: &[arg("handle", WireType::Handle)],
        result: ResultShape::Handle,
        r#async: false,
        capability: Some("net"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "read",
        args: &[
            arg("handle", WireType::Handle),
            arg("max_bytes", WireType::Num),
        ],
        result: ResultShape::Bytes,
        r#async: false,
        capability: Some("net"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "write",
        args: &[
            arg("handle", WireType::Handle),
            arg("data", WireType::Bytes),
        ],
        result: ResultShape::Num,
        r#async: false,
        capability: Some("net"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "close",
        args: &[arg("handle", WireType::Handle)],
        result: ResultShape::Unit,
        r#async: false,
        capability: Some("net"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "set_deadline",
        args: &[
            arg("handle", WireType::Handle),
            arg("millis", WireType::Num),
        ],
        result: ResultShape::Unit,
        r#async: false,
        capability: Some("net"),
        hosts: Hosts::NativeOnly,
    },
];

const TLS_ACTIONS: &[HostAction] = &[HostAction {
    name: "upgrade",
    args: &[
        arg("handle", WireType::Handle),
        arg("server_name", WireType::Str),
    ],
    result: ResultShape::Handle,
    r#async: false,
    capability: Some("net"),
    hosts: Hosts::NativeOnly,
}];

const TIME_ACTIONS: &[HostAction] = &[HostAction {
    name: "sleep_ms",
    args: &[arg("milliseconds", WireType::Num)],
    result: ResultShape::Num,
    r#async: false,
    capability: None,
    hosts: Hosts::NativeAndBrowser,
}];

const CONCURRENCY_ACTIONS: &[HostAction] = &[
    HostAction {
        name: "lock_acquire",
        args: &[arg("name", WireType::Str), arg("timeout_ms", WireType::Num)],
        result: ResultShape::Num,
        r#async: false,
        capability: None,
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "lock_release",
        args: &[arg("token", WireType::Num)],
        result: ResultShape::Unit,
        r#async: false,
        capability: None,
        hosts: Hosts::NativeOnly,
    },
];

const DB_ACTIONS: &[HostAction] = &[
    HostAction {
        name: "open",
        args: &[arg("url", WireType::Str)],
        result: ResultShape::Handle,
        r#async: false,
        capability: Some("db"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "query",
        args: &[
            arg("handle", WireType::Handle),
            arg("sql", WireType::Str),
            arg("params", WireType::Json),
        ],
        result: ResultShape::Entries,
        r#async: false,
        capability: Some("db"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "exec",
        args: &[
            arg("handle", WireType::Handle),
            arg("sql", WireType::Str),
            arg("params", WireType::Json),
        ],
        result: ResultShape::Num,
        r#async: false,
        capability: Some("db"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "close",
        args: &[arg("handle", WireType::Handle)],
        result: ResultShape::Unit,
        r#async: false,
        capability: Some("db"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "stats",
        args: &[arg("handle", WireType::Handle)],
        result: ResultShape::Json,
        r#async: false,
        capability: Some("db"),
        hosts: Hosts::NativeOnly,
    },
];

/// The single authoritative bridge catalog (RFD 27). Every entry is genuinely
/// host-implemented; kinds with no DS action surface yet (vault) are absent,
/// not stubbed.
pub const HOST_CATALOG: &[HostKind] = &[
    HostKind {
        name: "crypto",
        grant_owner: "@deka/crypto",
        actions: CRYPTO_ACTIONS,
    },
    HostKind {
        name: "fs",
        grant_owner: "@deka/fs",
        actions: FS_ACTIONS,
    },
    HostKind {
        name: "net",
        grant_owner: "@deka/tcp",
        actions: NET_ACTIONS,
    },
    HostKind {
        name: "tls",
        grant_owner: "@deka/tls",
        actions: TLS_ACTIONS,
    },
    HostKind {
        name: "time",
        grant_owner: "@deka/time",
        actions: TIME_ACTIONS,
    },
    HostKind {
        name: "db",
        grant_owner: "@deka/db",
        actions: DB_ACTIONS,
    },
];

/// Concurrency (`lock_acquire` / `lock_release`) is deliberately NOT in the
/// DS-facing catalog: the Rust ops are genuinely async (they park with a
/// tokio timeout), while the pinned dsc types every action it does not know
/// as sync. Serving them on the sync emit contract would either block the
/// isolate thread or mis-tag a Promise as `Ok`. They remain available on the
/// PHPX compatibility path; exposing them to DekaScript needs a dsc catalog
/// entry typed async, and is a runtime PR per RFD 27.
pub const PHPX_ONLY_ACTIONS: &[HostKind] = &[HostKind {
    name: "concurrency",
    grant_owner: "@deka/concurrency",
    actions: CONCURRENCY_ACTIONS,
}];

/// Look up a kind by name.
pub fn find_kind(name: &str) -> Option<&'static HostKind> {
    HOST_CATALOG.iter().find(|kind| kind.name == name)
}

/// Look up an action by kind and action name.
pub fn find_action(kind: &str, action: &str) -> Option<&'static HostAction> {
    find_kind(kind)?
        .actions
        .iter()
        .find(|candidate| candidate.name == action)
}

/// JSON gate payload for the JS bootstrap (`{kind: {action: {async: bool}}}`),
/// covering exactly the actions the pinned dsc can emit — i.e. the whole
/// catalog, since dsc performs no name validation. The isolate worker uses
/// this to replace its hand-maintained `DS_HOST_CATALOG` (deka#620).
pub fn js_catalog_json() -> String {
    let mut kinds = serde_json::Map::new();
    for kind in HOST_CATALOG {
        let mut actions = serde_json::Map::new();
        for action in kind.actions {
            actions.insert(
                action.name.to_string(),
                serde_json::json!({ "async": action.r#async }),
            );
        }
        kinds.insert(kind.name.to_string(), serde_json::Value::Object(actions));
    }
    serde_json::Value::Object(kinds).to_string()
}

/// Machine-readable dump of the FULL catalog for external tooling
/// (deka#620: diffing published packages' hand-maintained bridge declarations
/// against this catalog — see `bridge_diff` and `bridge_decl`). Unlike
/// [`js_catalog_json`] (the minimal gate payload for the isolate bootstrap),
/// this includes grant owners, argument names + wire types, and result shapes,
/// so a declaration diff can be derived from the same source of truth instead
/// of a hand-copied signature list.
pub fn catalog_json() -> String {
    fn wire_json(wire: WireType) -> serde_json::Value {
        match wire {
            WireType::Str => serde_json::json!("string"),
            WireType::Num => serde_json::json!("number"),
            WireType::Bool => serde_json::json!("boolean"),
            WireType::Bytes => serde_json::json!("bytes"),
            WireType::Handle => serde_json::json!("number"),
            WireType::Json => serde_json::json!("json"),
        }
    }
    fn shape_json(shape: ResultShape) -> serde_json::Value {
        match shape {
            ResultShape::Bytes => serde_json::json!("bytes"),
            ResultShape::Num => serde_json::json!("number"),
            ResultShape::Bool => serde_json::json!("boolean"),
            ResultShape::Unit => serde_json::json!("unit"),
            ResultShape::Handle => serde_json::json!("number"),
            ResultShape::Entries => serde_json::json!("entries"),
            ResultShape::Json => serde_json::json!("json"),
        }
    }
    let mut kinds = serde_json::Map::new();
    for kind in HOST_CATALOG {
        let mut actions = serde_json::Map::new();
        for action in kind.actions {
            let args: Vec<serde_json::Value> = action
                .args
                .iter()
                .map(|host_arg| {
                    serde_json::json!({"name": host_arg.name, "wire": wire_json(host_arg.wire)})
                })
                .collect();
            actions.insert(
                action.name.to_string(),
                serde_json::json!({
                    "args": args,
                    "result": shape_json(action.result),
                    "async": action.r#async,
                }),
            );
        }
        kinds.insert(
            kind.name.to_string(),
            serde_json::json!({
                "grant_owner": kind.grant_owner,
                "actions": actions,
            }),
        );
    }
    serde_json::Value::Object(kinds).to_string()
}

// ---- Declaration-file generation (rfd#27 2026-09-16 amendment) -------------

/// Wire type → DekaScript surface type. This is the single mapping shared by
/// the generated `deka-host.d.ds` and the bridge-declaration checker
/// (`bridge_decl`, deka#620 / deka#1098); keep it in one place so the catalog
/// and the checked declarations can never drift apart.
pub fn wire_type_to_ds(wire: WireType) -> &'static str {
    match wire {
        WireType::Str => "string",
        WireType::Num => "number",
        WireType::Bool => "boolean",
        WireType::Bytes => "bytes",
        // Handles are opaque u64 resource ids on the DS surface.
        WireType::Handle => "number",
        // Free-form JSON values are `JsValue` in declaration-space.
        WireType::Json => "JsValue",
    }
}

/// Result shape → DekaScript `Result<Ok, string>` Ok type. Host errors are
/// always `string` in the declaration file; stdlib packages wrap them.
pub fn result_shape_to_ds(shape: ResultShape) -> &'static str {
    match shape {
        ResultShape::Bytes => "bytes",
        ResultShape::Num => "number",
        ResultShape::Bool => "boolean",
        // Unit host results have no value on the DS side.
        ResultShape::Unit => "void",
        ResultShape::Handle => "number",
        // Directory listings are arrays of entry names.
        ResultShape::Entries => "Array<string>",
        ResultShape::Json => "JsValue",
    }
}

/// Render `deka-host.d.ds` from [`HOST_CATALOG`]. The output is deterministic
/// and byte-stable so it can be golden-file tested and published verbatim.
pub fn host_decl() -> String {
    let mut output = String::new();
    writeln!(
        output,
        "// deka-host.d.ds — generated from the deka host catalog. Do not edit."
    )
    .unwrap();
    output.push('\n');

    for (kind_index, kind) in HOST_CATALOG.iter().enumerate() {
        if kind_index > 0 {
            output.push('\n');
        }
        writeln!(output, "bridge {} {{", kind.name).unwrap();
        for action in kind.actions {
            let args: Vec<String> = action
                .args
                .iter()
                .map(|host_arg| {
                    format!("{}: {}", host_arg.name, wire_type_to_ds(host_arg.wire))
                })
                .collect();
            let async_keyword = if action.r#async { "async " } else { "" };
            let ok_type = result_shape_to_ds(action.result);
            writeln!(
                output,
                "  {async_keyword}fn {}({}) Result<{ok_type}, string>",
                action.name,
                args.join(", "),
            )
            .unwrap();
        }
        writeln!(output, "}}").unwrap();
    }
    output
}

// Re-exported so existing `permissions::host_bridge::*` grant and denial paths
// keep working after the grants/PermissionDenied split (deka#391 file-size
// gate: that file owns the catalog, this one stays under the line limit).
pub use crate::host_grants::{
    GrantError, GrantTable, HostGrant, PERMISSION_DENIED_MARKER, PermissionDenied,
    granted_kind_names, is_official_package_name, resolve_dependency_grants,
    resolve_project_root_grants,
};

// ---- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_complete_and_wellformed() {
        assert!(!HOST_CATALOG.is_empty());
        for kind in HOST_CATALOG {
            assert!(
                kind.grant_owner.starts_with("@deka/"),
                "kind '{}' grant_owner '{}' is not an official package",
                kind.name,
                kind.grant_owner
            );
            assert!(
                !kind.actions.is_empty(),
                "kind '{}' has no actions",
                kind.name
            );
            for action in kind.actions {
                assert!(
                    !action.name.is_empty() && !action.args.is_empty(),
                    "{}.{} must have non-empty name and args",
                    kind.name,
                    action.name
                );
                assert!(
                    matches!(
                        action.capability,
                        None | Some("read" | "write" | "net" | "db")
                    ),
                    "{}.{} has invalid capability {:?}",
                    kind.name,
                    action.name,
                    action.capability
                );
            }
        }
        // Every catalogued action is findable through both lookups.
        for kind in HOST_CATALOG {
            assert_eq!(find_kind(kind.name).map(|k| k.name), Some(kind.name));
            for action in kind.actions {
                let found = find_action(kind.name, action.name).expect("find_action");
                assert_eq!(found.name, action.name);
                assert_eq!(found.r#async, action.r#async);
            }
        }
        assert!(find_kind("bogus").is_none());
        assert!(find_action("crypto", "bogus").is_none());
        assert!(find_action("bogus", "read").is_none());
    }

    #[test]
    fn async_set_matches_dsc_0_8_1_exactly() {
        // The dsc 0.8.1 contract (scripts/dsc-version): precisely these four
        // fs actions compile with `await` + `.then(__deka_to_result)`;
        // EVERYTHING else is sync. Assert set equality, not just inclusion.
        let mut async_actions: Vec<(&str, &str)> = HOST_CATALOG
            .iter()
            .flat_map(|kind| {
                kind.actions
                    .iter()
                    .filter(|action| action.r#async)
                    .map(move |action| (kind.name, action.name))
            })
            .collect();
        async_actions.sort_unstable();
        assert_eq!(
            async_actions,
            vec![
                ("fs", "mkdirs"),
                ("fs", "read_dir"),
                ("fs", "read_file"),
                ("fs", "write_file"),
            ]
        );
    }

    #[test]
    fn hosts_match_rfd27_surface() {
        let mut browser_actions: Vec<(&str, &str)> = Vec::new();
        for kind in HOST_CATALOG {
            for action in kind.actions {
                match action.hosts {
                    Hosts::NativeAndBrowser => browser_actions.push((kind.name, action.name)),
                    Hosts::NativeOnly => {}
                }
            }
        }
        browser_actions.sort_unstable();
        // NativeAndBrowser == {crypto.*, time.sleep_ms} exactly (bcrypt_verify
        // is native-only and must not appear here).
        let mut expected: Vec<(&str, &str)> = CRYPTO_ACTIONS
            .iter()
            .filter(|action| action.hosts == Hosts::NativeAndBrowser)
            .map(|action| ("crypto", action.name))
            .collect();
        expected.push(("time", "sleep_ms"));
        expected.sort_unstable();
        assert_eq!(browser_actions, expected);
        // bcrypt_verify is native-only (no WebCrypto equivalent).
        let bcrypt = find_action("crypto", "bcrypt_verify").unwrap();
        assert_eq!(bcrypt.hosts, Hosts::NativeOnly);
    }

    #[test]
    fn native_only_kinds_carry_native_only_metadata() {
        // Every action of the native-only kinds (fs/net/tls/db) is marked
        // NativeOnly in the catalog — the browser shim must never claim it
        // can serve them.
        for name in ["fs", "net", "tls", "db"] {
            let kind = find_kind(name).expect("kind present");
            for action in kind.actions {
                assert_eq!(
                    action.hosts,
                    Hosts::NativeOnly,
                    "{}.{} must be NativeOnly",
                    kind.name,
                    action.name
                );
            }
        }
    }

    #[test]
    fn js_catalog_json_roundtrips_and_matches_catalog() {
        let parsed: serde_json::Value =
            serde_json::from_str(&js_catalog_json()).expect("js_catalog_json parses");
        let object = parsed.as_object().expect("top-level object");
        assert_eq!(object.len(), HOST_CATALOG.len());
        for kind in HOST_CATALOG {
            let kind_value = object.get(kind.name).expect("kind present");
            let actions = kind_value.as_object().expect("kind object");
            assert_eq!(actions.len(), kind.actions.len());
            for action in kind.actions {
                let flag = actions.get(action.name).expect("action present");
                assert_eq!(
                    flag.get("async").and_then(serde_json::Value::as_bool),
                    Some(action.r#async),
                    "{}.{} async flag",
                    kind.name,
                    action.name
                );
            }
        }
    }

    #[test]
    fn catalog_json_roundtrips_and_matches_catalog() {
        let parsed: serde_json::Value =
            serde_json::from_str(&catalog_json()).expect("catalog_json parses");
        let object = parsed.as_object().expect("top-level object");
        assert_eq!(object.len(), HOST_CATALOG.len());
        for kind in HOST_CATALOG {
            let kind_value = object.get(kind.name).expect("kind present");
            assert_eq!(
                kind_value
                    .get("grant_owner")
                    .and_then(serde_json::Value::as_str),
                Some(kind.grant_owner)
            );
            let actions = kind_value
                .get("actions")
                .and_then(serde_json::Value::as_object)
                .expect("actions object");
            assert_eq!(actions.len(), kind.actions.len());
            for action in kind.actions {
                let action_value = actions.get(action.name).expect("action present");
                assert_eq!(
                    action_value
                        .get("async")
                        .and_then(serde_json::Value::as_bool),
                    Some(action.r#async),
                    "{}.{} async flag",
                    kind.name,
                    action.name
                );
                let args = action_value
                    .get("args")
                    .and_then(serde_json::Value::as_array)
                    .expect("args array");
                assert_eq!(args.len(), action.args.len());
                for (arg_value, host_arg) in args.iter().zip(action.args.iter()) {
                    assert_eq!(
                        arg_value.get("name").and_then(serde_json::Value::as_str),
                        Some(host_arg.name)
                    );
                    assert!(arg_value.get("wire").is_some());
                }
                assert!(action_value.get("result").is_some());
            }
        }
    }

    #[test]
    fn host_decl_contains_every_catalog_action() {
        let decl = host_decl();
        assert!(decl.starts_with("// deka-host.d.ds"));

        // Collect every action we expect, keyed for diagnostics.
        let mut expected: std::collections::HashMap<String, (&'static HostAction, &'static str)> =
            std::collections::HashMap::new();
        for kind in HOST_CATALOG {
            for action in kind.actions {
                expected.insert(
                    format!("{}.{}", kind.name, action.name),
                    (action, kind.name),
                );
            }
        }

        // Parse the declaration file naïvely: every non-comment, non-blank line
        // inside a `bridge <kind> { ... }` block must match an action.
        let mut current_kind = "";
        for raw_line in decl.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            if line.starts_with("bridge ") && line.ends_with("{") {
                current_kind = line
                    .strip_prefix("bridge ")
                    .and_then(|s| s.strip_suffix(" {"))
                    .expect("bridge block header");
                continue;
            }
            if line == "}" {
                current_kind = "";
                continue;
            }

            // "async fn name(args) Result<Ok, string>"
            let line = line
                .strip_prefix("async ")
                .unwrap_or(line)
                .strip_prefix("fn ")
                .expect("action line starts with fn");
            let (name_rest, ok_type) = line
                .rsplit_once(") Result<")
                .expect("action line has Result<...> return");
            let (name, args_part) = name_rest.split_once('(').expect("action line has args");
            let name = name.trim();
            let key = format!("{}.{}", current_kind, name);
            let (action, _kind) = expected
                .remove(&key)
                .unwrap_or_else(|| panic!("unexpected action in declaration file: {key}"));
            assert_eq!(
                action.r#async,
                raw_line.trim().starts_with("async "),
                "{key} async flag mismatch"
            );
            assert_eq!(
                ok_type.strip_suffix(", string>").expect("error type is string"),
                result_shape_to_ds(action.result),
                "{key} return type mismatch"
            );
            let expected_args: Vec<String> = action
                .args
                .iter()
                .map(|host_arg| {
                    format!("{}: {}", host_arg.name, wire_type_to_ds(host_arg.wire))
                })
                .collect();
            assert_eq!(
                args_part,
                expected_args.join(", "),
                "{key} argument list mismatch"
            );
        }

        assert!(
            expected.is_empty(),
            "catalog actions missing from declaration file: {:?}",
            expected.keys()
        );
    }

    #[test]
    fn host_decl_matches_golden_file() {
        let golden = include_str!("../tests/fixtures/deka-host.d.ds");
        assert_eq!(
            host_decl(),
            golden,
            "host_decl() output differs from golden file; run bridge_diff --dump-host-decl > \
             crates/permissions/tests/fixtures/deka-host.d.ds after a deliberate change"
        );
    }
}
