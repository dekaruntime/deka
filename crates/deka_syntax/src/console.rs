//! The `console` global (rfd#44's console addition, "## `console` (added
//! 2026-09-16)").
//!
//! `console` is a host surface, not a stdlib import: no import makes it
//! visible, and it is typed directly by the checker
//! (`typeck::expr::Checker::check_console_call`) the same way `deka.panic`
//! and `deka.ui` are. [`METHODS`] is the single list of its member names —
//! the checker's unknown-method diagnostic and the LSP's completion list
//! both read it, so the two surfaces cannot drift apart.
//!
//! This is the full WHATWG Console namespace WinterTC requires, minus
//! `profile`/`profileEnd`/`timeStamp`/`createTask` (undecided; not part of
//! the WinterTC minimum). Every method's argument shape is fixed in
//! `check_console_call`, not here — this module only names the closed set.

/// Every `console.<method>` name the checker and LSP recognize.
pub const METHODS: &[&str] = &[
    "log",
    "info",
    "debug",
    "warn",
    "error",
    "assert",
    "count",
    "countReset",
    "time",
    "timeEnd",
    "timeLog",
    "group",
    "groupEnd",
    "groupCollapsed",
    "clear",
    "dir",
    "dirxml",
    "table",
    "trace",
];

/// Native diagnostic output uses a separate host sink from ordinary output.
pub const STDERR_OPERATION: &str = "console_stderr";

/// Shared declaration for the basic variadic Printable console methods.
/// The checker and native compiler consume this catalog; formatting stays in
/// the VM printer rather than being reimplemented by each sink.
pub fn output_operation(method: &str) -> Option<&'static str> {
    match method {
        "log" | "info" | "debug" => Some("echo"),
        "warn" | "error" => Some(STDERR_OPERATION),
        _ => None,
    }
}
