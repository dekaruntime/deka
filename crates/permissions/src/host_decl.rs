//! `deka-host.d.ds` generation (rfd#27 2026-09-16 amendment). Split out of
//! `host_bridge.rs` (deka#391 file-size gate: that file owns the catalog
//! data, this one owns the declaration-file rendering that reads it).
//!
//! The wire/result → DekaScript type mapping here is the single function the
//! generated `deka-host.d.ds` and the `bridge_decl` checker both use, so the
//! catalog and the checked declarations can never drift apart (deka#1145).

use crate::host_bridge::{HOST_CATALOG, ResultShape, WireType};
use std::fmt::Write;

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

/// Result shape → DekaScript `Result<Ok, _>` Ok type. This describes what the
/// runtime actually returns (corrected 2026-09-16, rfd#27 amendment, after
/// the first generated file disagreed with the published `@deka/fs` and
/// `@deka/tcp`): the bridge envelope's `finish` (worker_execution.rs) sets
/// `data = true` for a unit result, never `undefined`, and `data = entries`
/// — an array of host-built `PhpDirEntry` objects, never bare name strings —
/// for a directory listing.
pub fn result_shape_to_ds(shape: ResultShape) -> &'static str {
    match shape {
        ResultShape::Bytes => "bytes",
        ResultShape::Num => "number",
        ResultShape::Bool => "boolean",
        // Unit host results resolve to the envelope's literal `true`.
        ResultShape::Unit => "boolean",
        ResultShape::Handle => "number",
        // Directory listings are arrays of host-built DirEntry objects.
        ResultShape::Entries => "Array<DirEntry>",
        // Database rows are arbitrary JSON objects, not DirEntry — distinct
        // from Entries (deka#1145).
        ResultShape::Rows => "Array<JsValue>",
        ResultShape::Json => "JsValue",
    }
}

/// Result error type for a bridge kind's declaration-file signatures.
/// Host errors are per kind (rfd#27 amendment): `fs` materializes a typed
/// `FsError` enum at the bridge boundary (`__dekaFsError` in
/// worker_execution.rs); every other kind's envelope carries the raw message
/// as `string`.
pub fn error_type_for_kind(kind_name: &str) -> &'static str {
    match kind_name {
        "fs" => "FsError",
        _ => "string",
    }
}

/// Render `deka-host.d.ds` from [`HOST_CATALOG`]. The output is deterministic
/// and byte-stable so it can be golden-file tested and published verbatim.
///
/// Ahead of the `bridge` blocks, the file declares the host-produced data
/// types referenced from them — `DirEntry`, `FsPermission`, `FsError` — shaped
/// exactly as the runtime builds them (`PhpDirEntry` in
/// `deka_host::modules::fs`; `__dekaFsError` in worker_execution.rs), because
/// the host is what builds them, not any one package.
pub fn host_decl() -> String {
    let mut output = String::new();
    writeln!(
        output,
        "// deka-host.d.ds — generated from the deka host catalog. Do not edit."
    )
    .unwrap();
    output.push('\n');

    writeln!(output, "interface DirEntry {{").unwrap();
    writeln!(output, "  name: string").unwrap();
    writeln!(output, "  is_dir: boolean").unwrap();
    writeln!(output, "  is_file: boolean").unwrap();
    writeln!(output, "}}").unwrap();
    output.push('\n');

    writeln!(output, "struct FsPermission {{").unwrap();
    writeln!(output, "  capability: string").unwrap();
    writeln!(output, "  target: string").unwrap();
    writeln!(output, "}}").unwrap();
    output.push('\n');

    writeln!(output, "enum FsError {{").unwrap();
    writeln!(output, "  PermissionDenied(FsPermission),").unwrap();
    writeln!(output, "  UnsupportedHost,").unwrap();
    writeln!(output, "  InvalidPayload,").unwrap();
    writeln!(output, "  Failed(string),").unwrap();
    writeln!(output, "}}").unwrap();
    output.push('\n');

    for kind in HOST_CATALOG.iter() {
        writeln!(output, "bridge {} {{", kind.name).unwrap();
        let error_type = error_type_for_kind(kind.name);
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
                "  {async_keyword}fn {}({}) Result<{ok_type}, {error_type}>",
                action.name,
                args.join(", "),
            )
            .unwrap();
        }
        writeln!(output, "}}").unwrap();
        output.push('\n');
    }
    // Drop the trailing blank line after the last block so the file ends
    // with a single newline, matching the header's own formatting.
    if output.ends_with("}\n\n") {
        output.pop();
    }
    output
}

// ---- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_bridge::HostAction;

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
        // inside a `bridge <kind> { ... }` block must match an action. Ahead
        // of the bridge blocks, the file declares host data types (DirEntry,
        // FsPermission, FsError) in `interface`/`struct`/`enum` blocks; skip
        // those wholesale by brace depth rather than trying to parse them as
        // actions.
        let mut current_kind = "";
        let mut skip_depth = 0u32;
        for raw_line in decl.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            if skip_depth > 0 {
                if line.ends_with('{') {
                    skip_depth += 1;
                } else if line == "}" {
                    skip_depth -= 1;
                }
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
            if current_kind.is_empty() {
                // A non-bridge declaration (interface/struct/enum) the `fs`
                // block depends on, not an action line.
                if line.ends_with('{') {
                    skip_depth = 1;
                }
                continue;
            }

            // "async fn name(args) Result<Ok, ErrorType>"
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
            let (action, kind_name) = expected
                .remove(&key)
                .unwrap_or_else(|| panic!("unexpected action in declaration file: {key}"));
            assert_eq!(
                action.r#async,
                raw_line.trim().starts_with("async "),
                "{key} async flag mismatch"
            );
            let error_type = error_type_for_kind(kind_name);
            let expected_suffix = format!(", {error_type}>");
            let ok_type = ok_type.strip_suffix(expected_suffix.as_str()).unwrap_or_else(|| {
                panic!("{key}: expected error type `{error_type}`, return was `{ok_type}>`")
            });
            assert_eq!(
                ok_type,
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
