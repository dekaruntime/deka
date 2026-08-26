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
        Expr::JsxElement { .. } | Expr::JsxFragment { .. } => {
            return Err(format!("unsupported expression: {:?}", expr));
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
