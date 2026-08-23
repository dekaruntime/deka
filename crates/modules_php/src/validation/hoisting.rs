//! Hoisting-related validation for DekaScript.
//!
//! DekaScript hoists `fn`, `struct`, `enum`, and `type` declarations but keeps
//! `let` and `const` in source order. This module catches the remaining
//! footguns that the parser cannot easily reject, such as a `const`
//! initializer referencing a binding declared later in the same scope.

use php_rs::parser::ast::{
    Expr, ExprId, JsxChild, Name, ObjectKey, Program, Stmt, StmtId,
};
use php_rs::parser::span::Span;
use std::collections::HashMap;

use super::{ErrorKind, Severity, ValidationError, ValidationWarning};

/// Collect bare identifier references from an expression. This is a shallow
/// but broad traversal covering the common expression forms where a forward
/// reference to a value binding can occur.
fn collect_expr_refs(source: &[u8], expr: ExprId<'_>, out: &mut Vec<(String, Span)>) {
    fn span_text(source: &[u8], span: Span) -> String {
        String::from_utf8_lossy(&source[span.start..span.end]).to_string()
    }

    fn token_text(source: &[u8], tok: &php_rs::parser::lexer::token::Token) -> String {
        String::from_utf8_lossy(tok.text(source)).to_string()
    }

    fn name_text(source: &[u8], name: &Name<'_>) -> String {
        name.parts
            .last()
            .map(|t| token_text(source, t))
            .unwrap_or_default()
    }

    match expr {
        Expr::Variable { name, span } => {
            out.push((span_text(source, *name), *span));
        }
        Expr::Call { func, args, .. } => {
            collect_expr_refs(source, *func, out);
            for arg in args.iter() {
                collect_expr_refs(source, arg.value, out);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_expr_refs(source, *left, out);
            collect_expr_refs(source, *right, out);
        }
        Expr::Unary { expr: inner, .. } => collect_expr_refs(source, *inner, out),
        Expr::Array { items, .. } => {
            for item in items.iter() {
                collect_expr_refs(source, item.value, out);
                if let Some(key) = item.key {
                    collect_expr_refs(source, key, out);
                }
            }
        }
        Expr::ObjectLiteral { items, .. } => {
            for item in items.iter() {
                collect_expr_refs(source, item.value, out);
                // Object keys are literals, not value references.
                let _ = item.key;
            }
        }
        Expr::JsxElement {
            name,
            attributes,
            children,
            ..
        } => {
            let tag = name_text(source, name);
            if tag.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                out.push((tag, name.span));
            }
            for attr in attributes.iter() {
                if let Some(value) = attr.value {
                    collect_expr_refs(source, value, out);
                }
            }
            for child in children.iter() {
                if let JsxChild::Expr(e) = child {
                    collect_expr_refs(source, *e, out);
                }
            }
        }
        Expr::ArrayDimFetch { array, dim, .. } => {
            collect_expr_refs(source, *array, out);
            if let Some(dim) = dim {
                collect_expr_refs(source, *dim, out);
            }
        }
        Expr::DotAccess { target, .. } => collect_expr_refs(source, *target, out),
        Expr::PropertyFetch { target, property, .. } => {
            collect_expr_refs(source, *target, out);
            collect_expr_refs(source, *property, out);
        }
        Expr::MethodCall { target, args, .. } => {
            collect_expr_refs(source, *target, out);
            for arg in args.iter() {
                collect_expr_refs(source, arg.value, out);
            }
        }
        Expr::Ternary {
            condition,
            if_true,
            if_false,
            ..
        } => {
            collect_expr_refs(source, *condition, out);
            if let Some(t) = if_true {
                collect_expr_refs(source, t, out);
            }
            collect_expr_refs(source, *if_false, out);
        }
        Expr::Match { condition, arms, .. } => {
            collect_expr_refs(source, *condition, out);
            for arm in arms.iter() {
                collect_expr_refs(source, arm.body, out);
            }
        }
        Expr::Bridge { args, .. } => {
            for arg in args.iter() {
                collect_expr_refs(source, arg.value, out);
            }
        }
        Expr::InterpolatedString { parts, .. } | Expr::ShellExec { parts, .. } => {
            for part in parts.iter() {
                collect_expr_refs(source, *part, out);
            }
        }
        Expr::Closure { params, uses, .. } => {
            for param in params.iter() {
                if let Some(default) = param.default {
                    collect_expr_refs(source, default, out);
                }
            }
            for u in uses.iter() {
                out.push((token_text(source, u.var), u.span));
            }
        }
        _ => {}
    }
}

fn extract_static_var_name(source: &[u8], expr: ExprId<'_>) -> Option<String> {
    match expr {
        Expr::Variable { name, .. } => Some(String::from_utf8_lossy(&source[name.start..name.end]).to_string()),
        Expr::Assign { var, .. } => extract_static_var_name(source, *var),
        _ => None,
    }
}

fn line_of(source: &[u8], span: Span) -> usize {
    span.line_info(source).map(|info| info.line).unwrap_or(0)
}

fn declare_top_level_bindings(
    source: &[u8],
    stmts: &[StmtId<'_>],
    out: &mut HashMap<String, usize>,
) {
    for stmt in stmts.iter() {
        match stmt {
            Stmt::Const { consts, span, .. } => {
                let line = line_of(source, *span);
                for item in consts.iter() {
                    let name = String::from_utf8_lossy(item.name.text(source)).to_string();
                    out.insert(name, line);
                }
            }
            Stmt::Static { vars, span, .. } => {
                let line = line_of(source, *span);
                for item in vars.iter() {
                    if let Some(name) = extract_static_var_name(source, item.var) {
                        out.insert(name, line);
                    }
                }
            }
            _ => {}
        }
    }
}

fn check_initializer_forward_refs(
    source: &[u8],
    binding_lines: &HashMap<String, usize>,
    name: &str,
    init: ExprId<'_>,
    errors: &mut Vec<ValidationError>,
) {
    let mut refs = Vec::new();
    collect_expr_refs(source, init, &mut refs);
    for (ref_name, ref_span) in refs {
        if ref_name == name {
            // Self-reference in initializer is always invalid.
            let (column, underline_len) = if let Some(info) = ref_span.line_info(source) {
                (info.column, ref_name.len())
            } else {
                (1, ref_name.len().max(1))
            };
            errors.push(ValidationError {
                kind: ErrorKind::SyntaxError,
                line: line_of(source, ref_span),
                column,
                message: format!("`{ref_name}` cannot be used in its own initializer"),
                help_text: "Remove the self-reference or initialize the binding to a literal value.".to_string(),
                suggestion: None,
                underline_length: underline_len,
                severity: Severity::Error,
            });
            continue;
        }
        if let Some(&decl_line) = binding_lines.get(&ref_name) {
            let use_line = line_of(source, ref_span);
            if use_line > 0 && decl_line > use_line {
                let (column, underline_len) = if let Some(info) = ref_span.line_info(source) {
                    (info.column, ref_name.len())
                } else {
                    (1, ref_name.len().max(1))
                };
                errors.push(ValidationError {
                    kind: ErrorKind::SyntaxError,
                    line: use_line,
                    column,
                    message: format!(
                        "`{ref_name}` is used here but is not initialized until line {decl_line}"
                    ),
                    help_text: format!(
                        "Move the declaration of `{ref_name}` above this initializer, or reorder the statements so the binding is initialized before it is used."
                    ),
                    suggestion: None,
                    underline_length: underline_len,
                    severity: Severity::Error,
                });
            }
        }
    }
}

fn check_top_level_statements(
    source: &[u8],
    binding_lines: &HashMap<String, usize>,
    stmts: &[StmtId<'_>],
    errors: &mut Vec<ValidationError>,
) {
    for stmt in stmts.iter() {
        match stmt {
            Stmt::Const { consts, .. } => {
                for item in consts.iter() {
                    let name = String::from_utf8_lossy(item.name.text(source)).to_string();
                    check_initializer_forward_refs(source, binding_lines, &name, item.value, errors);
                }
            }
            Stmt::Static { vars, .. } => {
                for item in vars.iter() {
                    if let Some(name) = extract_static_var_name(source, item.var) {
                        let init = item.default.unwrap_or(item.var);
                        check_initializer_forward_refs(source, binding_lines, &name, init, errors);
                    }
                }
            }
            Stmt::Expression { expr, .. } => {
                let mut refs = Vec::new();
                collect_expr_refs(source, *expr, &mut refs);
                for (ref_name, ref_span) in refs {
                    if let Some(&decl_line) = binding_lines.get(&ref_name) {
                        let use_line = line_of(source, ref_span);
                        if use_line > 0 && decl_line > use_line {
                            let (column, underline_len) = if let Some(info) = ref_span.line_info(source) {
                                (info.column, ref_name.len())
                            } else {
                                (1, ref_name.len().max(1))
                            };
                            errors.push(ValidationError {
                                kind: ErrorKind::SyntaxError,
                                line: use_line,
                                column,
                                message: format!(
                                    "`{ref_name}` is used here but is not initialized until line {decl_line}"
                                ),
                                help_text: format!(
                                    "Move the declaration of `{ref_name}` above this statement, or reorder the statements so the binding is initialized before it is used."
                                ),
                                suggestion: None,
                                underline_length: underline_len,
                                severity: Severity::Error,
                            });
                        }
                    }
                }
            }
            Stmt::Block { statements, .. }
            | Stmt::If { then_block: statements, else_block: None, .. }
            | Stmt::While { body: statements, .. }
            | Stmt::DoWhile { body: statements, .. }
            | Stmt::For { body: statements, .. }
            | Stmt::Foreach { body: statements, .. } => {
                check_top_level_statements(source, binding_lines, statements, errors);
            }
            Stmt::If {
                then_block,
                else_block: Some(else_block),
                ..
            } => {
                check_top_level_statements(source, binding_lines, then_block, errors);
                check_top_level_statements(source, binding_lines, else_block, errors);
            }
            Stmt::Switch { cases, .. } => {
                for case in cases.iter() {
                    if let Some(cond) = case.condition {
                        let mut refs = Vec::new();
                        collect_expr_refs(source, cond, &mut refs);
                        for (ref_name, ref_span) in refs {
                            if let Some(&decl_line) = binding_lines.get(&ref_name) {
                                let use_line = line_of(source, ref_span);
                                if use_line > 0 && decl_line > use_line {
                                    let (column, underline_len) = if let Some(info) = ref_span.line_info(source) {
                                        (info.column, ref_name.len())
                                    } else {
                                        (1, ref_name.len().max(1))
                                    };
                                    errors.push(ValidationError {
                                        kind: ErrorKind::SyntaxError,
                                        line: use_line,
                                        column,
                                        message: format!(
                                            "`{ref_name}` is used here but is not initialized until line {decl_line}"
                                        ),
                                        help_text: format!(
                                            "Move the declaration of `{ref_name}` above this switch condition, or reorder the statements so the binding is initialized before it is used."
                                        ),
                                        suggestion: None,
                                        underline_length: underline_len,
                                        severity: Severity::Error,
                                    });
                                }
                            }
                        }
                    }
                    check_top_level_statements(source, binding_lines, case.body, errors);
                }
            }
            Stmt::Function { body, .. }
            | Stmt::ReceiverMethod { body, .. }
            | Stmt::Try { body, .. }
            | Stmt::Declare { body, .. } => {
                // Function/receiver bodies are executed lazily; references to
                // top-level bindings declared later may be valid depending on
                // call timing. Skip them to avoid false positives.
                let _ = body;
            }
            _ => {}
        }
    }
}

/// Validate that DekaScript value bindings are not referenced before their
/// declaration at the top level.
pub fn validate_hoisting<'a>(
    program: &'a Program<'a>,
    source: &str,
    _file_path: &str,
) -> (Vec<ValidationError>, Vec<ValidationWarning>) {
    let source_bytes = source.as_bytes();
    let mut binding_lines = HashMap::new();
    declare_top_level_bindings(source_bytes, program.statements, &mut binding_lines);

    let mut errors = Vec::new();
    check_top_level_statements(source_bytes, &binding_lines, program.statements, &mut errors);
    (errors, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bumpalo::Bump;
    use php_rs::parser::lexer::Lexer;
    use php_rs::parser::parser::{Parser, ParserMode};

    fn deka_errors(source: &str) -> Vec<ValidationError> {
        let arena = Bump::new();
        let mut parser = Parser::new_with_mode(Lexer::new(source.as_bytes()), &arena, ParserMode::Ds);
        let program = parser.parse_program();
        assert!(program.errors.is_empty(), "parse errors: {:?}", program.errors);
        let (errors, _) = validate_hoisting(&program, source, "test.ds");
        errors
    }

    #[test]
    fn const_referencing_future_let_is_flagged() {
        let errors = deka_errors("const a = b + 1\nlet b = 1\n");
        assert!(
            errors.iter().any(|e| e.message.contains("b") && e.message.contains("not initialized")),
            "expected forward-reference error for b, got: {:?}",
            errors
        );
    }

    #[test]
    fn const_referencing_preceding_let_is_allowed() {
        let errors = deka_errors("let a = 1\nconst b = a + 2\n");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn const_component_used_before_definition_is_flagged() {
        let errors = deka_errors("const app = <Card />\nconst Card = fn(): Component { return <div /> }\n");
        assert!(
            errors.iter().any(|e| e.message.contains("Card") && e.message.contains("not initialized")),
            "expected forward-reference error for Card, got: {:?}",
            errors
        );
    }

    #[test]
    fn fn_component_used_before_definition_is_allowed() {
        let errors = deka_errors("const app = <Card />\nfn Card(): Component { return <div /> }\n");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn self_reference_in_const_initializer_is_flagged() {
        let errors = deka_errors("const x = x + 1\n");
        assert!(
            errors.iter().any(|e| e.message.contains("cannot be used in its own initializer")),
            "expected self-reference error, got: {:?}",
            errors
        );
    }

    #[test]
    fn block_scoped_let_does_not_leak() {
        let errors = deka_errors("const a = b + 1\nif (true) { let b = 1 }\n");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }
}