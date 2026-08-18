use php_rs::parser::ast::visitor::{Visitor, walk_type};
use php_rs::parser::ast::{Name, Program, Type};
use php_rs::parser::lexer::token::TokenKind;
use php_rs::parser::span::Span;

use super::{ErrorKind, Severity, ValidationError};

pub fn validate_type_annotations(program: &Program, source: &str) -> Vec<ValidationError> {
    let mut validator = TypeSyntaxValidator {
        source,
        errors: Vec::new(),
    };
    validator.visit_program(program);
    validator.errors
}

struct TypeSyntaxValidator<'a> {
    source: &'a str,
    errors: Vec<ValidationError>,
}

impl<'ast> Visitor<'ast> for TypeSyntaxValidator<'_> {
    fn visit_type(&mut self, ty: &'ast Type<'ast>) {
        self.check_type(ty);
        walk_type(self, ty);
    }
}

impl TypeSyntaxValidator<'_> {
    fn check_type(&mut self, ty: &Type) {
        match ty {
            Type::Nullable(inner) => {
                self.push_error(
                    ErrorKind::NullNotAllowed,
                    type_span(inner),
                    "Nullable types are not allowed in PHPX.".to_string(),
                    "Use Option<T> instead of ?T or T|null.",
                );
            }
            Type::Option(inner) => {
                // DekaScript `T?` is sugar for `Option<T>`; recurse into the inner type.
                self.check_type(inner);
            }
            Type::Union(types) => {
                if types.iter().any(is_null_type) {
                    self.push_error(
                        ErrorKind::NullNotAllowed,
                        type_span(ty),
                        "Nullable union types are not allowed in PHPX.".to_string(),
                        "Use Option<T> instead of T|null.",
                    );
                } else if !is_supported_union(types, self.source) {
                    self.push_error(
                        ErrorKind::TypeError,
                        type_span(ty),
                        "Only int|float unions are supported in PHPX.".to_string(),
                        "Use int|float or refactor to a struct/enum.",
                    );
                }
            }
            Type::Applied { base, args } => {
                let Some(base_name) = type_base_name(base, self.source) else {
                    self.push_error(
                        ErrorKind::TypeError,
                        type_span(base),
                        "Invalid generic base type.".to_string(),
                        "Use Option<T>, Result<T, E>, or array<T>.",
                    );
                    return;
                };
                let base_last = base_name.rsplit('\\').next().unwrap_or(&base_name);
                let expected = match base_last {
                    "Option" => Some(1),
                    "Result" => Some(2),
                    "Promise" => Some(1),
                    "array" | "Array" => Some(1),
                    _ => None,
                };

                if let Some(expected_count) = expected {
                    if args.len() != expected_count {
                        self.push_error(
                            ErrorKind::TypeError,
                            type_span(base),
                            format!(
                                "Generic '{}' expects {} type argument(s).",
                                base_last, expected_count
                            ),
                            "Update the type arguments to match the expected arity.",
                        );
                    }
                } else {
                    self.push_error(
                        ErrorKind::TypeError,
                        type_span(base),
                        format!("Unsupported generic base type '{}'.", base_last),
                        "Use Option<T>, Result<T, E>, Promise<T>, or array<T>.",
                    );
                }
            }
            Type::Simple(token) => {
                if token.kind == TokenKind::TypeNull {
                    self.push_error(
                        ErrorKind::NullNotAllowed,
                        token.span,
                        "Null is not allowed in PHPX type annotations.".to_string(),
                        "Use Option<T> instead of null.",
                    );
                }
            }
            _ => {}
        }
    }

    fn push_error(&mut self, kind: ErrorKind, span: Span, message: String, help_text: &str) {
        let (line, column, underline_length) = span_location(span, self.source);
        self.errors.push(ValidationError {
            kind,
            line,
            column,
            message,
            help_text: help_text.to_string(),
            suggestion: None,
            underline_length,
            severity: Severity::Error,
        });
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

fn type_span(ty: &Type) -> Span {
    match ty {
        Type::Simple(token) => token.span,
        Type::Name(name) => name.span,
        Type::Union(types) | Type::Intersection(types) => {
            types.first().map(type_span).unwrap_or_default()
        }
        Type::Nullable(inner) => type_span(inner),
        Type::Option(inner) => type_span(inner),
        Type::ObjectShape(fields) => fields.first().map(|field| field.span).unwrap_or_default(),
        Type::Applied { base, .. } => type_span(base),
    }
}

fn type_base_name(ty: &Type, source: &str) -> Option<String> {
    match ty {
        Type::Simple(token) => std::str::from_utf8(token.text(source.as_bytes()))
            .ok()
            .map(|text| text.to_string()),
        Type::Name(name) => name_to_string(name, source),
        _ => None,
    }
}

fn name_to_string(name: &Name, source: &str) -> Option<String> {
    std::str::from_utf8(name.span.as_str(source.as_bytes()))
        .ok()
        .map(|text| text.to_string())
}

fn is_null_type(ty: &Type) -> bool {
    match ty {
        Type::Nullable(_) => true,
        Type::Simple(token) => token.kind == TokenKind::TypeNull,
        _ => false,
    }
}

fn is_supported_union(types: &[Type], source: &str) -> bool {
    if types.len() != 2 {
        return false;
    }
    let left = type_base_name(&types[0], source).unwrap_or_default();
    let right = type_base_name(&types[1], source).unwrap_or_default();
    let left = left.as_str();
    let right = right.as_str();
    (left.eq_ignore_ascii_case("int") && right.eq_ignore_ascii_case("float"))
        || (left.eq_ignore_ascii_case("float") && right.eq_ignore_ascii_case("int"))
}
