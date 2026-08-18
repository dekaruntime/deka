use std::collections::{HashMap, HashSet};

use php_rs::parser::ast::visitor::{Visitor, walk_expr};
use php_rs::parser::ast::{
    BinaryOp, ClassKind, ClassMember, Expr, ExprId, Program, PropertyEntry, Stmt, UnaryOp,
};
use php_rs::parser::span::Span;

use super::{ErrorKind, Severity, ValidationError};

pub fn validate_struct_definitions(program: &Program, source: &str) -> Vec<ValidationError> {
    let struct_defs = collect_struct_definitions(program, source);
    let mut errors = Vec::new();

    for def in struct_defs.values() {
        for error in &def.errors {
            errors.push(error.clone());
        }
    }

    errors
}

pub fn validate_struct_literals(program: &Program, source: &str) -> Vec<ValidationError> {
    let struct_defs = collect_struct_definitions(program, source);
    let mut validator = StructLiteralValidator {
        source,
        struct_defs,
        errors: Vec::new(),
    };
    validator.visit_program(program);
    validator.errors
}

#[derive(Debug, Clone)]
struct StructFieldInfo {
    has_default: bool,
    span: Span,
}

#[derive(Debug, Clone)]
struct StructDef {
    fields: HashMap<String, StructFieldInfo>,
    errors: Vec<ValidationError>,
}

fn collect_struct_definitions(program: &Program, source: &str) -> HashMap<String, StructDef> {
    let mut defs = HashMap::new();
    let mut struct_names = HashSet::new();

    for stmt in program.statements {
        if let Stmt::Class { kind, name, .. } = stmt {
            if matches!(kind, ClassKind::Struct) {
                if let Some(name_str) = token_text(name, source) {
                    struct_names.insert(name_str);
                }
            }
        }
    }

    for stmt in program.statements {
        let Stmt::Class {
            kind,
            name,
            members,
            ..
        } = stmt
        else {
            continue;
        };
        if !matches!(kind, ClassKind::Struct) {
            continue;
        }

        let struct_name = token_text(name, source).unwrap_or_else(|| "struct".to_string());
        let mut fields: HashMap<String, StructFieldInfo> = HashMap::new();
        let mut errors = Vec::new();

        for member in *members {
            match member {
                ClassMember::Method { name, span, .. } => {
                    if token_text(name, source).as_deref() == Some("__construct") {
                        errors.push(struct_error(
                            *span,
                            source,
                            "Struct constructors are not allowed in PHPX.".to_string(),
                            "Use struct literals instead of __construct.",
                        ));
                    }
                }
                ClassMember::Property {
                    ty, entries, span, ..
                } => {
                    if ty.is_none() {
                        errors.push(struct_error(
                            *span,
                            source,
                            "Struct fields require explicit types.".to_string(),
                            "Add a type annotation like `$field: Type`.",
                        ));
                    }
                    for entry in *entries {
                        handle_struct_field(entry, &struct_name, &mut fields, source, &mut errors);
                    }
                }
                ClassMember::PropertyHook {
                    ty,
                    name,
                    default,
                    span,
                    ..
                } => {
                    if ty.is_none() {
                        errors.push(struct_error(
                            *span,
                            source,
                            "Struct fields require explicit types.".to_string(),
                            "Add a type annotation like `$field: Type`.",
                        ));
                    }
                    let entry = PropertyEntry {
                        name,
                        default: *default,
                        annotations: &[],
                        optional: false,
                        is_mut: false,
                        span: *span,
                    };
                    handle_struct_field(&entry, &struct_name, &mut fields, source, &mut errors);
                }
                ClassMember::Embed { types, span, .. } => {
                    for ty in *types {
                        if let Some(name_str) = name_to_string(ty, source) {
                            if !struct_names.contains(&name_str) {
                                errors.push(struct_error(
                                    *span,
                                    source,
                                    format!(
                                        "Struct composition requires a struct type, but '{}' was not found.",
                                        name_str
                                    ),
                                    "Ensure the composed type is a struct declared in this module.",
                                ));
                            }
                            fields.insert(
                                name_str.clone(),
                                StructFieldInfo {
                                    has_default: false,
                                    span: *span,
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        defs.insert(struct_name, StructDef { fields, errors });
    }

    defs
}

fn handle_struct_field(
    entry: &PropertyEntry,
    struct_name: &str,
    fields: &mut HashMap<String, StructFieldInfo>,
    source: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(name_str) = token_text(entry.name, source) else {
        return;
    };
    if fields.contains_key(&name_str) {
        errors.push(struct_error(
            entry.span,
            source,
            format!(
                "Duplicate field '{}' in struct '{}'.",
                name_str, struct_name
            ),
            "Remove or rename the duplicate field.",
        ));
        return;
    }

    if let Some(default_expr) = entry.default {
        if !is_constant_expr(default_expr) {
            errors.push(struct_error(
                entry.span,
                source,
                format!(
                    "Default value for '{}' in struct '{}' must be a constant expression.",
                    name_str, struct_name
                ),
                "Use a constant literal or object/struct literal for defaults.",
            ));
        }
    }

    fields.insert(
        name_str.clone(),
        StructFieldInfo {
            has_default: entry.default.is_some(),
            span: entry.span,
        },
    );
}

struct StructLiteralValidator<'a> {
    source: &'a str,
    struct_defs: HashMap<String, StructDef>,
    errors: Vec<ValidationError>,
}

impl<'ast> Visitor<'ast> for StructLiteralValidator<'_> {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        if let Expr::StructLiteral { name, fields, span } = expr {
            self.validate_struct_literal(name, fields, *span);
        }
        walk_expr(self, expr);
    }
}

impl StructLiteralValidator<'_> {
    fn validate_struct_literal(
        &mut self,
        name: &php_rs::parser::ast::Name,
        fields: &[php_rs::parser::ast::StructLiteralField],
        span: Span,
    ) {
        let Some(struct_name) = name_to_string(name, self.source) else {
            return;
        };
        let Some(def) = self.struct_defs.get(&struct_name) else {
            self.errors.push(struct_error(
                span,
                self.source,
                format!("Unknown struct '{}'.", struct_name),
                "Define the struct before using a literal.",
            ));
            return;
        };

        let mut seen = HashSet::new();
        for field in fields {
            let Some(field_name) = token_text(field.name, self.source) else {
                continue;
            };
            if !seen.insert(field_name.clone()) {
                self.errors.push(struct_error(
                    field.span,
                    self.source,
                    format!(
                        "Duplicate field '{}' in struct literal '{}'.",
                        field_name, struct_name
                    ),
                    "Remove the duplicate field.",
                ));
                continue;
            }
            if !def.fields.contains_key(&field_name) {
                self.errors.push(struct_error(
                    field.span,
                    self.source,
                    format!(
                        "Unknown field '{}' in struct literal '{}'.",
                        field_name, struct_name
                    ),
                    "Remove the extra field or update the struct definition.",
                ));
            }
        }

        for (name, info) in &def.fields {
            if info.has_default {
                continue;
            }
            if !seen.contains(name) {
                self.errors.push(struct_error(
                    info.span,
                    self.source,
                    format!(
                        "Missing required field '{}' for struct '{}'.",
                        name, struct_name
                    ),
                    "Provide a value for the missing field.",
                ));
            }
        }
    }
}

fn is_constant_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Integer { .. }
        | Expr::Float { .. }
        | Expr::String { .. }
        | Expr::Boolean { .. }
        | Expr::Null { .. } => true,
        Expr::Array { items, .. } => items.iter().all(|item| {
            if item.unpack {
                return false;
            }
            let key_ok = item.key.map(is_constant_expr).unwrap_or(true);
            key_ok && is_constant_expr(item.value)
        }),
        Expr::ObjectLiteral { items, .. } => items.iter().all(|item| is_constant_expr(item.value)),
        Expr::StructLiteral { fields, .. } => {
            fields.iter().all(|field| is_constant_expr(field.value))
        }
        Expr::ClassConstFetch { .. } => true,
        Expr::Binary {
            op, left, right, ..
        } => matches!(op, BinaryOp::BitOr) && is_constant_expr(left) && is_constant_expr(right),
        Expr::Unary { op, expr, .. } => {
            matches!(op, UnaryOp::Plus | UnaryOp::Minus) && is_constant_expr(expr)
        }
        _ => false,
    }
}

fn struct_error(span: Span, source: &str, message: String, help_text: &str) -> ValidationError {
    let (line, column, underline_length) = span_location(span, source);
    ValidationError {
        kind: ErrorKind::StructError,
        line,
        column,
        message,
        help_text: help_text.to_string(),
        suggestion: None,
        underline_length,
        severity: Severity::Error,
    }
}

fn span_location(span: Span, source: &str) -> (usize, usize, usize) {
    if let Some(info) = span.line_info(source.as_bytes()) {
        let padding = std::cmp::min(info.line_text.len(), info.column.saturating_sub(1));
        let highlight_len = std::cmp::max(
            1,
            std::cmp::min(span.len(), info.line_text.len().saturating_sub(padding)),
        );
        (info.line, info.column, highlight_len)
    } else {
        (1, 1, 1)
    }
}

fn token_text(token: &php_rs::parser::lexer::token::Token, source: &str) -> Option<String> {
    std::str::from_utf8(token.text(source.as_bytes()))
        .ok()
        .map(|text| text.to_string())
}

fn name_to_string(name: &php_rs::parser::ast::Name, source: &str) -> Option<String> {
    std::str::from_utf8(name.span.as_str(source.as_bytes()))
        .ok()
        .map(|text| text.to_string())
}
