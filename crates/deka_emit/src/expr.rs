//! Expression emission.

use deka_syntax::{BinOp, Expr};

use crate::util::{bin_op_str, escape_string, un_op_str};

pub fn emit_expr(out: &mut String, expr: &Expr) -> Result<(), String> {
    match expr {
        Expr::Number { value, .. } => {
            if value.is_nan() {
                out.push_str("NaN");
            } else if value.is_infinite() {
                if value.is_sign_negative() {
                    out.push_str("-Infinity");
                } else {
                    out.push_str("Infinity");
                }
            } else if *value == 0.0 && value.is_sign_negative() {
                out.push_str("-0");
            } else {
                out.push_str(&format!("{}", value));
            }
        }
        Expr::BigInt { value, .. } => {
            out.push_str(value);
            out.push('n');
        }
        Expr::String { value, .. } => {
            out.push('"');
            out.push_str(&escape_string(value));
            out.push('"');
        }
        Expr::Boolean { value, .. } => {
            out.push_str(if *value { "true" } else { "false" });
        }
        Expr::None { .. } => {
            out.push_str("null");
        }
        Expr::Identifier { name, .. } => {
            out.push_str(name);
        }
        Expr::Binary { op, left, right, .. } => {
            if *op == BinOp::Pipe {
                // Simple pipe: `left |> right` becomes `(right)(left)`.
                out.push('(');
                emit_expr(out, right)?;
                out.push_str(")(");
                emit_expr(out, left)?;
                out.push(')');
            } else {
                emit_expr(out, left)?;
                out.push(' ');
                out.push_str(bin_op_str(*op));
                out.push(' ');
                emit_expr(out, right)?;
            }
        }
        Expr::Unary { op, operand, .. } => {
            out.push_str(un_op_str(*op));
            emit_expr(out, operand)?;
        }
        Expr::Call { callee, args, .. } => {
            emit_expr(out, callee)?;
            out.push('(');
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                emit_expr(out, arg)?;
            }
            out.push(')');
        }
        Expr::FieldAccess { object, field, .. } => {
            emit_expr(out, object)?;
            out.push('.');
            out.push_str(field);
        }
        Expr::StructLiteral { fields, .. } => {
            out.push_str("({ ");
            for (i, field) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(field.name);
                out.push_str(": ");
                emit_expr(out, &field.value)?;
            }
            out.push_str(" })");
        }
        Expr::IndexAccess { object, index, .. } => {
            emit_expr(out, object)?;
            out.push('[');
            emit_expr(out, index)?;
            out.push(']');
        }
        Expr::Paren { expr, .. } => {
            out.push('(');
            emit_expr(out, expr)?;
            out.push(')');
        }
        Expr::Array { elements, .. } => {
            out.push('[');
            for (i, element) in elements.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                emit_expr(out, element)?;
            }
            out.push(']');
        }
        Expr::Object { fields, .. } => {
            out.push_str("{");
            for (i, field) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                // Empty key is the sentinel used by the parser for `{ ...obj }`.
                if field.key.is_empty() {
                    out.push_str("...");
                    emit_expr(out, &field.value)?;
                } else {
                    out.push_str(&field.key);
                    out.push_str(": ");
                    emit_expr(out, &field.value)?;
                }
            }
            out.push_str("}");
        }
        Expr::Spread { expr, .. } => {
            out.push_str("...");
            emit_expr(out, expr)?;
        }
        Expr::Unsafe { source, .. } => {
            emit_unsafe(out, source)?;
        }
        Expr::EnumConstructor {
            case_name,
            payload,
            ..
        } => {
            out.push_str("({ __case: \"");
            out.push_str(case_name);
            out.push_str("\"");
            if let Some(payload) = payload {
                out.push_str(", value: ");
                emit_expr(out, payload)?;
            }
            out.push_str(" })");
        }
        Expr::Match { scrutinee, arms, .. } => {
            crate::r#match::emit_match(out, scrutinee, arms)?;
        }
        Expr::Await { expr, .. } => {
            out.push_str("await ");
            emit_expr(out, expr)?;
        }
        Expr::JsxElement { element, .. } => {
            emit_jsx_element(out, element)?;
        }
        Expr::JsxFragment { children, .. } => {
            emit_jsx_fragment(out, children)?;
        }
        Expr::JsxText { value, .. } => {
            out.push('"');
            out.push_str(&crate::util::escape_string(value));
            out.push('"');
        }
        Expr::TemplateLiteral { parts, .. } => {
            out.push('`');
            for part in parts.iter() {
                match part {
                    deka_syntax::TemplatePart::Text(text) => out.push_str(text),
                    deka_syntax::TemplatePart::Expr(expr) => {
                        out.push_str("${");
                        emit_expr(out, expr)?;
                        out.push('}');
                    }
                }
            }
            out.push('`');
        }
        Expr::Function { params, body, is_async, .. } => {
            if *is_async {
                out.push_str("async function(");
            } else {
                out.push_str("function(");
            }
            for (i, param) in params.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(param.name);
            }
            out.push_str(") {\n");
            for stmt in body.iter() {
                crate::stmt::emit_stmt(out, stmt, 2)?;
                out.push('\n');
            }
            out.push('}');
        }
    }
    Ok(())
}

/// Emit an `unsafe { ... }` block as a JavaScript IIFE wrapped in try/catch.
///
/// The raw source inside the braces is passed through verbatim. The wrapper
/// returns a `Result<T, E>` enum value (`{ __case: "Ok", value: ... }` or
/// `{ __case: "Err", value: err }`). If the source contains top-level `await`,
/// the wrapper is async and the value is awaited.
fn emit_unsafe(out: &mut String, source: &str) -> Result<(), String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        out.push_str("(function() { try { return { __case: \"Ok\", value: undefined }; } catch (err) { return { __case: \"Err\", value: err }; } })()");
        return Ok(());
    }

    let is_async = js_has_top_level_await(trimmed);
    let is_statement_block = raw_js_looks_like_statements(trimmed);

    let fn_kw = if is_async { "async function" } else { "function" };
    let inner = if is_statement_block {
        format!("({fn_kw}() {{ {trimmed} }})()")
    } else {
        format!("({fn_kw}() {{ return ({trimmed}); }})()")
    };

    let awaited = if is_async {
        format!("await {inner}")
    } else {
        inner
    };

    out.push('(');
    out.push_str(fn_kw);
    out.push_str("() { try { return { __case: \"Ok\", value: ");
    out.push_str(&awaited);
    out.push_str(" }; } catch (err) { return { __case: \"Err\", value: err }; } })()");

    Ok(())
}

/// Heuristic to decide whether raw JavaScript inside an `unsafe { ... }` block
/// should be treated as a statement block or a single expression.
fn raw_js_looks_like_statements(raw: &str) -> bool {
    if raw.contains(';') {
        return true;
    }
    let head = raw
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| c == '(' || c == '{' || c == '[');
    matches!(
        head,
        "const"
            | "let"
            | "var"
            | "function"
            | "class"
            | "if"
            | "for"
            | "while"
            | "do"
            | "try"
            | "switch"
            | "return"
            | "throw"
            | "break"
            | "continue"
            | "with"
            | "debugger"
            | "import"
            | "export"
            | "async"
    )
}

/// Heuristic to detect a top-level `await` keyword in raw JavaScript.
fn js_has_top_level_await(raw: &str) -> bool {
    raw.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| word == "await")
}

/// Emit a JSX element as a `deka.ui.jsx` or `deka.ui.jsxs` runtime call.
fn emit_jsx_element(out: &mut String, element: &deka_syntax::JsxElement) -> Result<(), String> {
    let is_component = element
        .tag
        .chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false);
    let tag_expr = if is_component {
        element.tag.to_string()
    } else {
        format!("\"{}\"", crate::util::escape_string(element.tag))
    };

    let mut props = Vec::new();
    for attr in element.attributes.iter() {
        if attr.name.is_empty() {
            // Spread attribute: `{...expr}` stored with empty name.
            if let Some(value) = &attr.value {
                props.push(format!("...",));
                let mut buf = String::new();
                emit_expr(&mut buf, value)?;
                props.last_mut().unwrap().push_str(&buf);
            }
        } else {
            let value = match &attr.value {
                Some(v) => {
                    let mut buf = String::new();
                    emit_expr(&mut buf, v)?;
                    buf
                }
                None => "true".to_string(),
            };
            props.push(format!(
                "\"{}\": {}",
                crate::util::escape_string(attr.name),
                value
            ));
        }
    }

    let mut child_values = Vec::new();
    for child in element.children.iter() {
        let mut buf = String::new();
        emit_expr(&mut buf, child)?;
        child_values.push(buf);
    }

    if !child_values.is_empty() {
        if child_values.len() == 1 {
            props.push(format!("\"children\": {}", child_values[0]));
        } else {
            props.push(format!("\"children\": [{}]", child_values.join(", ")));
        }
    }

    let fn_name = if child_values.len() > 1 { "jsxs" } else { "jsx" };
    out.push_str("deka.ui.");
    out.push_str(fn_name);
    out.push('(');
    out.push_str(&tag_expr);
    out.push_str(", {");
    out.push_str(&props.join(", "));
    out.push_str("})");

    Ok(())
}

/// Emit a JSX fragment as a `deka.ui.jsx` call with `deka.ui.Fragment`.
fn emit_jsx_fragment(out: &mut String, children: &[deka_syntax::Expr]) -> Result<(), String> {
    let mut child_values = Vec::new();
    for child in children.iter() {
        let mut buf = String::new();
        emit_expr(&mut buf, child)?;
        child_values.push(buf);
    }

    let fn_name = if child_values.len() > 1 { "jsxs" } else { "jsx" };
    out.push_str("deka.ui.");
    out.push_str(fn_name);
    out.push_str("(deka.ui.Fragment, {");
    if !child_values.is_empty() {
        if child_values.len() == 1 {
            out.push_str("\"children\": ");
            out.push_str(&child_values[0]);
        } else {
            out.push_str("\"children\": [");
            out.push_str(&child_values.join(", "));
            out.push(']');
        }
    }
    out.push_str("})");

    Ok(())
}
