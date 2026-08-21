use std::collections::HashSet;

use php_rs::parser::ast::visitor::{Visitor, walk_stmt, walk_type};
use php_rs::parser::ast::{Program, Stmt, Type};
use php_rs::parser::span::Span;

use super::{ErrorKind, Severity, ValidationError, ValidationWarning};

pub fn validate_generics(
    program: &Program,
    source: &str,
) -> (Vec<ValidationError>, Vec<ValidationWarning>) {
    let mut validator = GenericValidator {
        source,
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    validator.visit_program(program);
    (validator.errors, validator.warnings)
}

struct GenericValidator<'a> {
    source: &'a str,
    errors: Vec<ValidationError>,
    warnings: Vec<ValidationWarning>,
}

impl<'ast> Visitor<'ast> for GenericValidator<'_> {
    fn visit_stmt(&mut self, stmt: php_rs::parser::ast::StmtId<'ast>) {
        match stmt {
            Stmt::Function {
                type_params,
                params,
                return_type,
                span,
                ..
            } => {
                self.check_generic_usage(type_params, params, *return_type, *span);
            }
            Stmt::TypeAlias {
                type_params,
                ty,
                span,
                ..
            } => {
                self.check_alias_generic_usage(type_params, ty, *span);
            }
            _ => {}
        }

        walk_stmt(self, stmt);
    }
}

impl GenericValidator<'_> {
    fn check_generic_usage(
        &mut self,
        type_params: &[php_rs::parser::ast::TypeParam],
        params: &[php_rs::parser::ast::Param],
        return_type: Option<&Type>,
        span: Span,
    ) {
        if type_params.is_empty() {
            return;
        }
        let param_names: HashSet<String> = type_params
            .iter()
            .filter_map(|param| token_text(param.name, self.source))
            .collect();
        if param_names.is_empty() {
            return;
        }

        let mut used = HashSet::new();
        for param in params {
            if let Some(ty) = param.ty {
                collect_type_param_names(ty, &param_names, &mut used, self.source);
            }
        }
        if let Some(ty) = return_type {
            collect_type_param_names(ty, &param_names, &mut used, self.source);
        }

        for name in param_names {
            if !used.contains(&name) {
                let (line, column, _) = span_location(span, self.source);
                self.warnings.push(ValidationWarning {
                    kind: ErrorKind::TypeError,
                    line,
                    column,
                    message: format!("Generic parameter '{}' is never used.", name),
                    help_text: "Remove the unused generic parameter or use it in the signature."
                        .to_string(),
                    suggestion: None,
                    underline_length: 1,
                    severity: Severity::Warning,
                });
            }
        }
    }

    fn check_alias_generic_usage(
        &mut self,
        type_params: &[php_rs::parser::ast::TypeParam],
        ty: &Type,
        span: Span,
    ) {
        if type_params.is_empty() {
            return;
        }
        let param_names: HashSet<String> = type_params
            .iter()
            .filter_map(|param| token_text(param.name, self.source))
            .collect();
        if param_names.is_empty() {
            return;
        }
        let mut used = HashSet::new();
        collect_type_param_names(ty, &param_names, &mut used, self.source);
        for name in param_names {
            if !used.contains(&name) {
                let (line, column, _) = span_location(span, self.source);
                self.warnings.push(ValidationWarning {
                    kind: ErrorKind::TypeError,
                    line,
                    column,
                    message: format!("Generic parameter '{}' is never used.", name),
                    help_text: "Remove the unused generic parameter or use it in the alias."
                        .to_string(),
                    suggestion: None,
                    underline_length: 1,
                    severity: Severity::Warning,
                });
            }
        }
    }
}

fn collect_type_param_names(
    ty: &Type,
    params: &HashSet<String>,
    used: &mut HashSet<String>,
    source: &str,
) {
    match ty {
        Type::Simple(token) => {
            if let Some(name) = token_text(token, source) {
                if params.contains(&name) {
                    used.insert(name);
                }
            }
        }
        Type::Name(name) => {
            if let Some(name_str) = name_to_string(name, source) {
                if params.contains(&name_str) {
                    used.insert(name_str);
                }
            }
        }
        _ => {}
    }

    walk_type(
        &mut TypeParamWalker {
            params,
            used,
            source,
        },
        ty,
    );
}

struct TypeParamWalker<'a> {
    params: &'a HashSet<String>,
    used: &'a mut HashSet<String>,
    source: &'a str,
}

impl<'ast> Visitor<'ast> for TypeParamWalker<'_> {
    fn visit_type(&mut self, ty: &'ast Type<'ast>) {
        match ty {
            Type::Simple(token) => {
                if let Some(name) = token_text(token, self.source) {
                    if self.params.contains(&name) {
                        self.used.insert(name);
                    }
                }
            }
            Type::Name(name) => {
                if let Some(name_str) = name_to_string(name, self.source) {
                    if self.params.contains(&name_str) {
                        self.used.insert(name_str);
                    }
                }
            }
            _ => {}
        }
        walk_type(self, ty);
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
