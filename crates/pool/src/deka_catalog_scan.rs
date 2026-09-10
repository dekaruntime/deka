//! RFD 21 source gate: scan DekaScript sources for `deka.*` catalog calls
//! before dsc compiles them, validate against the closed catalog in
//! [`runtime_core::deka_catalog`], and lower `safe { deka.kind.method(...) }`
//! to the verbatim call dsc emits raw.
//!
//! dsc (pinned, 0.8.2) knows no `safe` keyword: it would reject the syntax
//! outright. But it types the ambient `deka` global as `Infer` and emits a
//! plain `deka.kind.method(args)` expression **verbatim** — which is exactly
//! RFD 21's "safe is the call verbatim". So the loader, which owns the
//! source-to-dsc boundary, performs the parse + catalog check itself:
//!
//! - `safe { deka.kind.method(args) }` (stdlib only) is validated — known
//!   kind, known method, `Safe` classification, arity — and rewritten to the
//!   bare call, which dsc compiles to the raw helper invocation returning its
//!   declared type. No try/catch is paid for a helper that cannot throw.
//! - `unsafe { deka.kind.method(args) }` (stdlib only) is validated the same
//!   way and left for dsc to wrap in its try/catch → `Result<T, E>`.
//! - Anything else — unknown helper, wrong arity, a catalog call in a user
//!   package, a bare catalog call outside the two doors — is a source
//!   diagnostic with file:line:col, before dsc ever runs.
//!
//! Because sources must be dsc-parseable on disk, a rewrite triggers a staged
//! compile: the project tree is mirrored (hardlinked) under `.cache`, the
//! rewritten files become real copies there, and dsc runs against the mirror.
//! Projects that use no `safe` blocks compile in place exactly as before.
//!
//! The catalog and this scanner read no process environment (deka#801);
//! package stdlib-ness comes from `@deka/*` naming only.

use std::fs;
use std::path::{Path, PathBuf};

use runtime_core::deka_catalog;
use runtime_core::host_bridge;
use runtime_core::DEKA_VALIDATION_ERROR_MARKER;

/// A source diagnostic: `path:line:col: message`, matching dsc's format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanDiagnostic {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for ScanDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}: {}", self.path.display(), self.line, self.column, self.message)
    }
}

/// Result of scanning one source file.
#[derive(Debug, Default)]
pub struct ScanResult {
    /// Source with every validated `safe { deka.* }` site lowered to the bare
    /// call. `None` when no rewrite was needed.
    pub rewritten: Option<String>,
    pub diagnostics: Vec<(usize, String)>,
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$')
}

/// Byte-cursor helpers shared by the scan passes: string-, template-, and
/// comment-aware movement so punctuation inside literals never confuses
/// delimiter matching.
struct Cursor<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(src: &'a [u8], pos: usize) -> Self {
        Cursor { src, pos }
    }

    fn at(&self, pos: usize) -> Option<u8> {
        self.src.get(pos).copied()
    }

    /// Skip whitespace, `//` line comments (incl. `///` doc comments), and
    /// string/template literals starting at `pos`. Returns the new offset.
    fn skip_insignificant(&self, mut pos: usize) -> usize {
        loop {
            match self.at(pos) {
                Some(b) if b.is_ascii_whitespace() => pos += 1,
                Some(b'/') if self.at(pos + 1) == Some(b'/') => {
                    while let Some(b) = self.at(pos) {
                        if b == b'\n' {
                            break;
                        }
                        pos += 1;
                    }
                }
                Some(b'"') | Some(b'\'') => pos = self.skip_string(pos),
                Some(b'`') => pos = self.skip_template(pos),
                _ => return pos,
            }
        }
    }

    /// Skip a `'...'` / `"..."` literal starting at its quote.
    fn skip_string(&self, mut pos: usize) -> usize {
        let quote = self.src[pos];
        pos += 1;
        while let Some(b) = self.at(pos) {
            if b == b'\\' {
                pos += 2;
                continue;
            }
            pos += 1;
            if b == quote {
                break;
            }
        }
        pos
    }

    /// Skip a `` `...` `` template literal starting at its backtick, handling
    /// nested `${ ... }` expressions (which may contain strings, templates,
    /// and comments) recursively.
    fn skip_template(&self, mut pos: usize) -> usize {
        pos += 1; // backtick
        while let Some(b) = self.at(pos) {
            match b {
                b'\\' => pos += 2,
                b'`' => return pos + 1,
                b'$' if self.at(pos + 1) == Some(b'{') => {
                    pos = self.skip_balanced(pos + 1, b'{', b'}');
                }
                _ => pos += 1,
            }
        }
        pos
    }

    /// Skip a balanced delimiter pair (the opener at `pos`), aware of
    /// strings, templates, and line comments. Returns the offset just past
    /// the matching closer, or end of input.
    fn skip_balanced(&self, pos: usize, open: u8, close: u8) -> usize {
        let mut depth = 0usize;
        let mut pos = pos;
        while let Some(b) = self.at(pos) {
            match b {
                _ if b == open => {
                    depth += 1;
                    pos += 1;
                }
                _ if b == close => {
                    depth -= 1;
                    pos += 1;
                    if depth == 0 {
                        return pos;
                    }
                }
                b'"' | b'\'' => pos = self.skip_string(pos),
                b'`' => pos = self.skip_template(pos),
                b'/' if self.at(pos + 1) == Some(b'/') => {
                    pos = self.skip_insignificant(pos);
                }
                _ => pos += 1,
            }
        }
        pos
    }
}

/// Parse `deka.<kind>.<method>(` at `pos` (after insignificant-skipping by
/// the caller). On a shape match returns `(kind, method, args_start)` where
/// `args_start` is the offset just past the `(`.
fn parse_call_head(src: &[u8], mut pos: usize) -> Option<(&str, &str, usize)> {
    let cur = Cursor { src, pos: 0 };
    if src.len() < pos + 4 || &src[pos..pos + 4] != b"deka" {
        return None;
    }
    pos += 4;
    // Not followed by another identifier byte (`dekaX` is a different name).
    if src.get(pos).is_some_and(|&b| is_ident_byte(b)) {
        return None;
    }
    let read_ident = |pos: usize| -> Option<(usize, usize)> {
        let start = pos;
        let mut end = pos;
        while src.get(end).is_some_and(|&b| is_ident_byte(b)) {
            end += 1;
        }
        (end > start).then_some((start, end))
    };
    if cur.at(pos) != Some(b'.') {
        return None;
    }
    pos = cur.skip_insignificant(pos + 1);
    let (ks, ke) = read_ident(pos)?;
    pos = cur.skip_insignificant(ke);
    if cur.at(pos) != Some(b'.') {
        return None;
    }
    pos = cur.skip_insignificant(pos + 1);
    let (ms, me) = read_ident(pos)?;
    pos = cur.skip_insignificant(me);
    if cur.at(pos) != Some(b'(') {
        return None;
    }
    let kind = std::str::from_utf8(&src[ks..ke]).ok()?;
    let method = std::str::from_utf8(&src[ms..me]).ok()?;
    Some((kind, method, pos + 1))
}

/// Count top-level commas between `args_start` and its matching `)`,
/// returning `(argc, offset_past_close_paren)`. A trailing comma before the
/// closer does not introduce an extra argument.
fn scan_args(src: &[u8], args_start: usize) -> (usize, usize) {
    let cur = Cursor { src, pos: 0 };
    let mut argc = 0usize;
    let mut saw_value = false;
    let mut last_sig_comma = false;
    let mut pos = args_start;
    let mut depth = 0usize;
    while let Some(b) = cur.at(pos) {
        match b {
            b'(' | b'[' | b'{' => {
                depth += 1;
                pos += 1;
            }
            b')' if depth == 0 => {
                let count = if saw_value && !last_sig_comma { argc + 1 } else { argc };
                return (count, pos + 1);
            }
            b')' | b']' | b'}' => {
                depth -= 1;
                pos += 1;
            }
            b',' if depth == 0 => {
                argc += 1;
                last_sig_comma = true;
                pos += 1;
            }
            b'"' | b'\'' => {
                saw_value = true;
                last_sig_comma = false;
                pos = cur.skip_string(pos);
            }
            b'`' => {
                saw_value = true;
                last_sig_comma = false;
                pos = cur.skip_template(pos);
            }
            b'/' if cur.at(pos + 1) == Some(b'/') => {
                pos = cur.skip_insignificant(pos);
            }
            b if b.is_ascii_whitespace() => pos += 1,
            _ => {
                saw_value = true;
                last_sig_comma = false;
                pos += 1;
            }
        }
    }
    (if saw_value && !last_sig_comma { argc + 1 } else { argc }, pos)
}

/// Scan one source file. `stdlib` is whether the file's package is an
/// official `@deka/*` stdlib package — non-stdlib sources may not reference
/// the catalog at all.
pub fn scan_source(source: &str, stdlib: bool) -> ScanResult {
    let src = source.as_bytes();
    let cur = Cursor { src, pos: 0 };
    let mut result = ScanResult::default();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    // Spans already claimed by a validated/​diagnosed safe/unsafe block, so
    // the bare-reference pass does not re-report them.
    let mut claimed: Vec<(usize, usize)> = Vec::new();

    let mut pos = 0usize;
    while pos < src.len() {
        match src[pos] {
            b'"' | b'\'' => pos = cur.skip_string(pos),
            b'`' => pos = cur.skip_template(pos),
            b'/' if cur.at(pos + 1) == Some(b'/') => pos = cur.skip_insignificant(pos),
            b if is_ident_byte(b) && (pos == 0 || !is_ident_byte(src[pos - 1])) => {
                let start = pos;
                let mut end = pos;
                while src.get(end).is_some_and(|&c| is_ident_byte(c)) {
                    end += 1;
                }
                let word = &source[start..end];
                if word == "safe" || word == "unsafe" {
                    pos = scan_block(
                        source,
                        start,
                        end,
                        word == "safe",
                        stdlib,
                        &mut result,
                        &mut edits,
                        &mut claimed,
                    );
                } else {
                    pos = end;
                }
            }
            _ => pos += 1,
        }
    }

    // Bare-reference pass: any remaining `deka.<catalog-kind>` use outside
    // the two doors is a diagnostic (stdlib must route through safe/unsafe;
    // user code may not touch the catalog at all).
    let mut scan = 0usize;
    while scan < src.len() {
        match src[scan] {
            b'"' | b'\'' => {
                scan = cur.skip_string(scan);
                continue;
            }
            b'`' => {
                scan = cur.skip_template(scan);
                continue;
            }
            b'/' if cur.at(scan + 1) == Some(b'/') => {
                scan = cur.skip_insignificant(scan);
                continue;
            }
            _ => {}
        }
        if let Some((kind, _method, _)) = parse_call_head(src, scan) {
            let in_claimed = claimed.iter().any(|&(s, e)| scan >= s && scan < e);
            let in_edit = edits.iter().any(|&(s, e, _)| scan >= s && scan < e);
            if !in_claimed && !in_edit {
                if deka_catalog::is_catalog_kind(kind) {
                    result.diagnostics.push((
                        scan,
                        if stdlib {
                            format!(
                                "catalog call `deka.{kind}.*` must appear as the body of `safe {{ }}` or `unsafe {{ }}` so its classification is checked (RFD 21)"
                            )
                        } else {
                            format!(
                                "the `deka.*` catalog is closed and stdlib-only; application code may not call `deka.{kind}.*` (RFD 21)"
                            )
                        },
                    ));
                }
                scan += 4;
                continue;
            }
        }
        scan += 1;
    }

    if !edits.is_empty() {
        let mut rewritten = String::with_capacity(source.len());
        let mut cursor = 0usize;
        for (start, end, replacement) in edits {
            rewritten.push_str(&source[cursor..start]);
            rewritten.push_str(&replacement);
            cursor = end;
        }
        rewritten.push_str(&source[cursor..]);
        result.rewritten = Some(rewritten);
    }
    result
}

/// Handle one `safe { ... }` / `unsafe { ... }` block starting at the keyword.
/// Returns the offset to continue scanning from.
#[allow(clippy::too_many_arguments)]
fn scan_block(
    source: &str,
    kw_start: usize,
    kw_end: usize,
    is_safe: bool,
    stdlib: bool,
    result: &mut ScanResult,
    edits: &mut Vec<(usize, usize, String)>,
    claimed: &mut Vec<(usize, usize)>,
) -> usize {
    let src = source.as_bytes();
    let cur = Cursor { src, pos: 0 };
    let mut pos = cur.skip_insignificant(kw_end);
    // `unsafe<T> { ... }` result-type annotation (dsc#460).
    if !is_safe && cur.at(pos) == Some(b'<') {
        let mut depth = 0usize;
        while let Some(b) = cur.at(pos) {
            match b {
                b'<' => depth += 1,
                b'>' => {
                    depth -= 1;
                    if depth == 0 {
                        pos += 1;
                        break;
                    }
                }
                _ => {}
            }
            pos += 1;
        }
        pos = cur.skip_insignificant(pos);
    }
    if cur.at(pos) != Some(b'{') {
        // Not a block (plain identifier use); leave it to dsc.
        return kw_end;
    }
    let body_start = pos + 1;
    let block_end = cur.skip_balanced(pos, b'{', b'}');
    let body_end = block_end.saturating_sub(1);
    let body_trimmed = trim_span(source, body_start, body_end);

    if is_safe {
        return finish_safe_block(
            source,
            kw_start,
            body_trimmed,
            block_end,
            stdlib,
            result,
            edits,
            claimed,
        );
    }

    // unsafe { ... }: validate only when the body is exactly a catalog call
    // (modulo a trailing `;`). Raw platform JS under unsafe is untouched.
    if let Some((kind, method, args_start)) = parse_call_head(src, body_trimmed.0) {
        let (_argc, call_end) = scan_args(src, args_start);
        let after = cur.skip_insignificant(call_end);
        // Clean when nothing (or only `;` + whitespace) trails the call
        // before the closing brace.
        let clean_shape = after > body_trimmed.1
            || source[after..body_trimmed.1].trim_end_matches(';').trim().is_empty();
        if clean_shape {
            if deka_catalog::is_catalog_kind(kind) {
                claimed.push((kw_start, block_end));
                if contains_block_keyword(src, body_trimmed.0, body_trimmed.1) {
                    result.diagnostics.push((
                        body_trimmed.0,
                        "nested `safe` / `unsafe` blocks are not supported in a catalog call; \
                         bind the inner result to a `const` first (RFD 21)"
                            .to_string(),
                    ));
                    return block_end;
                }
                validate_call(
                    source,
                    body_trimmed.0,
                    kind,
                    method,
                    args_start,
                    stdlib,
                    "unsafe",
                    result,
                );
            }
            return block_end;
        }
        // Starts like a catalog call but has trailing statements.
        if deka_catalog::is_catalog_kind(kind) {
            claimed.push((kw_start, block_end));
            result.diagnostics.push((
                body_trimmed.0,
                format!(
                    "malformed catalog call under `unsafe`: the body must be exactly `deka.{kind}.{method}(...)`, with nothing before or after"
                ),
            ));
        }
        return block_end;
    }
    block_end
}

/// Validate a parsed `deka.<kind>.<method>(args)` call span against the
/// closed catalog and, when valid and `safe`, record the lowering edit.
#[allow(clippy::too_many_arguments)]
fn finish_safe_block(
    source: &str,
    kw_start: usize,
    body: (usize, usize),
    block_end: usize,
    stdlib: bool,
    result: &mut ScanResult,
    edits: &mut Vec<(usize, usize, String)>,
    claimed: &mut Vec<(usize, usize)>,
) -> usize {
    let src = source.as_bytes();
    let cur = Cursor { src, pos: 0 };
    let head_start = cur.skip_insignificant(body.0);
    let Some(head) = parse_call_head(src, head_start) else {
        result.diagnostics.push((
            body.0,
            "`safe { }` bodies must be exactly one `deka.<kind>.<method>(...)` catalog call; \
             platform JavaScript is not allowed under `safe` (RFD 21)"
                .to_string(),
        ));
        claimed.push((kw_start, block_end));
        return block_end;
    };
    let (_argc, call_end) = scan_args(src, head.2);
    let after = cur.skip_insignificant(call_end);
    // `after` past the trimmed body end means only whitespace/comments sat
    // between the call and the closing brace — still a clean shape.
    if after <= body.1 && !source[after..body.1].trim().is_empty() {
        result.diagnostics.push((
            body.0,
            format!(
                "malformed `safe` catalog call: the body must be exactly `deka.{}.{}(...)`, with nothing after the call",
                head.0, head.1
            ),
        ));
        claimed.push((kw_start, block_end));
        return block_end;
    }
    claimed.push((kw_start, block_end));
    if contains_block_keyword(src, head_start, call_end) {
        result.diagnostics.push((
            head_start,
            "nested `safe` / `unsafe` blocks are not supported in a catalog call; \
             bind the inner result to a `const` first (RFD 21)"
                .to_string(),
        ));
        return block_end;
    }
    let valid = validate_call(source, body.0, head.0, head.1, head.2, stdlib, "safe", result);
    if valid {
        // Lower `safe { deka.k.m(args) }` to `deka.k.m(args)`: dsc types the
        // ambient `deka` global as Infer, emits the call verbatim, and the
        // runtime preamble resolves `deka` for stdlib modules — the verbatim
        // call is RFD 21's safe emission.
        let call = source[head_start..call_end].trim().to_string();
        edits.push((kw_start, block_end, strip_trailing_comma(&call)));
    }
    block_end
}

/// Remove a top-level trailing comma directly before the outer call's closing
/// paren. dsc rejects trailing commas in argument lists; the `safe` grammar
/// tolerates them so the lowered call must normalize them away.
fn strip_trailing_comma(call: &str) -> String {
    let bytes = call.as_bytes();
    let Some(open) = bytes.iter().position(|&b| b == b'(') else { return call.to_string() };
    let cur = Cursor { src: bytes, pos: 0 };
    let mut depth = 0usize;
    let mut close = None;
    let mut pos = open;
    while let Some(b) = cur.at(pos) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(pos);
                    break;
                }
            }
            b'"' | b'\'' => {
                pos = cur.skip_string(pos);
                continue;
            }
            b'`' => {
                pos = cur.skip_template(pos);
                continue;
            }
            _ => {}
        }
        pos += 1;
    }
    let Some(close) = close else { return call.to_string() };
    let mut tail = close;
    while tail > open && bytes[tail - 1].is_ascii_whitespace() {
        tail -= 1;
    }
    if tail > open && bytes[tail - 1] == b',' {
        let mut out = String::with_capacity(call.len() - 1);
        out.push_str(&call[..tail - 1]);
        out.push_str(&call[tail..]);
        out
    } else {
        call.to_string()
    }
}

/// Validate kind/method/classification/arity for a call whose head parsed.
/// Returns whether the call is fully valid.
fn validate_call(
    source: &str,
    call_offset: usize,
    kind: &str,
    method: &str,
    args_start: usize,
    stdlib: bool,
    door: &str,
    result: &mut ScanResult,
) -> bool {
    if !stdlib {
        result.diagnostics.push((
            call_offset,
            format!(
                "the `deka.*` catalog is closed and stdlib-only; application code may not call `deka.{kind}.{method}` under `{door}` (RFD 21)"
            ),
        ));
        return false;
    }
    let Some(entry) = deka_catalog::find_method(kind, method) else {
        let kind_known = deka_catalog::find_kind(kind).is_some();
        result.diagnostics.push((
            call_offset,
            if kind_known {
                format!(
                    "unknown `deka.{kind}` helper `{method}` — the catalog is closed; new helpers are a reviewed addition (RFD 21)"
                )
            } else {
                format!(
                    "unknown `deka` kind `{kind}` — the catalog is closed; known kinds: {}",
                    deka_catalog::DEKA_CATALOG
                        .iter()
                        .map(|k| k.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
        ));
        return false;
    };
    if door == "safe" && entry.safety != deka_catalog::Safety::Safe {
        result.diagnostics.push((
            call_offset,
            format!(
                "`deka.{kind}.{method}` may throw ({}) — call it under `unsafe {{ }}` so the failure is a `Result` value you must handle",
                entry.doc
            ),
        ));
        return false;
    }
    let (argc, _close) = scan_args(source.as_bytes(), args_start);
    if let Err(err) = deka_catalog::check_arity(entry, argc) {
        result.diagnostics.push((
            call_offset,
            format!("`deka.{kind}.{method}`: {err}"),
        ));
        return false;
    }
    true
}

fn trim_span(source: &str, start: usize, end: usize) -> (usize, usize) {
    let mut s = start;
    let mut e = end;
    while s < e && source.as_bytes()[s].is_ascii_whitespace() {
        s += 1;
    }
    while e > s && source.as_bytes()[e - 1].is_ascii_whitespace() {
        e -= 1;
    }
    (s, e)
}

/// Whether the span contains a `safe` / `unsafe` keyword (word-delimited,
/// outside strings/comments/templates). Used to reject nested blocks, which
/// the single-pass lowering cannot reach.
fn contains_block_keyword(src: &[u8], start: usize, end: usize) -> bool {
    let cur = Cursor { src, pos: 0 };
    let mut pos = start;
    while pos < end && pos < src.len() {
        match src[pos] {
            b'"' | b'\'' => pos = cur.skip_string(pos),
            b'`' => pos = cur.skip_template(pos),
            b'/' if cur.at(pos + 1) == Some(b'/') => pos = cur.skip_insignificant(pos),
            b if is_ident_byte(b) && (pos == 0 || !is_ident_byte(src[pos - 1])) => {
                let word_start = pos;
                let mut word_end = pos;
                while word_end < end && src.get(word_end).is_some_and(|&c| is_ident_byte(c)) {
                    word_end += 1;
                }
                let word = &src[word_start..word_end];
                if word == b"safe" || word == b"unsafe" {
                    return true;
                }
                pos = word_end;
            }
            _ => pos += 1,
        }
    }
    false
}

// ---- Compile-root preparation (scan + optional staging) ---------------------

/// Directories never mirrored into a compile stage and never scanned: VCS and
/// tooling state dsc cannot import, plus the cache trees themselves.
const SKIPPED_DIRS: &[&str] = &[".git", ".cache", "node_modules", "target"];

fn is_skipped_dir(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| SKIPPED_DIRS.contains(&name))
}

/// Manifest `name` at `root/deka.json`, when present and parseable.
fn manifest_name(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join("deka.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("name")?.as_str().map(str::to_string)
}

/// Whether the source file at `path` belongs to an official `@deka/*` stdlib
/// package: a `@deka/*` dependency under `root/{ds_modules,php_modules}`, the
/// project root itself when its manifest names an official package, or a
/// linked package outside `root` whose nearest manifest is official.
pub fn is_stdlib_source(path: &Path, root: &Path) -> bool {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path = canonical.as_path();
    for dir in ["ds_modules", "php_modules"] {
        if let Ok(rel) = path.strip_prefix(root.join(dir)) {
            let mut components = rel.components();
            let Some(first) = components.next() else { return false };
            let first = first.as_os_str().to_string_lossy();
            let name = if first.starts_with('@') {
                let Some(second) = components.next() else { return false };
                format!("{first}/{}", second.as_os_str().to_string_lossy())
            } else {
                first.into_owned()
            };
            return host_bridge::is_official_package_name(&name);
        }
    }
    if path.starts_with(&root) {
        return manifest_name(&root)
            .map(|name| host_bridge::is_official_package_name(&name))
            .unwrap_or(false);
    }
    path.ancestors()
        .find(|ancestor| ancestor.join("deka.json").is_file())
        .and_then(|ancestor| manifest_name(ancestor))
        .map(|name| host_bridge::is_official_package_name(&name))
        .unwrap_or(false)
}

/// A staged compile root. Deletes the mirror on drop.
#[derive(Debug)]
pub struct StageGuard {
    dir: PathBuf,
    /// Original root this mirror reflects.
    source_root: PathBuf,
}

impl StageGuard {
    pub fn root(&self) -> &Path {
        &self.dir
    }

    /// Map an absolute source-root path to its staged equivalent. Input paths
    /// are canonicalized first: callers may hold the macOS `/var` spelling
    /// while the guard's roots are canonical `/private/var` paths.
    pub fn map_path(&self, path: &Path) -> PathBuf {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let rel = path.strip_prefix(&self.source_root).unwrap_or(&path);
        self.dir.join(rel)
    }

    /// Map an absolute staged path back to its source-root equivalent.
    pub fn unmap_path(&self, path: &Path) -> PathBuf {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let rel = path.strip_prefix(&self.dir).unwrap_or(&path);
        self.source_root.join(rel)
    }

    /// Rewrite staged paths in a dsc diagnostic back to source paths so
    /// diagnostics name the files the user actually owns.
    pub fn remap_diagnostic(&self, text: &str) -> String {
        text.replace(
            self.dir.to_string_lossy().as_ref(),
            self.source_root.to_string_lossy().as_ref(),
        )
    }
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[derive(Debug)]
pub enum PreparedRoot {
    /// No `safe` blocks in the tree; compile against the real root.
    InPlace,
    /// At least one `safe` block was lowered; compile against the mirror.
    Staged(StageGuard),
}

/// Scan every DekaScript source under `root` for catalog calls, validating
/// against the closed catalog. When any `safe` block needed lowering, build a
/// staged mirror for dsc; otherwise report [`PreparedRoot::InPlace`].
pub fn prepare_compile_root(root: &Path) -> Result<PreparedRoot, String> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    // Collect candidate sources first (skip tooling dirs).
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !is_skipped_dir(&entry.file_name()) {
                    stack.push(path);
                }
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("ds" | "dsx")
            ) {
                sources.push(path);
            }
        }
    }
    sources.sort();

    let mut diagnostics: Vec<ScanDiagnostic> = Vec::new();
    let mut rewritten: Vec<(PathBuf, String)> = Vec::new();
    for path in sources {
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let stdlib = is_stdlib_source(&path, &root);
        let result = scan_source(&source, stdlib);
        // Offsets to 1-based line:col against the original source.
        let line_starts: Vec<usize> = std::iter::once(0usize)
            .chain(source.bytes().enumerate().filter_map(|(i, b)| (b == b'\n').then_some(i + 1)))
            .collect();
        let locate = |offset: usize| {
            let line = line_starts.partition_point(|&start| start <= offset);
            let column = offset - line_starts[line - 1] + 1;
            (line, column)
        };
        for (offset, message) in result.diagnostics {
            let (line, column) = locate(offset);
            diagnostics.push(ScanDiagnostic { path: path.clone(), line, column, message });
        }
        if let Some(text) = result.rewritten {
            rewritten.push((path, text));
        }
    }

    if !diagnostics.is_empty() {
        let detail = diagnostics
            .iter()
            .map(ScanDiagnostic::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("{DEKA_VALIDATION_ERROR_MARKER}{detail}"));
    }

    if rewritten.is_empty() {
        return Ok(PreparedRoot::InPlace);
    }

    // Stage: mirror the tree with hardlinks; lowered sources become real
    // copies inside the mirror only. The user's tree is never modified.
    // The name embeds a nanosecond stamp so concurrent compiles of the same
    // root never share a mirror.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let stage = root.join(".cache").join(format!("dsc-stage-{}-{nonce}", std::process::id()));
    let _ = fs::remove_dir_all(&stage);
    mirror_tree(&root, &stage, &rewritten.iter().cloned().collect())?;
    // dsc verifies every locked dependency by hashing the sources it reads.
    // Inside the mirror those are the lowered sources, which cannot match the
    // lock's published-content hashes — so verify the real tree against the
    // real lock here (failing closed on any mismatch), then rewrite the
    // mirror's lock copy to describe the lowered mirror truthfully. The
    // user's deka.lock is never modified: the mirror holds a real copy.
    if let Err(err) = reconcile_staged_lock(&root, &stage, &rewritten) {
        let _ = fs::remove_dir_all(&stage);
        return Err(err);
    }
    Ok(PreparedRoot::Staged(StageGuard { dir: stage, source_root: root }))
}

/// Package directories under `ds_modules`/`php_modules` that contain at least
/// one rewritten source, as `(modules_dir, package_name)`.
fn rewritten_packages(
    root: &Path,
    rewritten: &[(PathBuf, String)],
) -> Vec<(String, String)> {
    let mut packages = Vec::new();
    for (path, _) in rewritten {
        for modules_dir in ["ds_modules", "php_modules"] {
            let Ok(rel) = path.strip_prefix(root.join(modules_dir)) else {
                continue;
            };
            let mut components = rel.components();
            let Some(first) = components.next() else { break };
            let first = first.as_os_str().to_string_lossy();
            let name = if first.starts_with('@') {
                let Some(second) = components.next() else { break };
                format!("{first}/{}", second.as_os_str().to_string_lossy())
            } else {
                first.into_owned()
            };
            let entry = (modules_dir.to_string(), name);
            if !packages.contains(&entry) {
                packages.push(entry);
            }
            break;
        }
    }
    packages
}

/// Verify rewritten dependency packages against the real lock and patch the
/// staged mirror's lock copy so dsc's own integrity check sees the lowered
/// content it actually compiles.
fn reconcile_staged_lock(
    root: &Path,
    stage: &Path,
    rewritten: &[(PathBuf, String)],
) -> Result<(), String> {
    let packages = rewritten_packages(root, rewritten);
    if packages.is_empty() {
        return Ok(());
    }
    let real_lock_path = root.join("deka.lock");
    let lock_text = fs::read_to_string(&real_lock_path).map_err(|err| {
        format!("{DEKA_VALIDATION_ERROR_MARKER}failed to read {}: {err}", real_lock_path.display())
    })?;
    let mut lock: serde_json::Value =
        serde_json::from_str(&lock_text).map_err(|err| {
            format!("{DEKA_VALIDATION_ERROR_MARKER}failed to parse {}: {err}", real_lock_path.display())
        })?;

    for (modules_dir, name) in &packages {
        let real_pkg = root.join(modules_dir).join(name);
        // The lock's hashes pin the published sources. Verify them before
        // anything compiles the lowered form; a mismatch fails closed with
        // the same remedy dsc's own check would print.
        let real = deka_host::integrity::compute_package_integrity(&real_pkg)
            .map_err(|err| format!("{DEKA_VALIDATION_ERROR_MARKER}{err}"))?;
        let mismatched = lock
            .get("packages")
            .and_then(|packages| packages.get(name))
            .and_then(|entry| entry.get(2))
            .map(|hashes| {
                hashes.pointer("/fsGraph/hash").and_then(|v| v.as_str()) != Some(real.fs_graph.as_str())
                    || hashes.pointer("/moduleGraph/hash").and_then(|v| v.as_str()) != Some(real.module_graph.as_str())
            })
            .unwrap_or(true);
        if mismatched {
            return Err(format!(
                "{DEKA_VALIDATION_ERROR_MARKER}❌ Integrity Mismatch\n\n    module '{name}' in {modules_dir}/ does not match deka.lock\n\n    = help: run `deka install` or `deka add {name}`. dsc does not fetch packages or write deka.lock."
            ));
        }

        // Lowering is semantics-preserving, so the mirror's hashes describe
        // the same package after a deterministic transform. Patching the
        // mirror lock keeps dsc's check meaningful: it still compares what
        // it compiles against the staged tree, and anything that rewrites a
        // package without passing the real-lock verification above cannot
        // reach this point.
        let staged = deka_host::integrity::compute_package_integrity(&stage.join(modules_dir).join(name))
            .map_err(|err| format!("{DEKA_VALIDATION_ERROR_MARKER}{err}"))?;
        let entry = lock
            .pointer_mut(&format!("/packages/{}/2", name.replace('/', "~1")))
            .expect("lock entry verified above");
        entry["fsGraph"]["hash"] = serde_json::Value::String(staged.fs_graph);
        entry["moduleGraph"]["hash"] = serde_json::Value::String(staged.module_graph);
    }

    // The mirror hardlinked the real lock; replace (never edit through the
    // link) so the user's lock file is untouched.
    let staged_lock = stage.join("deka.lock");
    let _ = fs::remove_file(&staged_lock);
    fs::write(&staged_lock, serde_json::to_string_pretty(&lock).map_err(|err| err.to_string())?)
        .map_err(|err| format!("failed to write {}: {err}", staged_lock.display()))?;
    Ok(())
}

/// Hardlink-copy `from` into `to`, replacing rewritten sources with their
/// lowered text. Hardlink failure (cross-device) falls back to a byte copy.
fn mirror_tree(
    from: &Path,
    to: &Path,
    rewritten: &std::collections::HashMap<PathBuf, String>,
) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|err| format!("failed to create {}: {err}", to.display()))?;
    let entries = fs::read_dir(from)
        .map_err(|err| format!("failed to read {}: {err}", from.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            if !is_skipped_dir(&entry.file_name()) {
                mirror_tree(&path, &target, rewritten)?;
            }
            continue;
        }
        if let Some(text) = rewritten.get(&path) {
            fs::write(&target, text)
                .map_err(|err| format!("failed to write {}: {err}", target.display()))?;
            continue;
        }
        match fs::hard_link(&path, &target) {
            Ok(()) => {}
            Err(_) => {
                fs::copy(&path, &target)
                    .map_err(|err| format!("failed to copy {}: {err}", path.display()))?;
            }
        };
    }
    Ok(())
}

// ---- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(src: &str) -> ScanResult {
        scan_source(src, true)
    }

    fn messages(result: &ScanResult) -> Vec<String> {
        result.diagnostics.iter().map(|(_, m)| m.clone()).collect()
    }

    #[test]
    fn safe_block_is_lowered_to_the_verbatim_call() {
        let result = scan("const n = safe { deka.bytes.len(b) }\n");
        assert!(result.diagnostics.is_empty(), "{:?}", messages(&result));
        let rewritten = result.rewritten.expect("safe site lowered");
        assert_eq!(rewritten, "const n = deka.bytes.len(b)\n");
    }

    #[test]
    fn safe_lowering_handles_multiline_and_nested_args() {
        let src = "const x = safe { deka.bytes.concat(\n  deka.bytes.from_string(\"a, }({\\\"\\\"\"),\n  other,\n) }\n";
        let result = scan(src);
        assert!(result.diagnostics.is_empty(), "{:?}", messages(&result));
        let rewritten = result.rewritten.expect("lowered");
        // Trailing comma before the closer is normalized away (dsc rejects
        // trailing commas); the call is otherwise verbatim.
        assert!(rewritten.contains("const x = deka.bytes.concat(\n  deka.bytes.from_string(\"a, }({\\\"\\\"\"),\n  other\n)\n"), "{rewritten}");
        assert!(!rewritten.contains("safe {"));
    }

    #[test]
    fn safe_statement_position_lowering() {
        let result = scan("safe { deka.io.echo(message) }\n");
        assert!(result.diagnostics.is_empty(), "{:?}", messages(&result));
        assert_eq!(result.rewritten.unwrap(), "deka.io.echo(message)\n");
    }

    #[test]
    fn unknown_kind_is_a_diagnostic() {
        let result = scan("const n = safe { deka.bogus.len(b) }\n");
        let msgs = messages(&result);
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("unknown `deka` kind `bogus`"), "{msgs:?}");
        assert!(result.rewritten.is_none());
    }

    #[test]
    fn unknown_method_is_a_diagnostic() {
        let result = scan("const n = safe { deka.bytes.bogus(b) }\n");
        let msgs = messages(&result);
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("unknown `deka.bytes` helper `bogus`"), "{msgs:?}");
    }

    #[test]
    fn wrong_arity_is_a_diagnostic() {
        let result = scan("const n = safe { deka.bytes.get(b) }\n");
        let msgs = messages(&result);
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("expected 2 argument(s), found 1"), "{msgs:?}");

        let ok = scan("const n = safe { deka.bytes.slice(b, 0, 1) }\n");
        assert!(ok.diagnostics.is_empty(), "{:?}", messages(&ok));
        let too_many = scan("const n = safe { deka.bytes.slice(b, 0, 1, 2) }\n");
        assert_eq!(too_many.diagnostics.len(), 1);
    }

    #[test]
    fn throwing_helper_under_safe_is_rejected() {
        let result = scan("const s = safe { deka.bytes.to_string(b) }\n");
        let msgs = messages(&result);
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("may throw"), "{msgs:?}");
        assert!(msgs[0].contains("unsafe"), "{msgs:?}");
    }

    #[test]
    fn unsafe_catalog_call_is_validated_not_rewritten() {
        let result = scan("const r = unsafe { deka.json.parse(s) }\n");
        assert!(result.diagnostics.is_empty(), "{:?}", messages(&result));
        assert!(result.rewritten.is_none(), "unsafe blocks pass through to dsc");
    }

    #[test]
    fn unsafe_with_type_annotation_still_validates() {
        let result = scan("const r = unsafe<object> { deka.json.parse(s) }\n");
        assert!(result.diagnostics.is_empty(), "{:?}", messages(&result));
    }

    #[test]
    fn unsafe_trailing_semicolon_is_accepted() {
        let result = scan("const r = unsafe { deka.json.parse(s); }\n");
        assert!(result.diagnostics.is_empty(), "{:?}", messages(&result));
    }

    #[test]
    fn malformed_safe_body_is_a_diagnostic() {
        let result = scan("safe { console.log(message) }\n");
        let msgs = messages(&result);
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("must be exactly one `deka.<kind>.<method>(...)`"), "{msgs:?}");
    }

    #[test]
    fn bare_catalog_call_outside_the_doors_is_a_diagnostic() {
        let result = scan("const n = deka.bytes.len(b)\n");
        let msgs = messages(&result);
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("must appear as the body of `safe { }` or `unsafe { }`"), "{msgs:?}");
    }

    #[test]
    fn deka_ui_is_not_a_catalog_kind() {
        let result = scan("const f = unsafe { deka.ui.Foo.create() }\n");
        assert!(result.diagnostics.is_empty(), "{:?}", messages(&result));
    }

    #[test]
    fn user_package_may_not_use_the_catalog() {
        for src in [
            "const n = safe { deka.bytes.len(b) }\n",
            "const r = unsafe { deka.json.parse(s) }\n",
            "const n = deka.bytes.len(b)\n",
        ] {
            let result = scan_source(src, false);
            assert_eq!(result.diagnostics.len(), 1, "{src}");
            assert!(messages(&result)[0].contains("stdlib-only"), "{src}");
            assert!(result.rewritten.is_none(), "{src}");
        }
    }

    #[test]
    fn strings_and_comments_do_not_trigger() {
        let src = "// safe { deka.bytes.len(b) } in a comment\n\
                   const s = \"safe { deka.bytes.len(b) } in a string\"\n\
                   const t = `safe { deka.bytes.len(b) } in a template`\n";
        let result = scan(src);
        assert!(result.diagnostics.is_empty(), "{:?}", messages(&result));
        assert!(result.rewritten.is_none());
    }

    #[test]
    fn template_args_with_nested_delimiters_scan_cleanly() {
        let src = "const b = safe { deka.bytes.from_string(`x${name},(})`) }\n";
        let result = scan(src);
        assert!(result.diagnostics.is_empty(), "{:?}", messages(&result));
        assert_eq!(
            result.rewritten.unwrap(),
            "const b = deka.bytes.from_string(`x${name},(})`)\n"
        );
    }

    #[test]
    fn unsafe_raw_platform_js_is_untouched() {
        let result = scan("const r = unsafe { JSON.parse(s) }\nconst v = unsafe { console.log(1) }\n");
        assert!(result.diagnostics.is_empty(), "{:?}", messages(&result));
        assert!(result.rewritten.is_none());
    }

    #[test]
    fn nested_safe_blocks_are_rejected_with_guidance() {
        let result = scan("const h = safe { deka.bytes.to_hex(safe { deka.bytes.slice(b, 1) }) }\n");
        let msgs = messages(&result);
        assert_eq!(msgs.len(), 1, "{msgs:?}");
        assert!(msgs[0].contains("nested"), "{msgs:?}");
        assert!(result.rewritten.is_none());
    }

    #[test]
    fn line_col_are_one_based() {
        let result = scan("\n\nconst n = safe { deka.bogus.len(b) }\n");
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].0, 19);
    }

    #[test]
    fn stage_guard_maps_paths_both_ways() {
        let tmp = std::env::temp_dir().join(format!("deka-scan-{}", std::process::id()));
        let root = tmp.join("root");
        let stage = tmp.join("stage");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("main.ds"), "export const n = 1\n").unwrap();
        let guard = StageGuard {
            dir: stage.clone(),
            source_root: root.canonicalize().unwrap_or(root.clone()),
        };
        assert_eq!(guard.map_path(&root.join("main.ds")), stage.join("main.ds"));
        // unmap canonicalizes its input, so the result carries the canonical
        // (/private/var) root spelling on macOS.
        let expected = root.canonicalize().unwrap_or(root.clone()).join("main.ds");
        assert_eq!(guard.unmap_path(&stage.join("main.ds")), expected);
        let diag = guard.remap_diagnostic(&format!("{}:1:1: bad", stage.display()));
        assert!(diag.contains(&root.display().to_string()), "{diag}");
        drop(guard);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prepare_compile_root_lowers_safe_via_stage_and_cleans_up() {
        let tmp = std::env::temp_dir().join(format!("deka-stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pkg = tmp.join("ds_modules").join("@deka").join("demo");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(tmp.join("deka.json"), r#"{"name":"app"}"#).unwrap();
        fs::write(pkg.join("deka.json"), r#"{"name":"@deka/demo","version":"1.0.0"}"#).unwrap();
        fs::write(pkg.join("index.ds"), "export const n = safe { deka.time.now() }\n").unwrap();
        fs::write(tmp.join("main.ds"), "import { n } from \"demo\"\nexport const total = n\n").unwrap();
        // dsc verifies locked dependencies against deka.lock; staging rewrites
        // the package's sources, so the mirror's lock copy is re-pinned to the
        // lowered content after the real tree verified against the real lock.
        let integrity =
            deka_host::integrity::compute_package_integrity(&pkg).expect("package integrity");
        let lock = serde_json::json!({
            "lockfileVersion": 1,
            "packages": {
                "@deka/demo": [
                    "@deka/demo@1.0.0",
                    "linkhash:@deka/demo",
                    {
                        "repo": "https://github.com/dekaruntime/deka.git",
                        "gitRef": "v1.0.0",
                        "source": "deka.gg",
                        "dependencies": [],
                        "moduleGraph": { "algo": "sha256", "hash": integrity.module_graph },
                        "fsGraph": { "algo": "sha256", "hash": integrity.fs_graph }
                    },
                    ""
                ]
            }
        });
        let real_lock = serde_json::to_string_pretty(&lock).unwrap();
        fs::write(tmp.join("deka.lock"), &real_lock).unwrap();

        let prepared = prepare_compile_root(&tmp).expect("prepare");
        let PreparedRoot::Staged(guard) = prepared else {
            panic!("safe block must stage the compile");
        };
        // The user tree is untouched; the mirror carries the lowered source.
        let original = fs::read_to_string(pkg.join("index.ds")).unwrap();
        assert!(original.contains("safe {"), "user tree never modified");
        let lowered = fs::read_to_string(guard.map_path(&pkg.join("index.ds"))).unwrap();
        assert_eq!(lowered, "export const n = deka.time.now()\n");
        assert!(guard.root().join("main.ds").is_file(), "entry mirrored");
        // The user's lock is byte-identical after staging — the mirror holds
        // a real copy, never an edit through the hardlink.
        assert_eq!(fs::read_to_string(tmp.join("deka.lock")).unwrap(), real_lock);
        // The mirror's lock copy is re-pinned to the lowered content, which
        // is exactly what dsc will hash when it verifies the package.
        let staged_lock: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(guard.root().join("deka.lock")).unwrap(),
        )
        .unwrap();
        let staged_hash = staged_lock["packages"]["@deka/demo"][2]["fsGraph"]["hash"]
            .as_str()
            .unwrap();
        let staged_pkg = guard.map_path(&pkg);
        let staged_integrity =
            deka_host::integrity::compute_package_integrity(&staged_pkg).unwrap();
        assert_eq!(staged_hash, staged_integrity.fs_graph);
        assert_ne!(staged_hash, integrity.fs_graph, "lowered content hashes differently");
        let stage_dir = guard.root().to_path_buf();
        drop(guard);
        assert!(!stage_dir.exists(), "stage mirror removed on drop");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prepare_compile_root_rejects_rewritten_package_that_does_not_match_lock() {
        let tmp = std::env::temp_dir().join(format!("deka-lockneg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pkg = tmp.join("ds_modules").join("@deka").join("demo");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(tmp.join("deka.json"), r#"{"name":"app"}"#).unwrap();
        fs::write(pkg.join("deka.json"), r#"{"name":"@deka/demo","version":"1.0.0"}"#).unwrap();
        fs::write(pkg.join("index.ds"), "export const n = safe { deka.time.now() }\n").unwrap();
        fs::write(tmp.join("main.ds"), "import { n } from \"demo\"\nexport const total = n\n").unwrap();
        // Pin the lock against different (published) content, then let the
        // on-disk package drift: staging must fail closed, not re-pin a
        // tampered package.
        let integrity =
            deka_host::integrity::compute_package_integrity(&pkg).expect("package integrity");
        fs::write(
            tmp.join("deka.lock"),
            serde_json::to_string(&serde_json::json!({
                "lockfileVersion": 1,
                "packages": {
                    "@deka/demo": [
                        "@deka/demo@1.0.0",
                        "linkhash:@deka/demo",
                        {
                            "moduleGraph": { "algo": "sha256", "hash": integrity.module_graph },
                            "fsGraph": { "algo": "sha256", "hash": integrity.fs_graph }
                        },
                        ""
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(pkg.join("index.ds"), "export const n = safe { deka.time.now() }\nexport const drift = 1\n").unwrap();

        let err = prepare_compile_root(&tmp).expect_err("tampered package must not stage");
        assert!(
            err.contains("Integrity Mismatch") && err.contains("@deka/demo"),
            "{err}"
        );
        // A failed staging must not leave a mirror behind.
        let leftover = fs::read_dir(tmp.join(".cache"))
            .map(|entries| entries.flatten().count())
            .unwrap_or(0);
        assert_eq!(leftover, 0, "failed staging must clean up its mirror: {err}");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prepare_compile_root_without_safe_compiles_in_place() {
        let tmp = std::env::temp_dir().join(format!("deka-inplace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("deka.json"), r#"{"name":"app"}"#).unwrap();
        fs::write(tmp.join("main.ds"), "export const n = 1\n").unwrap();
        let prepared = prepare_compile_root(&tmp).expect("prepare");
        assert!(matches!(prepared, PreparedRoot::InPlace));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prepare_compile_root_reports_source_diagnostics_with_locations() {
        let tmp = std::env::temp_dir().join(format!("deka-diag-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let pkg = tmp.join("ds_modules").join("@deka").join("demo");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("deka.json"), r#"{"name":"@deka/demo"}"#).unwrap();
        fs::write(pkg.join("index.ds"), "export const n = safe { deka.bytes.bogus(b) }\n").unwrap();
        let err = prepare_compile_root(&tmp).expect_err("diagnostic");
        assert!(err.contains(&pkg.join("index.ds").display().to_string()), "{err}");
        assert!(err.contains(":1:25:"), "{err}");
        assert!(err.contains("unknown `deka.bytes` helper `bogus`"), "{err}");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prepare_compile_root_flags_user_package_catalog_use() {
        let tmp = std::env::temp_dir().join(format!("deka-user-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("deka.json"), r#"{"name":"app"}"#).unwrap();
        fs::write(tmp.join("main.ds"), "const n = unsafe { deka.json.parse(s) }\nexport const x = n\n").unwrap();
        let err = prepare_compile_root(&tmp).expect_err("diagnostic");
        assert!(err.contains("stdlib-only"), "{err}");
        let _ = fs::remove_dir_all(&tmp);
    }
}
