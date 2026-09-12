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
//! Kinds with no host implementation yet (neo4j, vault) are
//! deliberately absent — the catalog lists only genuinely host-implemented
//! actions, zero stubs.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

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
        args: &[arg("algorithm", WireType::Str), arg("data", WireType::Bytes)],
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
        args: &[arg("handle", WireType::Handle), arg("max_bytes", WireType::Num)],
        result: ResultShape::Bytes,
        r#async: false,
        capability: Some("read"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "write",
        args: &[arg("handle", WireType::Handle), arg("data", WireType::Bytes)],
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
        args: &[arg("handle", WireType::Handle), arg("max_bytes", WireType::Num)],
        result: ResultShape::Bytes,
        r#async: false,
        capability: Some("net"),
        hosts: Hosts::NativeOnly,
    },
    HostAction {
        name: "write",
        args: &[arg("handle", WireType::Handle), arg("data", WireType::Bytes)],
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
        args: &[arg("handle", WireType::Handle), arg("millis", WireType::Num)],
        result: ResultShape::Unit,
        r#async: false,
        capability: Some("net"),
        hosts: Hosts::NativeOnly,
    },
];

const TLS_ACTIONS: &[HostAction] = &[HostAction {
    name: "upgrade",
    args: &[arg("handle", WireType::Handle), arg("server_name", WireType::Str)],
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
/// host-implemented; kinds with no DS action surface yet (neo4j,
/// vault) are absent, not stubbed.
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
    find_kind(kind)?.actions.iter().find(|candidate| candidate.name == action)
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

// ---- Host grants -----------------------------------------------------------

/// A grant record: which bridge kinds one locked dependency digest unlocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostGrant {
    pub name: String,
    pub version: String,
    /// Lockfile-pinned content digest; the lookup key.
    pub digest: String,
    pub kinds: Vec<String>,
}

/// Grant table deserialized from the project's grant-table JSON (an array of
/// [`HostGrant`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GrantTable {
    pub grants: Vec<HostGrant>,
}

impl GrantTable {
    /// Parse a grant-table JSON document (array of grants).
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|error| format!("invalid grant table: {error}"))
    }

    /// Find the grant for a lockfile-pinned digest.
    pub fn lookup(&self, digest: &str) -> Option<&HostGrant> {
        self.grants.iter().find(|grant| grant.digest == digest)
    }
}

/// Compile-time grant violation (RFD 27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    /// An application package (non-`@deka/*`) declared `host.kinds` in its
    /// manifest. Only official packages may self-declare kinds; application
    /// bridge access must come from the project's grant table.
    AppDeclaresHostKinds { package: String, path: String },
}

impl fmt::Display for GrantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrantError::AppDeclaresHostKinds { package, path } => write!(
                formatter,
                "package '{package}' ({path}) declares host.kinds, but only @deka/* packages may self-declare bridge kinds"
            ),
        }
    }
}

impl std::error::Error for GrantError {}

/// Official (workspace) packages may self-declare bridge kinds.
pub fn is_official_package_name(name: &str) -> bool {
    name.starts_with("@deka/")
}

/// RFD 27 project-root rule: a project-root manifest that sets `host.kinds`
/// grants those kinds only when the package is official (`@deka/*` workspace
/// grant); an application project root declaring kinds is a compile error.
/// No `host.kinds` → no root grants.
pub fn resolve_project_root_grants(
    manifest_name: &str,
    manifest_path: &Path,
    host_kinds: Option<&[String]>,
) -> Result<Vec<String>, GrantError> {
    let Some(kinds) = host_kinds else { return Ok(Vec::new()) };
    if is_official_package_name(manifest_name) {
        Ok(kinds.to_vec())
    } else {
        Err(GrantError::AppDeclaresHostKinds {
            package: manifest_name.to_string(),
            path: manifest_path.display().to_string(),
        })
    }
}

/// RFD 27 dependency rule: a dependency manifest's `host.kinds` is untrusted
/// and ignored. Grants come only from the grant table via the lockfile-pinned
/// digest; no pin or no table hit → no grants.
pub fn resolve_dependency_grants(table: &GrantTable, lock_digest: Option<&str>) -> Vec<String> {
    let Some(digest) = lock_digest else { return Vec::new() };
    table.lookup(digest).map(|grant| grant.kinds.clone()).unwrap_or_default()
}

/// Union of the kinds granted to a project: the project-root grants (workspace
/// rule) plus, for each pinned dependency digest, the table lookup result.
/// Returns the [`GrantError`] unchanged when the project root declared kinds
/// illegally. The result is deduplicated in first-seen order.
pub fn granted_kind_names(
    project_root: &Result<Vec<String>, GrantError>,
    table: &GrantTable,
    dependency_digests: &[Option<String>],
) -> Result<Vec<String>, GrantError> {
    let mut names: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    };
    for kind in project_root.as_ref().map_err(Clone::clone)? {
        push(kind);
    }
    for digest in dependency_digests {
        for kind in resolve_dependency_grants(table, digest.as_deref()) {
            push(&kind);
        }
    }
    Ok(names)
}

// ---- PermissionDenied ------------------------------------------------------

/// Marker prefixed to the encoded denial so it survives the CoreError /
/// thrown-message boundary between the Rust op and DekaScript.
pub const PERMISSION_DENIED_MARKER: &str = "__DEKA_PERMISSION_DENIED__";

/// A bridge call denied by the capability policy. Encoded with
/// [`PERMISSION_DENIED_MARKER`] + compact JSON on the wire; decoded back on
/// the JS side into a DS-level `Err` — never a throw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDenied {
    pub capability: String,
    pub target: String,
}

impl PermissionDenied {
    /// Wire encoding: [`PERMISSION_DENIED_MARKER`] + compact JSON
    /// `{"capability":..,"target":..}`.
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_else(|_| "{\"capability\":\"\",\"target\":\"\"}".to_string());
        format!("{PERMISSION_DENIED_MARKER}{json}")
    }

    /// Decode a CoreError/thrown message back into a denial; `None` when the
    /// message is not a denial (no marker, bad JSON, wrong shape).
    pub fn decode(message: &str) -> Option<Self> {
        let json = message.strip_prefix(PERMISSION_DENIED_MARKER)?;
        serde_json::from_str(json).ok()
    }
}

// ---- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
            assert!(!kind.actions.is_empty(), "kind '{}' has no actions", kind.name);
            for action in kind.actions {
                assert!(
                    !action.name.is_empty() && !action.args.is_empty(),
                    "{}.{} must have non-empty name and args",
                    kind.name,
                    action.name
                );
                assert!(
                    matches!(action.capability, None | Some("read" | "write" | "net" | "db")),
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
    fn grant_table_roundtrips_and_lookups_by_digest() {
        let json = r#"[
            {"name":"@deka/fs","version":"1.2.0","digest":"sha256:aaa","kinds":["fs"]},
            {"name":"@deka/tcp","version":"0.9.1","digest":"sha256:bbb","kinds":["net","tls"]}
        ]"#;
        let table = GrantTable::from_json(json).expect("parse");
        assert_eq!(table.grants.len(), 2);
        let hit = table.lookup("sha256:bbb").expect("digest hit");
        assert_eq!(hit.name, "@deka/tcp");
        assert_eq!(hit.kinds, vec!["net", "tls"]);
        assert!(table.lookup("sha256:missing").is_none());
        assert!(GrantTable::from_json("not json").is_err());
        assert!(GrantTable::from_json("{}").is_err());
    }

    #[test]
    fn project_root_grants_follow_rfd27_rules() {
        let path = PathBuf::from("/proj/deka.json");
        // Official workspace root may self-declare kinds.
        let official = resolve_project_root_grants("@deka/fs", &path, Some(&["fs".to_string()]));
        assert_eq!(official, Ok(vec!["fs".to_string()]));
        // Official root without kinds grants nothing.
        assert_eq!(resolve_project_root_grants("@deka/fs", &path, None), Ok(vec![]));
        // App root declaring kinds is a compile error.
        let app = resolve_project_root_grants("my-app", &path, Some(&["fs".to_string()]));
        assert_eq!(
            app,
            Err(GrantError::AppDeclaresHostKinds {
                package: "my-app".to_string(),
                path: "/proj/deka.json".to_string(),
            })
        );
        // App root without kinds is fine and grants nothing.
        assert_eq!(resolve_project_root_grants("my-app", &path, None), Ok(vec![]));
        assert!(is_official_package_name("@deka/crypto"));
        assert!(!is_official_package_name("my-app"));
    }

    #[test]
    fn dependency_grants_come_only_from_the_table() {
        let table = GrantTable::from_json(
            r#"[{"name":"@deka/fs","version":"1.0.0","digest":"sha256:aaa","kinds":["fs"]}]"#,
        )
        .unwrap();
        assert_eq!(resolve_dependency_grants(&table, Some("sha256:aaa")), vec!["fs"]);
        // No lock pin → no grants, even with a populated table.
        assert!(resolve_dependency_grants(&table, None).is_empty());
        // Pin with no table hit → no grants.
        assert!(resolve_dependency_grants(&table, Some("sha256:zzz")).is_empty());
    }

    #[test]
    fn granted_kind_names_unions_root_and_dependencies() {
        let table = GrantTable::from_json(
            r#"[
                {"name":"@deka/fs","version":"1.0.0","digest":"sha256:aaa","kinds":["fs"]},
                {"name":"@deka/tcp","version":"2.0.0","digest":"sha256:bbb","kinds":["net","tls","fs"]}
            ]"#,
        )
        .unwrap();
        let root = Ok(vec!["crypto".to_string(), "fs".to_string()]);
        let digests = vec![Some("sha256:aaa".to_string()), None, Some("sha256:bbb".to_string())];
        let names = granted_kind_names(&root, &table, &digests).expect("grants");
        // Deduped, first-seen order: root kinds then dependency kinds.
        assert_eq!(names, vec!["crypto", "fs", "net", "tls"]);
        // Root grant error propagates unchanged.
        let err = Err(GrantError::AppDeclaresHostKinds {
            package: "app".to_string(),
            path: "/p/deka.json".to_string(),
        });
        assert_eq!(granted_kind_names(&err, &table, &digests), err);
    }

    #[test]
    fn permission_denied_roundtrips_and_rejects_garbage() {
        let denial = PermissionDenied {
            capability: "fs.read".to_string(),
            target: "/etc/passwd".to_string(),
        };
        let encoded = denial.encode();
        assert!(encoded.starts_with(PERMISSION_DENIED_MARKER));
        assert_eq!(PermissionDenied::decode(&encoded), Some(denial));

        // Non-denial messages decode to None.
        assert_eq!(PermissionDenied::decode("some other error"), None);
        assert_eq!(PermissionDenied::decode(""), None);
        // Truncated payload: marker present but no/invalid JSON.
        assert_eq!(PermissionDenied::decode(PERMISSION_DENIED_MARKER), None);
        assert_eq!(PermissionDenied::decode(&format!("{PERMISSION_DENIED_MARKER}{{")), None);
        assert_eq!(
            PermissionDenied::decode(&format!("{PERMISSION_DENIED_MARKER}[1,2]")),
            None
        );
        // Marker in the middle of a larger message is not a denial.
        let wrapped = format!("op failed: {}", encoded);
        assert_eq!(PermissionDenied::decode(&wrapped), None);
    }
}
