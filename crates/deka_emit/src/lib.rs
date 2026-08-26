//! DekaScript JavaScript emitter (Compiler v2).
//!
//! Emits reasonably formatted JavaScript from the v2 AST, erasing all type
//! annotations.

use deka_syntax::{BinOp, Expr, ExportDecl, MatchArm, Pattern, Program, Stmt, UnOp};

/// Emit JavaScript for a parsed and type-checked program.
pub fn emit_js(program: &Program, _source: &str) -> Result<String, String> {
    let mut out = String::new();
    for (i, stmt) in program.statements.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        emit_stmt(&mut out, stmt, 0)?;
    }
    Ok(out)
}

fn emit_stmt(out: &mut String, stmt: &Stmt, indent: usize) -> Result<(), String> {
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

fn emit_expr(out: &mut String, expr: &Expr) -> Result<(), String> {
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
            emit_expr(out, left)?;
            out.push(' ');
            out.push_str(bin_op_str(*op));
            out.push(' ');
            emit_expr(out, right)?;
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
        Expr::Match {
            scrutinee,
            arms,
            ..
        } => {
            emit_match(out, scrutinee, arms)?;
        }
        Expr::Pipe { .. }
        | Expr::Await { .. }
        | Expr::JsxElement { .. }
        | Expr::JsxFragment { .. } => {
            return Err(format!("unsupported expression: {:?}", expr));
        }
    }
    Ok(())
}

fn emit_match(out: &mut String, scrutinee: &Expr, arms: &[MatchArm]) -> Result<(), String> {
    let scrutinee_var = "__deka_scrutinee";
    out.push_str("((");
    out.push_str(scrutinee_var);
    out.push_str(") => {\n");

    for (i, arm) in arms.iter().enumerate() {
        let is_last = i == arms.len() - 1;
        emit_match_arm(out, arm, scrutinee_var, is_last, 2)?;
    }

    out.push_str("  throw new Error(\"non-exhaustive match\");\n");
    out.push_str("})(");
    emit_expr(out, scrutinee)?;
    out.push_str(")");
    Ok(())
}

fn emit_match_arm(
    out: &mut String,
    arm: &MatchArm,
    scrutinee_var: &str,
    is_last: bool,
    indent: usize,
) -> Result<(), String> {
    let condition = match_condition(&arm.pattern, scrutinee_var);

    if condition == "true" && is_last {
        // Last wildcard arm: just bind and return.
        emit_pattern_bindings(out, &arm.pattern, scrutinee_var, indent)?;
        write_indent(out, indent);
        out.push_str("return ");
        emit_expr(out, &arm.body)?;
        out.push_str(";\n");
        return Ok(());
    }

    write_indent(out, indent);
    out.push_str("if (");
    out.push_str(&condition);
    out.push_str(") {\n");
    emit_pattern_bindings(out, &arm.pattern, scrutinee_var, indent + 2)?;
    write_indent(out, indent + 2);
    out.push_str("return ");
    emit_expr(out, &arm.body)?;
    out.push_str(";\n");
    write_indent(out, indent);
    out.push_str("}\n");
    Ok(())
}

fn match_condition(pattern: &Pattern, scrutinee_var: &str) -> String {
    match pattern {
        Pattern::Wildcard { .. } => "true".to_string(),
        Pattern::Identifier { .. } => "true".to_string(),
        Pattern::Literal { expr, .. } => {
            let mut literal = String::new();
            // Literal patterns can reuse expression emission; they are always
            // simple literals, so this is safe.
            emit_expr(&mut literal, expr).expect("literal emission");
            format!("{} === {}", scrutinee_var, literal)
        }
        Pattern::Constructor { name, .. } => {
            format!("{}.__case === \"{}\"", scrutinee_var, name)
        }
        Pattern::Struct { .. } | Pattern::Tuple { .. } => {
            // Not yet supported; emit a failing condition so the arm is skipped.
            "false".to_string()
        }
    }
}

fn emit_pattern_bindings(
    out: &mut String,
    pattern: &Pattern,
    scrutinee_var: &str,
    indent: usize,
) -> Result<(), String> {
    match pattern {
        Pattern::Wildcard { .. } => {}
        Pattern::Identifier { name, .. } => {
            write_indent(out, indent);
            out.push_str("const ");
            out.push_str(name);
            out.push_str(" = ");
            out.push_str(scrutinee_var);
            out.push_str(";\n");
        }
        Pattern::Literal { .. } => {}
        Pattern::Constructor { name, payload, .. } => {
            if let Some(payload) = payload {
                let payload_access = if *name == "None" {
                    scrutinee_var.to_string()
                } else {
                    format!("{}.value", scrutinee_var)
                };
                emit_pattern_bindings(out, payload, &payload_access, indent)?;
            }
        }
        Pattern::Struct { .. } | Pattern::Tuple { .. } => {}
    }
    Ok(())
}

fn bin_op_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
    }
}

fn un_op_str(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
    }
}

fn write_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push(' ');
    }
}

fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bumpalo::Bump;
    use deka_syntax::parse;

    fn parse_and_emit(source: &str) -> String {
        let arena = Bump::new();
        let result = parse(source, &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.expect("parse produced no program");
        emit_js(&program, source).expect("emit failed")
    }

    #[test]
    fn emit_const_number() {
        let out = parse_and_emit("const x = 42;");
        assert!(out.contains("const x = 42;"), "got: {}", out);
    }

    #[test]
    fn emit_function_with_return() {
        let out = parse_and_emit("function add(a: number, b: number): number { return a + b; }");
        assert!(out.contains("function add(a, b) {"), "got: {}", out);
        assert!(out.contains("return a + b;"), "got: {}", out);
    }

    #[test]
    fn emit_call_expression() {
        let out = parse_and_emit("console.log(\"hello\");");
        assert!(out.contains("console.log(\"hello\");"), "got: {}", out);
    }

    #[test]
    fn emit_match_expression() {
        let out = parse_and_emit(
            "const o = Some(5); const x = match o { Some(n) => n, None => 0 };",
        );
        assert!(out.contains("__case"), "expected case dispatch, got: {}", out);
        assert!(out.contains("Some"), "got: {}", out);
        assert!(out.contains("None"), "got: {}", out);
    }

    #[test]
    fn emit_enum_constructor() {
        let out = parse_and_emit("const o = Some(5);");
        assert!(out.contains("__case"), "expected case tag, got: {}", out);
        assert!(out.contains("Some"), "got: {}", out);
    }

    #[test]
    fn emit_struct_literal() {
        let out = parse_and_emit(
            "struct Point { x: number, y: number } const p = Point { x: 1, y: 2 };",
        );
        assert!(out.contains("x: 1"), "got: {}", out);
        assert!(out.contains("y: 2"), "got: {}", out);
    }

    #[test]
    fn emit_user_defined_enum_constructor() {
        let out = parse_and_emit("enum Color { Red, Green, Blue } const c = Color.Red;");
        assert!(out.contains("__case"), "expected case tag, got: {}", out);
        assert!(out.contains("Red"), "got: {}", out);
    }

    #[test]
    fn emit_user_defined_enum_payload_constructor() {
        let out = parse_and_emit("enum Shape { Circle(number) } const s = Shape.Circle(5);");
        assert!(out.contains("__case"), "expected case tag, got: {}", out);
        assert!(out.contains("Circle"), "got: {}", out);
        assert!(out.contains("value: 5"), "got: {}", out);
    }

    #[test]
    fn emit_receiver_method() {
        let out = parse_and_emit(
            "struct Point { x: number, y: number } fn Point.distance(other: Point): number { return 0; } const p1 = Point { x: 0, y: 0 }; const p2 = Point { x: 3, y: 4 }; const d = p1.distance(p2);",
        );
        assert!(out.contains("function Point_distance"), "got: {}", out);
        assert!(out.contains("p1.distance(p2)"), "got: {}", out);
    }

    #[test]
    fn emit_import_named() {
        let out = parse_and_emit("import { add } from \"./math.ds\";");
        assert!(out.contains("import { add } from \"./math.ds\";"), "got: {}", out);
    }

    #[test]
    fn emit_import_aliased() {
        let out = parse_and_emit("import { add as plus } from \"./math.ds\";");
        assert!(out.contains("import { add as plus } from \"./math.ds\";"), "got: {}", out);
    }

    #[test]
    fn emit_import_side_effect() {
        let out = parse_and_emit("import \"./side-effects.ds\";");
        assert!(out.contains("import \"./side-effects.ds\";"), "got: {}", out);
    }

    #[test]
    fn emit_export_const() {
        let out = parse_and_emit("export const x: number = 42;");
        assert!(out.contains("export const x = 42;"), "got: {}", out);
    }

    #[test]
    fn emit_export_function() {
        let out = parse_and_emit("export function add(a: number, b: number): number { return a + b; }");
        assert!(out.contains("export function add(a, b) {"), "got: {}", out);
        assert!(out.contains("return a + b;"), "got: {}", out);
    }

    #[test]
    fn emit_array_literal() {
        let out = parse_and_emit("const a = [1, 2, 3];");
        assert!(out.contains("const a = [1, 2, 3];"), "got: {}", out);
    }

    #[test]
    fn emit_array_spread() {
        let out = parse_and_emit("const a = [...b];");
        assert!(out.contains("const a = [...b];"), "got: {}", out);
    }

    #[test]
    fn emit_object_literal() {
        let out = parse_and_emit("const o = { a: 1, b: \"two\" };");
        assert!(out.contains("const o = {a: 1, b: \"two\"};"), "got: {}", out);
    }

    #[test]
    fn emit_object_spread() {
        let out = parse_and_emit("const o = { ...base, x: 1 };");
        assert!(out.contains("const o = {...base, x: 1};"), "got: {}", out);
    }

    #[test]
    fn emit_index_access() {
        let out = parse_and_emit("const x = arr[0];");
        assert!(out.contains("const x = arr[0];"), "got: {}", out);
    }
}
