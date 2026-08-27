//! match-expression emission.

use deka_syntax::{Expr, MatchArm, Pattern};

use crate::expr::emit_expr;
use crate::util::write_indent;

pub fn emit_match(out: &mut String, scrutinee: &Expr, arms: &[MatchArm]) -> Result<(), String> {
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
