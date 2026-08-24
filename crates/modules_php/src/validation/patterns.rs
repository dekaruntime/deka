use std::collections::{HashMap, HashSet};

use php_rs::parser::ast::visitor::{Visitor, walk_expr};
use php_rs::parser::ast::{ClassMember, Expr, ExprId, MatchArm, Param, Program, Stmt};
use php_rs::parser::span::Span;

use super::{ErrorKind, Severity, ValidationError};

#[derive(Debug, Clone)]
struct EnumCaseInfo {
    params: Vec<String>,
}

#[derive(Debug, Clone)]
struct EnumInfo {
    cases: HashMap<String, EnumCaseInfo>,
}

/// Pattern-shape checks for `match` (duplicate arms, payload arity).
/// Exhaustiveness lives in typeck (`check_match_exhaustive`, deka#281).
pub fn validate_match_exhaustiveness(program: &Program, source: &str) -> Vec<ValidationError> {
    let enums = collect_enums(program, source);
    let mut validator = MatchValidator {
        source,
        enums,
        errors: Vec::new(),
    };
    validator.visit_program(program);
    validator.errors
}

struct MatchValidator<'a> {
    source: &'a str,
    enums: HashMap<String, EnumInfo>,
    errors: Vec<ValidationError>,
}

impl<'ast> Visitor<'ast> for MatchValidator<'_> {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        if let Expr::Match { arms, .. } = expr {
            self.validate_match(arms);
        }
        walk_expr(self, expr);
    }
}

impl MatchValidator<'_> {
    fn validate_match(&mut self, arms: &[MatchArm]) {
        let mut seen_cases: HashMap<String, HashSet<String>> = HashMap::new();

        for arm in arms {
            let Some(conds) = arm.conditions else {
                continue;
            };
            for cond in conds {
                let Some((enum_name, case_name)) = enum_case_from_expr(*cond, self.source) else {
                    continue;
                };
                let entry = seen_cases.entry(enum_name.clone()).or_default();
                if entry.contains(&case_name) {
                    self.errors.push(pattern_error(
                        cond.span(),
                        self.source,
                        format!("Unreachable match arm for {}::{}.", enum_name, case_name),
                        "Remove the duplicate enum case.",
                    ));
                } else {
                    entry.insert(case_name.clone());
                }
                if let Expr::StaticCall { args, .. } | Expr::Call { args, .. } = *cond {
                    self.validate_payload_binding(&enum_name, &case_name, args, cond.span());
                }
            }
        }
    }

    fn validate_payload_binding(
        &mut self,
        enum_name: &str,
        case_name: &str,
        args: &[php_rs::parser::ast::Arg],
        span: Span,
    ) {
        let Some(info) = self
            .enums
            .get(enum_name)
            .and_then(|info| info.cases.get(case_name))
        else {
            return;
        };
        let expected = info.params.len();
        let got = args.len();
        if expected != got {
            self.errors.push(pattern_error(
                span,
                self.source,
                format!(
                    "Enum case {}::{} expects {} bindings, got {}.",
                    enum_name, case_name, expected, got
                ),
                "Match the enum case payload arity.",
            ));
            return;
        }
        for arg in args {
            if !is_variable_binding(arg.value, self.source) {
                self.errors.push(pattern_error(
                    arg.span,
                    self.source,
                    format!(
                        "Enum case {}::{} payload bindings must be variables.",
                        enum_name, case_name
                    ),
                    "Use variable bindings like $value.",
                ));
                break;
            }
        }
    }
}

fn collect_enums(program: &Program, source: &str) -> HashMap<String, EnumInfo> {
    let mut enums = HashMap::new();
    for stmt in program.statements {
        let Stmt::Enum { name, members, .. } = stmt else {
            continue;
        };
        let Some(enum_name) = token_text(name, source) else {
            continue;
        };
        let mut cases = HashMap::new();
        for member in *members {
            if let ClassMember::Case { name, payload, .. } = member {
                if let Some(case_name) = token_text(name, source) {
                    let params = payload
                        .map(|params| {
                            params
                                .iter()
                                .filter_map(|param| param_name(param, source))
                                .collect()
                        })
                        .unwrap_or_default();
                    cases.insert(case_name, EnumCaseInfo { params });
                }
            }
        }
        enums.insert(enum_name, EnumInfo { cases });
    }

    // Built-in enums
    enums
        .entry("Option".to_string())
        .or_insert_with(|| EnumInfo {
            cases: HashMap::from([
                (
                    "Some".to_string(),
                    EnumCaseInfo {
                        params: vec!["value".to_string()],
                    },
                ),
                ("None".to_string(), EnumCaseInfo { params: Vec::new() }),
            ]),
        });
    enums
        .entry("Result".to_string())
        .or_insert_with(|| EnumInfo {
            cases: HashMap::from([
                (
                    "Ok".to_string(),
                    EnumCaseInfo {
                        params: vec!["value".to_string()],
                    },
                ),
                (
                    "Err".to_string(),
                    EnumCaseInfo {
                        params: vec!["error".to_string()],
                    },
                ),
            ]),
        });

    enums
}

fn enum_case_from_expr(expr: ExprId<'_>, source: &str) -> Option<(String, String)> {
    match *expr {
        Expr::Variable { name, .. } => {
            let raw = std::str::from_utf8(name.as_str(source.as_bytes())).ok()?;
            let name = raw.trim().trim_start_matches('$');
            if name.eq_ignore_ascii_case("Some") || name.eq_ignore_ascii_case("None") {
                Some(("Option".to_string(), name.to_string()))
            } else if name.eq_ignore_ascii_case("Ok") || name.eq_ignore_ascii_case("Err") {
                Some(("Result".to_string(), name.to_string()))
            } else {
                None
            }
        }
        Expr::Call { func, .. } => enum_case_from_expr(func, source),
        Expr::DotAccess { target, property, .. } => {
            let class_name = expr_name(target, source)?;
            let case_name = std::str::from_utf8(property.span.as_str(source.as_bytes()))
                .ok()?
                .trim()
                .to_string();
            Some((class_name, case_name))
        }
        Expr::ClassConstFetch {
            class, constant, ..
        } => {
            let class_name = expr_name(class, source)?;
            let case_name = expr_name(constant, source)?;
            Some((class_name, case_name))
        }
        Expr::StaticCall { class, method, .. } => {
            let class_name = expr_name(class, source)?;
            let case_name = expr_name(method, source)?;
            Some((class_name, case_name))
        }
        _ => None,
    }
}

fn expr_name(expr: ExprId<'_>, source: &str) -> Option<String> {
    match *expr {
        Expr::Variable { name, .. } => {
            let raw = std::str::from_utf8(name.as_str(source.as_bytes())).ok()?;
            let trimmed = raw.trim();
            let trimmed = trimmed.trim_start_matches('\\');
            let trimmed = trimmed.trim_start_matches('$');
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

fn is_variable_binding(expr: ExprId<'_>, source: &str) -> bool {
    match *expr {
        Expr::Variable { name, .. } => {
            if let Ok(raw) = std::str::from_utf8(name.as_str(source.as_bytes())) {
                let trimmed = raw.trim_start();
                // DekaScript allows bare identifier bindings (e.g. `Some(v)`);
                // PHPX uses `$v`. Accept either form as long as it names a variable.
                let body = trimmed.strip_prefix('$').unwrap_or(trimmed);
                !body.is_empty()
                    && body
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_alphabetic() || c == '_')
                        .unwrap_or(false)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn param_name(param: &Param<'_>, source: &str) -> Option<String> {
    let raw = std::str::from_utf8(param.name.text(source.as_bytes())).ok()?;
    Some(raw.trim_start_matches('$').to_string())
}

fn token_text(token: &php_rs::parser::lexer::token::Token, source: &str) -> Option<String> {
    std::str::from_utf8(token.text(source.as_bytes()))
        .ok()
        .map(|text| text.to_string())
}

fn pattern_error(span: Span, source: &str, message: String, help_text: &str) -> ValidationError {
    let (line, column, underline_length) = span_location(span, source);
    ValidationError {
        kind: ErrorKind::PatternError,
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
