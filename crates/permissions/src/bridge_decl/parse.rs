//! DekaScript declaration scanner for `bridge_decl` (deka#620): tokenizer
//! plus a tolerant function-signature parser. Moved out of `bridge_decl.rs`
//! (deka#391 file-size gate). The parser deliberately accepts only the
//! declaration-shaped subset of the language and abandons anything else, so
//! unfamiliar syntax degrades to "not a declaration" instead of aborting a
//! file; the diff in the parent module decides what a declaration means.

// ---- Declaration model ------------------------------------------------------

/// A parsed `bridge kind.action(args)` call and the signature of the function
/// that declares it.
pub(super) struct Declaration {
    pub(super) line: usize,
    pub(super) export: String,
    pub(super) export_async: bool,
    pub(super) params: Vec<(String, Type)>,
    /// `None` when the enclosing function has no parseable return type.
    pub(super) return_type: Option<Type>,
    pub(super) awaited: bool,
    pub(super) kind: String,
    pub(super) action: String,
    pub(super) call_arg_count: usize,
    pub(super) body_start: usize,
    pub(super) body_end: usize,
}

/// A parsed DS type expression: dotted name (`Result`, `FsError`) plus
/// generic arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Type {
    pub(super) name: String,
    pub(super) args: Vec<Type>,
}

impl Type {
    pub(super) fn render(&self) -> String {
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
pub(super) struct Token {
    pub(super) text: String,
    pub(super) line: usize,
}

/// Tokenize `.ds` source, dropping whitespace and `//` line comments while
/// respecting string and template-string literals. Comment text mentioning
/// `bridge` (every published package has some) must not scan as a call.
pub(super) fn tokenize(source: &str) -> Result<Vec<Token>, String> {
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
pub(super) fn parse_declarations(tokens: &[Token]) -> Vec<Declaration> {
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
