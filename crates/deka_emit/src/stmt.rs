//! Statement emission.

use deka_syntax::{ExportDecl, Stmt};

use crate::expr::emit_expr;
use crate::util::write_indent;

pub fn emit_stmt(out: &mut String, stmt: &Stmt, indent: usize) -> Result<(), String> {
    match stmt {
        Stmt::Const { name, value, .. } => {
            write_indent(out, indent);
            out.push_str("const ");
            out.push_str(name);
            out.push_str(" = ");
            emit_expr(out, value)?;
            out.push_str(";");
        }
        Stmt::Let { name, value, .. } => {
            write_indent(out, indent);
            out.push_str("let ");
            out.push_str(name);
            out.push_str(" = ");
            emit_expr(out, value)?;
            out.push_str(";");
        }
        Stmt::Function {
            name,
            params,
            body,
            ..
        } => {
            write_indent(out, indent);
            out.push_str("function ");
            out.push_str(name);
            out.push('(');
            for (i, param) in params.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(param.name);
            }
            out.push_str(") {\n");
            for stmt in body.iter() {
                emit_stmt(out, stmt, indent + 2)?;
                out.push('\n');
            }
            write_indent(out, indent);
            out.push('}');
        }
        Stmt::Expr { expr, .. } => {
            write_indent(out, indent);
            emit_expr(out, expr)?;
            out.push_str(";");
        }
        Stmt::Return { value, .. } => {
            write_indent(out, indent);
            out.push_str("return");
            if let Some(value) = value {
                out.push(' ');
                emit_expr(out, value)?;
            }
            out.push_str(";");
        }
        Stmt::Export { decl, .. } => {
            write_indent(out, indent);
            out.push_str("export ");
            match decl {
                ExportDecl::Const { name, value, .. } => {
                    out.push_str("const ");
                    out.push_str(name);
                    out.push_str(" = ");
                    emit_expr(out, value)?;
                    out.push_str(";");
                }
                ExportDecl::Function {
                    name,
                    params,
                    body,
                    ..
                } => {
                    out.push_str("function ");
                    out.push_str(name);
                    out.push('(');
                    for (i, param) in params.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(param.name);
                    }
                    out.push_str(") {\n");
                    for stmt in body.iter() {
                        emit_stmt(out, stmt, indent + 2)?;
                        out.push('\n');
                    }
                    write_indent(out, indent);
                    out.push('}');
                }
            }
        }
        Stmt::Import { specifiers, source, .. } => {
            write_indent(out, indent);
            if specifiers.is_empty() {
                // Side-effect import.
                out.push_str("import \"");
                out.push_str(source);
                out.push_str("\";");
            } else {
                out.push_str("import { ");
                for (i, spec) in specifiers.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    if spec.imported == spec.local {
                        out.push_str(spec.imported);
                    } else {
                        out.push_str(spec.imported);
                        out.push_str(" as ");
                        out.push_str(spec.local);
                    }
                }
                out.push_str(" } from \"");
                out.push_str(source);
                out.push_str("\";");
            }
        }
        Stmt::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            write_indent(out, indent);
            out.push_str("if (");
            emit_expr(out, condition)?;
            out.push_str(") {\n");
            for stmt in then_body.iter() {
                emit_stmt(out, stmt, indent + 2)?;
                out.push('\n');
            }
            write_indent(out, indent);
            out.push('}');
            if !else_body.is_empty() {
                out.push_str(" else {\n");
                for stmt in else_body.iter() {
                    emit_stmt(out, stmt, indent + 2)?;
                    out.push('\n');
                }
                write_indent(out, indent);
                out.push('}');
            }
        }
        Stmt::For {
            init,
            condition,
            step,
            body,
            ..
        } => {
            write_indent(out, indent);
            out.push_str("for (");
            if let Some(init) = init {
                emit_for_init(out, init)?;
            }
            out.push_str("; ");
            if let Some(condition) = condition {
                emit_expr(out, condition)?;
            }
            out.push_str("; ");
            if let Some(step) = step {
                emit_expr(out, step)?;
            }
            out.push_str(") {\n");
            for stmt in body.iter() {
                emit_stmt(out, stmt, indent + 2)?;
                out.push('\n');
            }
            write_indent(out, indent);
            out.push('}');
        }
        Stmt::Struct { .. } | Stmt::Enum { .. } | Stmt::TypeAlias { .. } => {
            // Type declarations are erased at runtime.
        }
        Stmt::ReceiverMethod {
            receiver_type,
            name,
            params,
            body,
            ..
        } => {
            write_indent(out, indent);
            out.push_str("function ");
            out.push_str(receiver_type);
            out.push('_');
            out.push_str(name);
            out.push_str("(this");
            for param in params.iter() {
                out.push_str(", ");
                out.push_str(param.name);
            }
            out.push_str(") {\n");
            for stmt in body.iter() {
                emit_stmt(out, stmt, indent + 2)?;
                out.push('\n');
            }
            write_indent(out, indent);
            out.push('}');
        }
    }
    Ok(())
}

fn emit_for_init(out: &mut String, init: &deka_syntax::ForInit) -> Result<(), String> {
    match init {
        deka_syntax::ForInit::Const { name, value, .. } => {
            out.push_str("const ");
            out.push_str(name);
            out.push_str(" = ");
            emit_expr(out, value)?;
        }
        deka_syntax::ForInit::Let { name, value, .. } => {
            out.push_str("let ");
            out.push_str(name);
            out.push_str(" = ");
            emit_expr(out, value)?;
        }
        deka_syntax::ForInit::Expr(expr) => {
            emit_expr(out, expr)?;
        }
    }
    Ok(())
}
