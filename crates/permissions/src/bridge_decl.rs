//! Bridge declaration diffing (deka#620).
//!
//! A published `@deka/*` package declares host operations by calling
//! `bridge kind.action(args)` inside an exported function whose signature is
//! hand-maintained in the package's `.ds` source. The compiler's external dsc
//! performs no catalog validation — unknown actions type as sync
//! `Result<infer, infer>` — so a package whose declaration drifts from the
//! authoritative catalog ([`HOST_CATALOG`], RFD 27) ships silently and only
//! blows up at a consumer's call site (the @deka/fs deka#420 → deka#584 →
//! deka#618 history: declarations said sync `Result<T, string>`, the catalog
//! said async `Promise<Result<T, E>>`).
//!
//! This module diffs every declared bridge signature in a package's `.ds`
//! files against [`HOST_CATALOG`] — the same compiled artifact the runtime
//! gate uses, never a hand-copied list. It covers:
//!
//! - sync vs async status (fn `async` marker, `Promise<...>` return shape,
//!   and `await` on the call must all agree with the catalog's flag),
//! - argument shapes (arity and per-argument DS types vs wire types),
//! - return shapes (the `Ok` type of the declared `Result<...>` vs the
//!   catalog's [`ResultShape`]; the error type is package-chosen),
//! - unknown kinds/actions (a call the catalog does not list is a hard
//!   failure, not a silently-typed `Result<infer, infer>`).
//!
//! Wire/result → DS type mapping is the same single function used to generate
//! `deka-host.d.ds` (rfd#27 2026-09-16 amendment, corrected after the first
//! generated file disagreed with the published `@deka/fs` and `@deka/tcp`):
//! `Handle` → `number`, `Entries` → `Array<DirEntry>` (the runtime returns
//! host-built entry objects, never bare name strings), `Unit` → `boolean`
//! (the bridge envelope carries a literal `true`), `Json` → `JsValue`. Host
//! errors are per kind: `fs` carries `FsError`, every other kind `string`.
//! The `bridge_decl` checker and the declaration file therefore cannot drift
//! apart.

mod parse;

use crate::host_bridge::{HOST_CATALOG, HostAction, find_action};
use crate::host_decl::{result_shape_to_ds, wire_type_to_ds};
use parse::{Declaration, Type, parse_declarations, tokenize};

/// One mismatch between a declared bridge signature and the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeDiagnostic {
    /// Package whose declaration drifted (`"@deka/fs"`).
    pub package: String,
    /// File the declaration was found in.
    pub file: String,
    /// Line of the `bridge` call.
    pub line: usize,
    /// Enclosing function (the package export) that declares the signature.
    pub export: String,
    /// Bridge kind (`"fs"`).
    pub kind: String,
    /// Bridge action (`"read_file"`).
    pub action: String,
    /// Human-readable problem, naming both the catalog expectation and the
    /// declared signature.
    pub message: String,
}

impl std::fmt::Display for BridgeDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            formatter,
            "{}: export `{}` bridge {}.{}: {} (at {}:{})",
            self.package, self.export, self.kind, self.action, self.message, self.file, self.line
        )
    }
}

/// Result of checking one package's `.ds` files.
#[derive(Debug, Default)]
pub struct BridgeCheck {
    /// Number of `bridge kind.action(...)` calls checked against the catalog.
    pub declarations: usize,
    pub diagnostics: Vec<BridgeDiagnostic>,
}

impl BridgeCheck {
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// Check every `.ds` source in one package against the authoritative catalog.
/// `files` is a list of `(file_name, source_text)` pairs; `package` is the
/// package name used in diagnostics (`"@deka/fs"`).
pub fn check_package(package: &str, files: &[(String, String)]) -> BridgeCheck {
    let mut check = BridgeCheck::default();
    for (file, source) in files {
        check_source_into(package, file, source, &mut check);
    }
    check
}

/// Check a single `.ds` source, appending to `check`.
pub fn check_source_into(package: &str, file: &str, source: &str, check: &mut BridgeCheck) {
    let tokens = match tokenize(source) {
        Ok(tokens) => tokens,
        Err(message) => {
            check.diagnostics.push(BridgeDiagnostic {
                package: package.to_string(),
                file: file.to_string(),
                line: 1,
                export: String::new(),
                kind: String::new(),
                action: String::new(),
                message: format!("could not scan source: {message}"),
            });
            return;
        }
    };
    let declarations = parse_declarations(&tokens);
    for declaration in &declarations {
        check_declarations(package, file, declaration, check);
    }
    // A `bridge` call that no function owns is invalid surface; surface it
    // instead of silently skipping it.
    let owned: Vec<(usize, usize)> = declarations
        .iter()
        .map(|declaration| (declaration.body_start, declaration.body_end))
        .collect();
    for (index, token) in tokens.iter().enumerate() {
        if token.text == "bridge"
            && !owned
                .iter()
                .any(|(start, end)| index > *start && index < *end)
        {
            check.diagnostics.push(BridgeDiagnostic {
                package: package.to_string(),
                file: file.to_string(),
                line: token.line,
                export: String::new(),
                kind: String::new(),
                action: String::new(),
                message: "`bridge` call outside any function".to_string(),
            });
        }
    }
}

// ---- Diff against the catalog ------------------------------------------------

fn check_declarations(
    package: &str,
    file: &str,
    declaration: &Declaration,
    check: &mut BridgeCheck,
) {
    check.declarations += 1;
    let kind = declaration.kind.clone();
    let action = declaration.action.clone();
    let push = |check: &mut BridgeCheck, message: String| {
        check.diagnostics.push(BridgeDiagnostic {
            package: package.to_string(),
            file: file.to_string(),
            line: declaration.line,
            export: declaration.export.clone(),
            kind: kind.clone(),
            action: action.clone(),
            message,
        });
    };

    let Some(catalog_action) = find_action(&declaration.kind, &declaration.action) else {
        if declaration.kind.is_empty() || declaration.action.is_empty() {
            push(
                check,
                "malformed `bridge` call (expected `bridge kind.action(args)`)".to_string(),
            );
            return;
        }
        let known = HOST_CATALOG
            .iter()
            .find(|host_kind| host_kind.name == declaration.kind)
            .map(|host_kind| {
                let names: Vec<&str> = host_kind.actions.iter().map(|a| a.name).collect();
                format!("; known {} actions: {}", declaration.kind, names.join(", "))
            })
            .unwrap_or_else(|| "; kind is not in the catalog at all".to_string());
        push(
            check,
            format!(
                "unknown bridge {}.{}{}. The external dsc types unknown actions as sync \
                 Result<infer, infer>, so this would ship untyped",
                declaration.kind, declaration.action, known
            ),
        );
        return;
    };

    let catalog_signature = render_catalog_signature(&declaration.kind, catalog_action);
    let declared_signature = render_declared_signature(declaration);

    // Sync/async: the export's `async` marker and the `await` on the call must
    // agree with the catalog flag. The generated declaration file types an
    // async action as bare `async fn ... Result<T, E>`, but a real `async fn`
    // export needs a concrete declared return type and so may spell it out
    // explicitly as `Promise<Result<T, E>>` (deka#1145: the published
    // `@deka/fs` 0.4.1 does exactly this). Unwrap one layer of `Promise<...>`
    // before matching `Result<...>` so both spellings are recognized; the
    // `Promise<...>` wrapper is a call-site detail either way, not a distinct
    // signature shape.
    let return_type = declaration.return_type.as_ref();
    let unwrapped_return_type = return_type.map(|ty| {
        if ty.name == "Promise" && ty.args.len() == 1 {
            &ty.args[0]
        } else {
            ty
        }
    });
    let result_parts = unwrapped_return_type.and_then(|ty| {
        if ty.name == "Result" && ty.args.len() == 2 {
            Some((&ty.args[0], &ty.args[1]))
        } else {
            None
        }
    });
    let async_problems: Vec<String> = [
        (
            declaration.export_async,
            catalog_action.r#async,
            if catalog_action.r#async {
                "export is not `async` but the catalog action is async"
            } else {
                "export is `async` but the catalog action is sync"
            },
        ),
        (
            declaration.awaited,
            catalog_action.r#async,
            if catalog_action.r#async {
                "call is missing `await` but the catalog action is async"
            } else {
                "call uses `await` but the catalog action is sync"
            },
        ),
    ]
    .into_iter()
    .filter(|(declared, expected, _)| declared != expected)
    .map(|(_, _, problem)| problem.to_string())
    .collect();
    if !async_problems.is_empty() {
        push(
            check,
            format!(
                "sync/async mismatch: {}; catalog: `{}`; declared: `{}`",
                async_problems.join("; "),
                catalog_signature,
                declared_signature
            ),
        );
    }

    // Argument shapes: arity first, then per-argument types.
    if declaration.params.len() != catalog_action.args.len() {
        push(
            check,
            format!(
                "argument count mismatch: catalog takes {} arg(s) ({}) but the declaration has \
                 {}; catalog: `{}`; declared: `{}`",
                catalog_action.args.len(),
                catalog_action
                    .args
                    .iter()
                    .map(|host_arg| host_arg.name)
                    .collect::<Vec<_>>()
                    .join(", "),
                declaration.params.len(),
                catalog_signature,
                declared_signature
            ),
        );
    } else {
        for (host_arg, (param_name, param_type)) in
            catalog_action.args.iter().zip(declaration.params.iter())
        {
            let expected = wire_type_to_ds(host_arg.wire);
            if param_type.render() != expected {
                push(
                    check,
                    format!(
                        "argument `{}` shape mismatch: catalog wire type is `{}` (declared as \
                         `{}` in the catalog signature) but the declaration has `{}`; catalog: \
                         `{}`; declared: `{}`",
                        param_name,
                        expected,
                        host_arg.name,
                        param_type.render(),
                        catalog_signature,
                        declared_signature
                    ),
                );
            }
        }
    }

    // Return shape: the declared Ok type must match the catalog result shape.
    // The error type is package-chosen (string, FsError, ...) and unconstrained.
    match result_parts {
        Some((ok_type, _err_type)) => {
            let expected = result_shape_to_ds(catalog_action.result);
            if ok_type.render() != expected {
                push(
                    check,
                    format!(
                        "return shape mismatch: catalog result is `{}` but the \
                             declaration returns Ok type `{}`; catalog: `{}`; declared: `{}`",
                        expected,
                        ok_type.render(),
                        catalog_signature,
                        declared_signature
                    ),
                );
            }
        }
        None => {
            if async_problems.is_empty() {
                push(
                    check,
                    format!(
                        "return type is not `Result<T, E>`; catalog: `{}`; declared: `{}`",
                        catalog_signature, declared_signature
                    ),
                );
            }
        }
    }
}

/// Render a catalog entry as a DS-like signature for diagnostics, e.g.
/// `async fn fs.read_file(path: string) -> Result<bytes>`.
fn render_catalog_signature(kind: &str, action: &HostAction) -> String {
    let args: Vec<String> = action
        .args
        .iter()
        .map(|host_arg| {
            format!(
                "{}: {}",
                host_arg.name,
                wire_type_to_ds(host_arg.wire)
            )
        })
        .collect();
    let result = format!("Result<{}>", result_shape_to_ds(action.result));
    format!(
        "{}fn {}.{}({}) -> {result}",
        if action.r#async { "async " } else { "" },
        kind,
        action.name,
        args.join(", ")
    )
}

/// Render the declared signature for diagnostics, e.g.
/// `async fn read_file(path: string) -> Result<bytes, FsError>`.
fn render_declared_signature(declaration: &Declaration) -> String {
    let args: Vec<String> = declaration
        .params
        .iter()
        .map(|(name, ty)| format!("{}: {}", name, ty.render()))
        .collect();
    format!(
        "{}fn {}({}) -> {}",
        if declaration.export_async {
            "async "
        } else {
            ""
        },
        declaration.export,
        args.join(", "),
        declaration
            .return_type
            .as_ref()
            .map(Type::render)
            .unwrap_or_else(|| "<unparseable>".to_string())
    )
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const PKG: &str = "@deka/testpkg";

    fn check(source: &str) -> BridgeCheck {
        check_package(PKG, &[("index.ds".to_string(), source.to_string())])
    }

    fn messages(check: &BridgeCheck) -> Vec<String> {
        check
            .diagnostics
            .iter()
            .map(|diag| diag.message.clone())
            .collect()
    }

    #[test]
    fn clean_sync_declaration_passes() {
        let check = check(
            "export fn random_bytes(len: number) Result<bytes, string> {\n\
            \x20 const raw = bridge crypto.random_bytes(len)\n\
            \x20 return match (unsafe<Result<bytes, string>> { raw }) {\n\
            \x20   Ok(v) => v,\n\
            \x20   Err(e) => Err(\"cast failed\")\n\
            \x20 }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
        assert_eq!(check.declarations, 1);
    }

    #[test]
    fn clean_async_declaration_passes() {
        let check = check(
            "export async fn read_file(path: string) Result<bytes, FsError> {\n\
            \x20 const raw = await bridge fs.read_file(path)\n\
            \x20 return match (unsafe<Result<bytes, FsError>> { raw }) {\n\
            \x20   Ok(v) => v,\n\
            \x20   Err(e) => Err(FsError.Failed(\"cast failed\"))\n\
            \x20 }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
    }

    #[test]
    fn sync_declaration_for_async_action_fails_the_618_case() {
        // @deka/fs 0.3.0 shape (deka#618): sync declaration, catalog says async.
        let check = check(
            "export fn read_file(path: string) Result<bytes, string> {\n\
            \x20 return bridge fs.read_file(path)\n\
            }\n",
        );
        assert_eq!(check.diagnostics.len(), 1);
        let diagnostic = &check.diagnostics[0];
        assert_eq!(diagnostic.package, PKG);
        assert_eq!(diagnostic.export, "read_file");
        assert_eq!(diagnostic.kind, "fs");
        assert_eq!(diagnostic.action, "read_file");
        assert!(
            diagnostic.message.contains("sync/async mismatch"),
            "message: {}",
            diagnostic.message
        );
        // Both signatures are named.
        assert!(
            diagnostic
                .message
                .contains("async fn fs.read_file(path: string)")
        );
        assert!(diagnostic.message.contains("fn read_file(path: string)"));
    }

    #[test]
    fn async_declaration_for_sync_action_fails_too() {
        let check = check(
            "export async fn random_bytes(len: number) Result<bytes, string> {\n\
            \x20 const raw = await bridge crypto.random_bytes(len)\n\
            \x20 return match (unsafe<Result<bytes, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert_eq!(check.diagnostics.len(), 1);
        assert!(check.diagnostics[0].message.contains("sync/async mismatch"));
    }

    #[test]
    fn missing_await_on_async_action_fails() {
        let check = check(
            "export async fn read_file(path: string) Result<bytes, string> {\n\
            \x20 const raw = bridge fs.read_file(path)\n\
            \x20 return match (unsafe<Result<bytes, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert_eq!(check.diagnostics.len(), 1);
        assert!(
            check.diagnostics[0].message.contains("missing `await`"),
            "message: {}",
            check.diagnostics[0].message
        );
    }

    #[test]
    fn argument_shape_mismatch_fails() {
        let check = check(
            "export fn random_bytes(len: string) Result<bytes, string> {\n\
            \x20 const raw = bridge crypto.random_bytes(len)\n\
            \x20 return match (unsafe<Result<bytes, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert_eq!(check.diagnostics.len(), 1);
        assert!(
            check.diagnostics[0]
                .message
                .contains("argument `len` shape mismatch"),
            "message: {}",
            check.diagnostics[0].message
        );
    }

    #[test]
    fn argument_count_mismatch_fails() {
        let check = check(
            "export fn hmac(algorithm: string, key: bytes) Result<bytes, string> {\n\
            \x20 const raw = bridge crypto.hmac(algorithm, key)\n\
            \x20 return match (unsafe<Result<bytes, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert_eq!(check.diagnostics.len(), 1);
        assert!(
            check.diagnostics[0]
                .message
                .contains("argument count mismatch"),
            "message: {}",
            check.diagnostics[0].message
        );
    }

    #[test]
    fn return_shape_mismatch_fails() {
        // secure_compare returns Bool in the catalog; declaring number must fail.
        let check = check(
            "export fn secure_compare(a: bytes, b: bytes) Result<number, string> {\n\
            \x20 const raw = bridge crypto.secure_compare(a, b)\n\
            \x20 return match (unsafe<Result<number, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert_eq!(check.diagnostics.len(), 1);
        assert!(
            check.diagnostics[0]
                .message
                .contains("return shape mismatch"),
            "message: {}",
            check.diagnostics[0].message
        );
    }

    #[test]
    fn unit_result_maps_to_boolean() {
        // Catalog says Unit for mkdirs; the bridge envelope's `finish` sets
        // `data = true` for a unit result (worker_execution.rs), never JS
        // `undefined`, so the declaration types it as `boolean`, not `void`.
        let check = check(
            "export async fn mkdirs(path: string) Result<boolean, string> {\n\
            \x20 const raw = await bridge fs.mkdirs(path)\n\
            \x20 return match (unsafe<Result<boolean, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
    }

    #[test]
    fn void_result_for_unit_action_fails() {
        // The old (wrong) mapping: reverting result_shape_to_ds's Unit arm to
        // `void` must make this fail again (deka#1145).
        let check = check(
            "export async fn mkdirs(path: string) Result<void, string> {\n\
            \x20 const raw = await bridge fs.mkdirs(path)\n\
            \x20 return match (unsafe<Result<void, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert_eq!(check.diagnostics.len(), 1);
        assert!(
            check.diagnostics[0].message.contains("return shape mismatch"),
            "message: {}",
            check.diagnostics[0].message
        );
    }

    #[test]
    fn handle_result_maps_to_number() {
        let check = check(
            "export fn connect(host: string, port: number) Result<number, string> {\n\
            \x20 const raw = bridge net.connect(host, port)\n\
            \x20 return match (unsafe<Result<number, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
    }

    #[test]
    fn entries_result_maps_to_array_dir_entry() {
        // Catalog says Entries for read_dir; the runtime returns an array of
        // host-built DirEntry objects (PhpDirEntry), never bare name strings.
        let check = check(
            "export async fn read_dir(path: string) Result<Array<DirEntry>, string> {\n\
            \x20 const raw = await bridge fs.read_dir(path)\n\
            \x20 return match (unsafe<Result<Array<DirEntry>, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
    }

    #[test]
    fn array_string_for_entries_action_fails() {
        // The old (wrong) mapping: reverting result_shape_to_ds's Entries arm
        // to `Array<string>` must make this fail again (deka#1145).
        let check = check(
            "export async fn read_dir(path: string) Result<Array<string>, string> {\n\
            \x20 const raw = await bridge fs.read_dir(path)\n\
            \x20 return match (unsafe<Result<Array<string>, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert_eq!(check.diagnostics.len(), 1);
        assert!(
            check.diagnostics[0].message.contains("return shape mismatch"),
            "message: {}",
            check.diagnostics[0].message
        );
    }

    #[test]
    fn async_export_with_explicit_promise_wrapper_passes_the_1145_case() {
        // The real @deka/fs 0.4.1 shape (deka#1145): an `async fn` export
        // spells its return type out explicitly as `Promise<Result<T, E>>`
        // (a real async fn needs a concrete declared return type). Before the
        // fix, the checker only recognized the bare `Result<T, E>` shape and
        // misreported this as "return type is not Result<T, E>" even though
        // the sync/async status and Ok type both matched the catalog.
        let check = check(
            "export async fn read_file(path: string) Promise<Result<bytes, FsError>> {\n\
            \x20 const raw = await bridge fs.read_file(path)\n\
            \x20 return match (unsafe<Result<bytes, FsError>> { raw }) {\n\
            \x20   Ok(v) => v,\n\
            \x20   Err(e) => Err(FsError.Failed(\"cast failed\")),\n\
            \x20 }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
        assert_eq!(check.declarations, 1);
    }

    #[test]
    fn default_parameter_value_is_handled() {
        let check = check(
            "export fn read(handle: number, max_bytes: number = 4096) Result<bytes, string> {\n\
            \x20 const raw = bridge net.read(handle, max_bytes)\n\
            \x20 return match (unsafe<Result<bytes, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
    }

    #[test]
    fn unknown_action_fails() {
        let check = check(
            "export fn stat(path: string) Result<bytes, string> {\n\
            \x20 return bridge fs.stat(path)\n\
            }\n",
        );
        assert_eq!(check.diagnostics.len(), 1);
        assert!(
            check.diagnostics[0]
                .message
                .contains("unknown bridge fs.stat"),
            "message: {}",
            check.diagnostics[0].message
        );
    }

    #[test]
    fn unknown_kind_fails() {
        let check = check(
            "export fn launch(path: string) Result<bytes, string> {\n\
            \x20 return bridge process.launch(path)\n\
            }\n",
        );
        assert_eq!(check.diagnostics.len(), 1);
        assert!(
            check.diagnostics[0]
                .message
                .contains("unknown bridge process.launch"),
            "message: {}",
            check.diagnostics[0].message
        );
    }

    #[test]
    fn bridge_mentions_in_comments_are_ignored() {
        let check = check(
            "// Every export wraps a `bridge fs.*` call and needs an explicit\n\
            // `unsafe<T> { raw }` cast (dsc#223).\n\
            export fn read_file_sync(path: string) Result<bytes, string> {\n\
            \x20 const raw = bridge fs.read_file_sync(path)\n\
            \x20 return match (unsafe<Result<bytes, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
        assert_eq!(check.declarations, 1);
    }

    #[test]
    fn summon_interfaces_and_enums_do_not_confuse_the_scanner() {
        let check = check(
            "interface DirEntry {\n  name: string\n  is_dir: boolean\n}\n\
            struct FsPermission {\n  capability: string\n  target: string\n}\n\
            enum FsError {\n  PermissionDenied(FsPermission),\n  Failed(string),\n}\n\
            summon { total now_ms() number } from \"./shim.mjs\"\n\
            export fn now() number {\n  return now_ms()\n}\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
        assert_eq!(check.declarations, 0);
    }

    #[test]
    fn template_string_bodies_do_not_break_brace_matching() {
        let check = check(
            "export fn connect(host: string, port: number) Result<number, string> {\n\
            \x20 const raw = bridge net.connect(host, port)\n\
            \x20 const label = `connected to ${host}:${port} { }`\n\
            \x20 return match (unsafe<Result<number, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
    }

    #[test]
    fn unparseable_return_type_is_a_visible_failure_not_a_skip() {
        let check = check(
            "export fn connect(host: string, port: number) {\n\
            \x20 const raw = bridge net.connect(host, port)\n\
            \x20 return raw\n\
            }\n",
        );
        assert_eq!(
            check.diagnostics.len(),
            1,
            "diagnostics: {:?}",
            messages(&check)
        );
        assert!(
            check.diagnostics[0]
                .message
                .contains("return type is not `Result<T, E>`"),
            "message: {}",
            check.diagnostics[0].message
        );
    }

    #[test]
    fn bridge_outside_any_function_is_a_visible_failure() {
        let check = check("const raw = bridge fs.read_file(\"/etc/passwd\")\n");
        assert_eq!(check.diagnostics.len(), 1);
        assert!(
            check.diagnostics[0]
                .message
                .contains("outside any function"),
            "message: {}",
            check.diagnostics[0].message
        );
    }
}
