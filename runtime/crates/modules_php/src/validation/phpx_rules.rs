use php_rs::parser::ast::visitor::{Visitor, walk_expr, walk_stmt};
use php_rs::parser::ast::{BinaryOp, Expr, ExprId, Program, Stmt, StmtId};
use php_rs::parser::span::Span;

use super::{ErrorKind, Severity, ValidationError};

pub fn validate_no_null(program: &Program, source: &str) -> Vec<ValidationError> {
    let strict = std::env::var("PHPX_STRICT_NULL")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            value == "1" || value == "true" || value == "yes" || value == "on"
        })
        .unwrap_or(false);
    if !strict {
        return Vec::new();
    }
    let mut validator = NoNullValidator {
        source,
        errors: Vec::new(),
    };
    validator.visit_program(program);
    validator.errors
}

pub fn validate_no_exceptions(program: &Program, source: &str) -> Vec<ValidationError> {
    let mut validator = NoExceptionValidator {
        source,
        errors: Vec::new(),
    };
    validator.visit_program(program);
    validator.errors
}

pub fn validate_no_oop(program: &Program, source: &str, is_ds: bool) -> Vec<ValidationError> {
    let mut validator = NoOopValidator {
        source,
        errors: Vec::new(),
        is_ds,
    };
    validator.visit_program(program);
    validator.errors
}

pub fn validate_no_namespace(program: &Program, source: &str) -> Vec<ValidationError> {
    let mut validator = NoNamespaceValidator {
        source,
        errors: Vec::new(),
    };
    validator.visit_program(program);
    validator.errors
}

struct NoNullValidator<'a> {
    source: &'a str,
    errors: Vec<ValidationError>,
}

impl<'ast> Visitor<'ast> for NoNullValidator<'_> {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        match expr {
            Expr::Null { span } => {
                self.push_error(
                    ErrorKind::NullNotAllowed,
                    *span,
                    "Null literals are not allowed in PHPX.".to_string(),
                    "Use Option<T> instead of null.",
                );
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                if matches!(op, BinaryOp::EqEqEq | BinaryOp::NotEqEq)
                    && (is_null_expr(left) || is_null_expr(right))
                {
                    self.push_error(
                        ErrorKind::NullNotAllowed,
                        *span,
                        "Null comparisons are not allowed in PHPX.".to_string(),
                        "Use Option<T> and pattern matching instead of comparing to null.",
                    );
                }
            }
            Expr::Call { func, span, .. } => {
                if is_is_null_call(func, self.source) {
                    self.push_error(
                        ErrorKind::NullNotAllowed,
                        *span,
                        "is_null() is not allowed in PHPX.".to_string(),
                        "Use Option<T> and pattern matching instead.",
                    );
                }
            }
            _ => {}
        }

        walk_expr(self, expr);
    }
}

impl NoNullValidator<'_> {
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

struct NoExceptionValidator<'a> {
    source: &'a str,
    errors: Vec<ValidationError>,
}

impl<'ast> Visitor<'ast> for NoExceptionValidator<'_> {
    fn visit_stmt(&mut self, stmt: StmtId<'ast>) {
        match stmt {
            Stmt::Throw { span, .. } => {
                self.push_error(
                    ErrorKind::ExceptionNotAllowed,
                    *span,
                    "throw is not allowed in PHPX.".to_string(),
                    "Use Result<T, E> instead of throwing exceptions.",
                );
            }
            Stmt::Try { span, .. } => {
                self.push_error(
                    ErrorKind::ExceptionNotAllowed,
                    *span,
                    "try/catch is not allowed in PHPX.".to_string(),
                    "Use Result<T, E> instead of exceptions.",
                );
            }
            _ => {}
        }

        walk_stmt(self, stmt);
    }
}

impl NoExceptionValidator<'_> {
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

struct NoOopValidator<'a> {
    source: &'a str,
    errors: Vec<ValidationError>,
    // DekaScript traits (RFD 19) reuse the Stmt::Trait AST node for a
    // different feature than PHP's horizontal-reuse trait; only the latter
    // is rejected here.
    is_ds: bool,
}

impl<'ast> Visitor<'ast> for NoOopValidator<'_> {
    fn visit_stmt(&mut self, stmt: StmtId<'ast>) {
        match stmt {
            Stmt::Class {
                kind,
                extends,
                implements,
                span,
                ..
            } => {
                if let php_rs::parser::ast::ClassKind::Class = kind {
                    self.push_error(
                        ErrorKind::OopNotAllowed,
                        *span,
                        "Classes are not allowed in PHPX.".to_string(),
                        "Use structs instead of classes.",
                    );
                }
                if extends.is_some() {
                    self.push_error(
                        ErrorKind::OopNotAllowed,
                        *span,
                        "Inheritance is not allowed in PHPX.".to_string(),
                        "Use struct composition or interfaces instead.",
                    );
                }
                if !implements.is_empty() {
                    self.push_error(
                        ErrorKind::OopNotAllowed,
                        *span,
                        "implements is not allowed in PHPX.".to_string(),
                        "Use structural interfaces instead of implements.",
                    );
                }
            }
            Stmt::Trait { span, .. } => {
                if !self.is_ds {
                    self.push_error(
                        ErrorKind::OopNotAllowed,
                        *span,
                        "Traits are not allowed in PHPX.".to_string(),
                        "Use struct composition instead of traits.",
                    );
                }
            }
            Stmt::Interface { extends, span, .. } => {
                if !extends.is_empty() {
                    self.push_error(
                        ErrorKind::OopNotAllowed,
                        *span,
                        "Interface inheritance is not allowed in PHPX.".to_string(),
                        "Use structural interfaces without extends.",
                    );
                }
            }
            _ => {}
        }

        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        if let Expr::New { span, .. } = expr {
            self.push_error(
                ErrorKind::OopNotAllowed,
                *span,
                "new is not allowed in PHPX.".to_string(),
                "Use struct literals instead of new.",
            );
        }
        walk_expr(self, expr);
    }
}

impl NoOopValidator<'_> {
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

struct NoNamespaceValidator<'a> {
    source: &'a str,
    errors: Vec<ValidationError>,
}

impl<'ast> Visitor<'ast> for NoNamespaceValidator<'_> {
    fn visit_stmt(&mut self, stmt: StmtId<'ast>) {
        match stmt {
            Stmt::Namespace { span, .. } => {
                self.push_error(
                    ErrorKind::NamespaceNotAllowed,
                    *span,
                    "Namespaces are not allowed in PHPX.".to_string(),
                    "Use import/export modules instead of namespaces.",
                );
            }
            Stmt::Use { span, .. } => {
                self.push_error(
                    ErrorKind::NamespaceNotAllowed,
                    *span,
                    "use statements are not allowed in PHPX.".to_string(),
                    "Use import/export modules instead of namespaces.",
                );
            }
            _ => {}
        }

        walk_stmt(self, stmt);
    }
}

impl NoNamespaceValidator<'_> {
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

fn is_null_expr(expr: ExprId<'_>) -> bool {
    matches!(expr, Expr::Null { .. })
}

fn is_is_null_call(expr: ExprId<'_>, source: &str) -> bool {
    let Expr::Variable { name, .. } = expr else {
        return false;
    };
    let raw = name.as_str(source.as_bytes());
    let Ok(mut text) = std::str::from_utf8(raw) else {
        return false;
    };
    if let Some(stripped) = text.strip_prefix('\\') {
        text = stripped;
    }
    if let Some(stripped) = text.strip_prefix('$') {
        text = stripped;
    }
    text == "is_null"
}
