//! Expression emission.

use deka_syntax::{BinOp, Expr};

use crate::stmt::emit_stmt;
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
        Expr::Unsafe { body, .. } => {
            // Block expression encountered outside a function body; wrap in an
            // IIFE to preserve statement semantics.
            out.push_str("(() => {\n");
            for stmt in body.iter() {
                emit_stmt(out, stmt, 2)?;
                out.push('\n');
            }
            out.push_str("})()");
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
