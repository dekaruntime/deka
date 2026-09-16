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
//! Wire/result → DS type mapping is grounded in the published packages that
//! are known-clean from the deka#618 audit (@deka/crypto 0.3.1, @deka/time
//! 0.2.1, @deka/tcp 0.2.0, @deka/tls 0.2.0, @deka/fs 0.4.x): handles are
//! rids typed `number`, unit results are typed `boolean`, and directory
//! listings are `Array<SomeEntry>` with a package-defined element type.
//! `WireType::Json` / `ResultShape::Json` have no published consumer yet, so
//! their value types are opaque: arity is still checked.

use crate::host_bridge::{HOST_CATALOG, HostAction, ResultShape, WireType, find_action};

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

// ---- Declaration model ------------------------------------------------------

/// A parsed `bridge kind.action(args)` call and the signature of the function
/// that declares it.
struct Declaration {
    line: usize,
    export: String,
    export_async: bool,
    params: Vec<(String, Type)>,
    /// `None` when the enclosing function has no parseable return type.
    return_type: Option<Type>,
    awaited: bool,
    kind: String,
    action: String,
    call_arg_count: usize,
    body_start: usize,
    body_end: usize,
}

/// A parsed DS type expression: dotted name (`Result`, `FsError`) plus
/// generic arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Type {
    name: String,
    args: Vec<Type>,
}

impl Type {
    fn render(&self) -> String {
        if self.args.is_empty() {
            self.name.clone()
        } else {
            format!(
                "{}<{}>",
                self.name,
                self.args
                    .iter()
                    .map(Type::render)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

// ---- Tokenizer --------------------------------------------------------------

#[derive(Debug, Clone)]
struct Token {
    text: String,
    line: usize,
}

/// Tokenize `.ds` source, dropping whitespace and `//` line comments while
/// respecting string and template-string literals. Comment text mentioning
/// `bridge` (every published package has some) must not scan as a call.
fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut line = 1;
    while index < chars.len() {
        let char = chars[index];
        if char == '\n' {
            line += 1;
            index += 1;
            continue;
        }
        if char.is_whitespace() {
            index += 1;
            continue;
        }
        if char == '/' && chars.get(index + 1) == Some(&'/') {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if char == '"' || char == '\'' {
            let quote = char;
            let start_line = line;
            index += 1;
            let mut text = String::new();
            while index < chars.len() && chars[index] != quote {
                if chars[index] == '\\' {
                    text.push(chars[index]);
                    index += 1;
                    if index < chars.len() {
                        text.push(chars[index]);
                        index += 1;
                    }
                    continue;
                }
                if chars[index] == '\n' {
                    return Err(format!("unterminated string at line {start_line}"));
                }
                text.push(chars[index]);
                index += 1;
            }
            if index >= chars.len() {
                return Err(format!("unterminated string at line {start_line}"));
            }
            index += 1; // closing quote
            tokens.push(Token {
                text: format!("{quote}{text}{quote}"),
                line: start_line,
            });
            continue;
        }
        if char == '`' {
            let start_line = line;
            index += 1;
            let mut text = String::new();
            while index < chars.len() && chars[index] != '`' {
                if chars[index] == '\\' {
                    text.push(chars[index]);
                    index += 1;
                    if index < chars.len() {
                        text.push(chars[index]);
                        index += 1;
                    }
                    continue;
                }
                if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
                    // ${...} expression: copy through the matching brace.
                    let mut depth = 0;
                    text.push(chars[index]);
                    index += 1;
                    while index < chars.len() {
                        let inner = chars[index];
                        if inner == '\n' {
                            line += 1;
                        }
                        if inner == '{' {
                            depth += 1;
                        }
                        if inner == '}' {
                            depth -= 1;
                            text.push(inner);
                            index += 1;
                            if depth == 0 {
                                break;
                            }
                            continue;
                        }
                        text.push(inner);
                        index += 1;
                    }
                    continue;
                }
                if chars[index] == '\n' {
                    line += 1;
                }
                text.push(chars[index]);
                index += 1;
            }
            if index >= chars.len() {
                return Err(format!("unterminated template string at line {start_line}"));
            }
            index += 1; // closing backtick
            tokens.push(Token {
                text: format!("`{text}`"),
                line: start_line,
            });
            continue;
        }
        if char.is_alphanumeric() || char == '_' || char == '$' {
            let start = index;
            while index < chars.len()
                && (chars[index].is_alphanumeric() || chars[index] == '_' || chars[index] == '$')
            {
                index += 1;
            }
            tokens.push(Token {
                text: chars[start..index].iter().collect(),
                line,
            });
            continue;
        }
        tokens.push(Token {
            text: char.to_string(),
            line,
        });
        index += 1;
    }
    Ok(tokens)
}

// ---- Parser -----------------------------------------------------------------

/// Find every function whose body contains a `bridge` call, with the call's
/// details and the enclosing signature. Candidate `fn` tokens that do not
/// parse as a function (interface method shapes, etc.) are abandoned and
/// scanning resumes after the token, so unfamiliar syntax degrades to
/// "not a declaration" instead of aborting the file.
fn parse_declarations(tokens: &[Token]) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index].text != "fn" {
            index += 1;
            continue;
        }
        if let Some((declaration, next)) = parse_function(tokens, index) {
            if declaration_contains_bridge(tokens, &declaration) {
                declarations.push(declaration);
            }
            index = next;
        } else {
            index += 1;
        }
    }
    declarations
}

fn parse_function(tokens: &[Token], start: usize) -> Option<(Declaration, usize)> {
    let mut index = start + 1; // skip `fn`
    if index >= tokens.len() || !is_ident(&tokens[index]) {
        return None;
    }
    let export = tokens[index].text.clone();
    let line = tokens[index].line;
    index += 1;
    if tokens.get(index).map(|token| token.text.as_str()) != Some("(") {
        return None;
    }
    let (params, next) = parse_params(tokens, index)?;
    index = next;
    if index >= tokens.len() {
        return None;
    }
    let return_type = if tokens[index].text == "{" {
        None
    } else {
        let (ty, next) = parse_type(tokens, index)?;
        index = next;
        Some(ty)
    };
    if tokens.get(index).map(|token| token.text.as_str()) != Some("{") {
        return None;
    }
    let body_start = index;
    let body_end = matching_brace(tokens, body_start)?;
    let export_async = start >= 1 && tokens[start - 1].text == "async";
    let mut declaration = Declaration {
        line,
        export,
        export_async,
        params,
        return_type,
        awaited: false,
        kind: String::new(),
        action: String::new(),
        call_arg_count: 0,
        body_start,
        body_end,
    };
    parse_bridge_call(tokens, &mut declaration);
    Some((declaration, body_end + 1))
}

/// Fill in `awaited`, `kind`, `action`, and `call_arg_count` from the first
/// `bridge kind.action(...)` token in the body. (Published packages declare
/// exactly one bridge call per function; if a function ever wraps several,
/// each extra is reported by the "outside any function" sweep — visible, not
/// silent.)
fn parse_bridge_call(tokens: &[Token], declaration: &mut Declaration) {
    let mut index = declaration.body_start + 1;
    while index < declaration.body_end {
        if tokens[index].text == "bridge" {
            declaration.awaited =
                index > declaration.body_start && tokens[index - 1].text == "await";
            let mut cursor = index + 1;
            let kind = tokens
                .get(cursor)
                .filter(|token| is_ident(token))
                .map(|token| token.text.clone());
            let dot = tokens.get(cursor + 1).map(|token| token.text.as_str()) == Some(".");
            let action = tokens
                .get(cursor + 2)
                .filter(|token| is_ident(token))
                .map(|token| token.text.clone());
            let (Some(kind), true, Some(action)) = (kind, dot, action) else {
                break;
            };
            cursor += 3;
            if tokens.get(cursor).map(|token| token.text.as_str()) != Some("(") {
                break;
            }
            let call_end = matching_paren(tokens, cursor).unwrap_or(declaration.body_end);
            declaration.call_arg_count = count_top_level_args(tokens, cursor + 1, call_end);
            declaration.kind = kind;
            declaration.action = action;
            declaration.line = tokens[index].line;
            return;
        }
        index += 1;
    }
}

/// Keep every function whose body mentions `bridge`, whether or not the call
/// itself parsed: a malformed call is attributed to its function and reported
/// as a diagnostic, never silently dropped.
fn declaration_contains_bridge(tokens: &[Token], declaration: &Declaration) -> bool {
    tokens[declaration.body_start + 1..declaration.body_end]
        .iter()
        .any(|token| token.text == "bridge")
}

/// Split the parameter list starting at the `(` token. Returns the params and
/// the index just past the closing `)`. Default values are skipped.
fn parse_params(tokens: &[Token], open: usize) -> Option<(Vec<(String, Type)>, usize)> {
    let close = matching_paren(tokens, open)?;
    let mut params = Vec::new();
    let mut index = open + 1;
    while index < close {
        if !is_ident(&tokens[index]) {
            return None;
        }
        let name = tokens[index].text.clone();
        if tokens.get(index + 1).map(|token| token.text.as_str()) != Some(":") {
            return None;
        }
        let (ty, next) = parse_type(tokens, index + 2)?;
        params.push((name, ty));
        index = next;
        if index < close {
            if tokens[index].text == "=" {
                // Default value: skip through the next top-level `,` or `)`.
                let mut depth = 0_i32;
                while index < close {
                    match tokens[index].text.as_str() {
                        "(" | "[" => depth += 1,
                        ")" | "]" => depth -= 1,
                        "," if depth == 0 => break,
                        _ => {}
                    }
                    index += 1;
                }
            }
            if tokens.get(index).map(|token| token.text.as_str()) == Some(",") {
                index += 1;
            } else {
                break;
            }
        }
    }
    Some((params, close + 1))
}

/// Parse a (possibly generic, possibly dotted) type expression starting at
/// `index`. Returns the type and the index just past it.
fn parse_type(tokens: &[Token], index: usize) -> Option<(Type, usize)> {
    if index >= tokens.len() || !is_ident(&tokens[index]) {
        return None;
    }
    let mut name = tokens[index].text.clone();
    let mut cursor = index + 1;
    while tokens.get(cursor).map(|token| token.text.as_str()) == Some(".")
        && tokens.get(cursor + 1).is_some_and(|token| is_ident(token))
    {
        name.push('.');
        name.push_str(&tokens[cursor + 1].text);
        cursor += 2;
    }
    let mut args = Vec::new();
    if tokens.get(cursor).map(|token| token.text.as_str()) == Some("<") {
        cursor += 1;
        loop {
            let (arg, next) = parse_type(tokens, cursor)?;
            args.push(arg);
            cursor = next;
            match tokens.get(cursor).map(|token| token.text.as_str()) {
                Some(",") => cursor += 1,
                Some(">") => {
                    cursor += 1;
                    break;
                }
                _ => return None,
            }
        }
    }
    Some((Type { name, args }, cursor))
}

fn matching_delimiter(
    tokens: &[Token],
    open: usize,
    open_char: &str,
    close_char: &str,
) -> Option<usize> {
    if tokens.get(open).map(|token| token.text.as_str()) != Some(open_char) {
        return None;
    }
    let mut depth = 0_i32;
    for (offset, token) in tokens.iter().enumerate().skip(open) {
        match token.text.as_str() {
            _ if token.text == open_char => depth += 1,
            _ if token.text == close_char => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn matching_brace(tokens: &[Token], open: usize) -> Option<usize> {
    matching_delimiter(tokens, open, "{", "}")
}

fn matching_paren(tokens: &[Token], open: usize) -> Option<usize> {
    matching_delimiter(tokens, open, "(", ")")
}

/// Count top-level comma-separated arguments between `start` and `end`.
fn count_top_level_args(tokens: &[Token], start: usize, end: usize) -> usize {
    let mut count = 0;
    let mut depth = 0_i32;
    let mut saw_any = false;
    for token in &tokens[start..end] {
        match token.text.as_str() {
            "(" | "[" => depth += 1,
            ")" | "]" => depth -= 1,
            "," if depth == 0 => count += 1,
            _ => {}
        }
        saw_any = true;
    }
    if saw_any { count + 1 } else { 0 }
}

fn is_ident(token: &Token) -> bool {
    token
        .text
        .chars()
        .next()
        .is_some_and(|char| char.is_alphabetic() || char == '_' || char == '$')
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

    // Sync/async: the fn marker, the Promise return shape, and `await` on the
    // call must ALL agree with the catalog flag (dsc 0.8.1 contract).
    let return_type = declaration.return_type.as_ref();
    let promise_inner = return_type.and_then(|ty| {
        if ty.name == "Promise" && ty.args.len() == 1 {
            Some(&ty.args[0])
        } else {
            None
        }
    });
    let result_parts = promise_inner.or(return_type).and_then(|ty| {
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
            promise_inner.is_some(),
            catalog_action.r#async,
            if catalog_action.r#async {
                "return type is not `Promise<...>` but the catalog action is async"
            } else {
                "return type is `Promise<...>` but the catalog action is sync"
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
            let Some(expected) = wire_type_name(host_arg.wire) else {
                continue; // Json wire: opaque until a package declares one.
            };
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
        Some((ok_type, _err_type)) => match result_shape_type(catalog_action.result) {
            ReturnExpectation::Exact(expected) => {
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
            ReturnExpectation::Array => {
                if ok_type.name != "Array" || ok_type.args.len() != 1 {
                    push(
                        check,
                        format!(
                            "return shape mismatch: catalog result is a directory listing \
                                 (`Array<SomeEntry>`) but the declaration returns Ok type `{}`; \
                                 catalog: `{}`; declared: `{}`",
                            ok_type.render(),
                            catalog_signature,
                            declared_signature
                        ),
                    );
                }
            }
            ReturnExpectation::Opaque => {}
        },
        None => {
            if async_problems.is_empty() {
                push(
                    check,
                    format!(
                        "return type is not `Result<T, E>` (or `Promise<Result<T, E>>` for async \
                         actions); catalog: `{}`; declared: `{}`",
                        catalog_signature, declared_signature
                    ),
                );
            }
        }
    }
}

enum ReturnExpectation {
    Exact(&'static str),
    Array,
    Opaque,
}

fn result_shape_type(shape: ResultShape) -> ReturnExpectation {
    match shape {
        ResultShape::Bytes => ReturnExpectation::Exact("bytes"),
        ResultShape::Num => ReturnExpectation::Exact("number"),
        ResultShape::Bool => ReturnExpectation::Exact("boolean"),
        // Unit results are surfaced as boolean success flags on the DS side
        // (mkdirs, close, set_deadline — see published @deka/fs and @deka/tcp).
        ResultShape::Unit => ReturnExpectation::Exact("boolean"),
        // Handles are rids typed `number` (@deka/tcp, @deka/tls).
        ResultShape::Handle => ReturnExpectation::Exact("number"),
        ResultShape::Entries => ReturnExpectation::Array,
        // Json results have no published consumer yet; the Ok type is opaque.
        ResultShape::Json => ReturnExpectation::Opaque,
    }
}

fn wire_type_name(wire: WireType) -> Option<&'static str> {
    match wire {
        WireType::Str => Some("string"),
        WireType::Num => Some("number"),
        WireType::Bool => Some("boolean"),
        WireType::Bytes => Some("bytes"),
        WireType::Handle => Some("number"),
        // Json args have no published consumer yet; the type is opaque.
        WireType::Json => None,
    }
}

/// Render a catalog entry as a DS-like signature for diagnostics, e.g.
/// `async fn fs.read_file(path: string) -> Result<bytes>` (`boolean` covers
/// unit results, `Array<Entry>` covers directory listings).
fn render_catalog_signature(kind: &str, action: &HostAction) -> String {
    let args: Vec<String> = action
        .args
        .iter()
        .map(|host_arg| {
            let wire = wire_type_name(host_arg.wire).unwrap_or("json");
            format!("{}: {}", host_arg.name, wire)
        })
        .collect();
    let result = match result_shape_type(action.result) {
        ReturnExpectation::Exact(name) => format!("Result<{name}>"),
        ReturnExpectation::Array => "Result<Array<Entry>>".to_string(),
        ReturnExpectation::Opaque => "Result<Json>".to_string(),
    };
    format!(
        "{}fn {}.{}({}) -> {result}",
        if action.r#async { "async " } else { "" },
        kind,
        action.name,
        args.join(", ")
    )
}

/// Render the declared signature for diagnostics, e.g.
/// `async fn read_file(path: string) -> Promise<Result<bytes, FsError>>`.
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
            "export async fn read_file(path: string) Promise<Result<bytes, FsError>> {\n\
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
            "export async fn random_bytes(len: number) Promise<Result<bytes, string>> {\n\
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
            "export async fn read_file(path: string) Promise<Result<bytes, string>> {\n\
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
        // Catalog says Unit for mkdirs; published packages declare boolean.
        let check = check(
            "export async fn mkdirs(path: string) Promise<Result<boolean, string>> {\n\
            \x20 const raw = await bridge fs.mkdirs(path)\n\
            \x20 return match (unsafe<Result<boolean, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
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
    fn entries_result_accepts_any_array_element() {
        let check = check(
            "interface DirEntry {\n  name: string\n  is_dir: boolean\n}\n\
            export async fn read_dir(path: string) Promise<Result<Array<DirEntry>, string>> {\n\
            \x20 const raw = await bridge fs.read_dir(path)\n\
            \x20 return match (unsafe<Result<Array<DirEntry>, string>> { raw }) { Ok(v) => v, Err(e) => Err(\"x\") }\n\
            }\n",
        );
        assert!(check.is_clean(), "diagnostics: {:?}", messages(&check));
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
