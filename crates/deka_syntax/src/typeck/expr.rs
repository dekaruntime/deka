//! Expression typechecking.

use std::collections::{HashMap, HashSet};

use crate::ast;

use super::types::{is_assignable, Type};
use super::Checker;

impl<'a> Checker<'a> {
    pub(super) fn check_expr(&mut self, expr: &ast::Expr<'a>) -> Type<'a> {
        match expr {
            ast::Expr::Number { .. } => Type::Named { name: "number" },
            ast::Expr::String { .. } => Type::Named { name: "string" },
            ast::Expr::Boolean { value: true, .. } | ast::Expr::Boolean { value: false, .. } => {
                Type::Named { name: "boolean" }
            }
            ast::Expr::None { .. } => Type::None,
            ast::Expr::Identifier { name, span } => {
                self.lookup_var(name).unwrap_or_else(|| {
                    self.error_span(*span, format!("unknown identifier `{name}`"));
                    Type::Error
                })
            }
            ast::Expr::Binary {
                op,
                left,
                right,
                span,
            } => self.check_binary(*op, left, right, *span),
            ast::Expr::Unary {
                op,
                operand,
                span,
            } => self.check_unary(*op, operand, *span),
            ast::Expr::Call {
                callee,
                args,
                span,
                ..
            } => self.check_call(callee, args, *span),
            ast::Expr::FieldAccess { object, field, span } => {
                self.check_field_access(object, field, *span)
            }
            ast::Expr::StructLiteral {
                name,
                fields,
                span,
            } => self.check_struct_literal(name, fields, *span),
            ast::Expr::Paren { expr, .. } => self.check_expr(expr),
            ast::Expr::Match {
                scrutinee,
                arms,
                span,
            } => self.check_match(scrutinee, arms, *span),
            ast::Expr::EnumConstructor {
                enum_name,
                case_name,
                payload,
                span,
            } => self.check_enum_constructor(enum_name, case_name, payload.as_deref(), *span),
            _ => {
                self.error_at_expr(expr, "unsupported expression in v2 typeck");
                Type::Error
            }
        }
    }

    fn check_struct_literal(
        &mut self,
        name: &'a str,
        fields: &'a [ast::StructLiteralField<'a>],
        span: ast::Span,
    ) -> Type<'a> {
        let info = match self.structs.get(name).cloned() {
            Some(info) => info,
            None => {
                self.error_span(span, format!("unknown struct `{name}`"));
                return Type::Error;
            }
        };

        let mut seen_fields = HashSet::new();
        for field in fields {
            if !seen_fields.insert(field.name) {
                self.error_span(
                    field.span,
                    format!("duplicate field `{}` in struct literal", field.name),
                );
            }
            let expected_type = match info.fields.iter().find(|f| f.name == field.name) {
                Some(f) => self.resolve_ast_type(&f.ty),
                None => {
                    self.error_span(
                        field.span,
                        format!("struct `{name}` has no field `{}`", field.name),
                    );
                    Type::Error
                }
            };
            let value_type = self.check_expr(&field.value);
            if !is_assignable(&expected_type, &value_type) {
                self.error_span(
                    field.span,
                    format!(
                        "field `{}` expected type `{expected_type}`, found type `{value_type}`",
                        field.name
                    ),
                );
            }
        }

        for field in info.fields {
            if field.default_value.is_none() && !seen_fields.contains(field.name) {
                self.error_span(
                    span,
                    format!(
                        "missing required field `{}` in struct literal for `{name}`",
                        field.name
                    ),
                );
            }
        }

        Type::Struct { name }
    }

    fn check_field_access(
        &mut self,
        object: &ast::Expr<'a>,
        field: &'a str,
        span: ast::Span,
    ) -> Type<'a> {
        let object_type = self.check_expr(object);
        if object_type.is_error() {
            return Type::Error;
        }

        let struct_name = match &object_type {
            Type::Struct { name } => *name,
            _ => {
                self.error_span(
                    span,
                    format!("cannot access field `{field}` on type `{object_type}`"),
                );
                return Type::Error;
            }
        };

        let info = match self.structs.get(struct_name).cloned() {
            Some(info) => info,
            None => {
                self.error_span(span, format!("unknown struct `{struct_name}`"));
                return Type::Error;
            }
        };

        match info.fields.iter().find(|f| f.name == field) {
            Some(f) => self.resolve_ast_type(&f.ty),
            None => {
                self.error_span(
                    span,
                    format!(
                        "struct `{struct_name}` has no field `{field}`"
                    ),
                );
                Type::Error
            }
        }
    }

    fn check_enum_constructor(
        &mut self,
        enum_name: &'a str,
        case_name: &'a str,
        payload: Option<&ast::Expr<'a>>,
        span: ast::Span,
    ) -> Type<'a> {
        let payload_type = payload.map(|expr| self.check_expr(expr));

        if enum_name == "Option" {
            return self.check_option_constructor(case_name, payload, payload_type, span);
        }

        if enum_name == "Result" {
            return self.check_result_constructor(case_name, payload, payload_type, span);
        }

        // User-defined enum.
        let info = match self.enums.get(enum_name) {
            Some(i) => i,
            None => {
                self.error_span(span, format!("unknown enum `{enum_name}`"));
                return Type::Error;
            }
        };

        let case = match info.cases.iter().find(|c| c.name == case_name) {
            Some(c) => c,
            None => {
                self.error_span(
                    span,
                    format!("case `{case_name}` not found in enum `{enum_name}`"),
                );
                return Type::Error;
            }
        };

        match (&case.payload, payload_type) {
            (Some(expected), Some(actual)) => {
                let expected_ty = self.resolve_ast_type(expected);
                if !is_assignable(&expected_ty, &actual) {
                    self.error_span(
                        span,
                        format!(
                            "enum case `{case_name}` expected payload type `{expected_ty}`, found type `{actual}`"
                        ),
                    );
                }
            }
            (Some(_), None) => {
                self.error_span(span, format!("`{case_name}` requires a payload"));
            }
            (None, Some(_)) => {
                self.error_span(span, format!("`{case_name}` cannot have a payload"));
            }
            (None, None) => {}
        }

        Type::Named { name: enum_name }
    }

    fn check_option_constructor(
        &mut self,
        case_name: &'a str,
        payload: Option<&ast::Expr<'a>>,
        payload_type: Option<Type<'a>>,
        span: ast::Span,
    ) -> Type<'a> {
        match case_name {
            "Some" => match payload_type {
                Some(t) => Type::Option { inner: Box::new(t) },
                None => {
                    self.error_span(span, "`Some` requires a payload");
                    Type::Error
                }
            },
            "None" => {
                if payload.is_some() {
                    self.error_span(span, "`None` cannot have a payload");
                }
                Type::None
            }
            _ => {
                self.error_span(span, format!("unknown Option case `{case_name}`"));
                Type::Error
            }
        }
    }

    fn check_result_constructor(
        &mut self,
        case_name: &'a str,
        _payload: Option<&ast::Expr<'a>>,
        payload_type: Option<Type<'a>>,
        span: ast::Span,
    ) -> Type<'a> {
        match case_name {
            "Ok" => match payload_type {
                Some(t) => Type::Generic {
                    base: "Result",
                    args: vec![t, Type::Never],
                },
                None => {
                    self.error_span(span, "`Ok` requires a payload");
                    Type::Error
                }
            },
            "Err" => match payload_type {
                Some(e) => Type::Generic {
                    base: "Result",
                    args: vec![Type::Never, e],
                },
                None => {
                    self.error_span(span, "`Err` requires a payload");
                    Type::Error
                }
            },
            _ => {
                self.error_span(span, format!("unknown Result case `{case_name}`"));
                Type::Error
            }
        }
    }

    fn check_match(
        &mut self,
        scrutinee: &ast::Expr<'a>,
        arms: &'a [ast::MatchArm<'a>],
        span: ast::Span,
    ) -> Type<'a> {
        if arms.is_empty() {
            self.error_span(span, "match expression must have at least one arm");
            return Type::Error;
        }

        let scrutinee_type = self.check_expr(scrutinee);
        let mut result_type: Option<Type<'a>> = None;

        for arm in arms {
            self.scopes.push(HashMap::new());
            self.check_pattern(&arm.pattern, &scrutinee_type);
            let arm_type = self.check_expr(&arm.body);
            self.scopes.pop();

            match &result_type {
                Some(expected) => {
                    if !is_assignable(expected, &arm_type) {
                        self.error_at_expr(
                            &arm.body,
                            format!(
                                "match arm has type `{arm_type}`, expected type `{expected}`"
                            ),
                        );
                    }
                }
                None => result_type = Some(arm_type),
            }
        }

        result_type.unwrap_or(Type::None)
    }

    fn check_pattern(&mut self, pattern: &ast::Pattern<'a>, scrutinee_type: &Type<'a>) {
        match pattern {
            ast::Pattern::Wildcard { .. } => {}
            ast::Pattern::Identifier { name, .. } => {
                self.declare_var(name, scrutinee_type.clone());
            }
            ast::Pattern::Literal { expr, span } => {
                let literal_type = self.check_expr(expr);
                if !is_assignable(scrutinee_type, &literal_type) {
                    self.error_span(
                        *span,
                        format!(
                            "literal pattern has type `{literal_type}`, expected type `{scrutinee_type}`"
                        ),
                    );
                }
            }
            ast::Pattern::Constructor {
                name,
                payload,
                span,
            } => {
                self.check_constructor_pattern(name, payload.as_deref(), *span, scrutinee_type);
            }
            ast::Pattern::Struct { span, .. } | ast::Pattern::Tuple { span, .. } => {
                self.error_span(*span, "struct/tuple patterns are not supported in v2 typeck");
            }
        }
    }

    fn check_constructor_pattern(
        &mut self,
        name: &'a str,
        payload: Option<&ast::Pattern<'a>>,
        span: ast::Span,
        scrutinee_type: &Type<'a>,
    ) {
        // Built-in Option cases.
        if name == "Some" || name == "None" {
            match scrutinee_type {
                Type::Option { inner } => {
                    if name == "None" {
                        if payload.is_some() {
                            self.error_span(span, "`None` pattern cannot have a payload");
                        }
                    } else if let Some(p) = payload {
                        self.check_pattern(p, inner);
                    } else {
                        self.error_span(span, "`Some` pattern requires a payload");
                    }
                    return;
                }
                _ if scrutinee_type.is_error() => return,
                _ => {
                    self.error_span(
                        span,
                        format!(
                            "`{name}` is not a case of type `{scrutinee_type}`"
                        ),
                    );
                    return;
                }
            }
        }

        // Built-in Result cases.
        if name == "Ok" || name == "Err" {
            match scrutinee_type {
                Type::Generic { base: "Result", args } if args.len() == 2 => {
                    let expected_payload = if name == "Ok" { &args[0] } else { &args[1] };
                    if let Some(p) = payload {
                        self.check_pattern(p, expected_payload);
                    } else {
                        self.error_span(span, format!("`{name}` pattern requires a payload"));
                    }
                    return;
                }
                _ if scrutinee_type.is_error() => return,
                _ => {
                    self.error_span(
                        span,
                        format!(
                            "`{name}` is not a case of type `{scrutinee_type}`"
                        ),
                    );
                    return;
                }
            }
        }

        // User-defined enum cases.
        let enum_name = match self.case_to_enum.get(name).copied() {
            Some(n) => n,
            None => {
                self.error_span(span, format!("unknown constructor `{name}`"));
                return;
            }
        };

        let info = match self.enums.get(enum_name) {
            Some(i) => i,
            None => return,
        };

        if !scrutinee_type.is_error() && !matches!(scrutinee_type, Type::Named { name } if *name == enum_name) {
            self.error_span(
                span,
                format!("`{name}` is not a case of type `{scrutinee_type}`"),
            );
            return;
        }

        let case = match info.cases.iter().find(|c| c.name == name) {
            Some(c) => c,
            None => {
                self.error_span(span, format!("case `{name}` not found in enum `{enum_name}`"));
                return;
            }
        };

        if let Some(payload_type) = &case.payload {
            let resolved_payload = self.resolve_ast_type(payload_type);
            if let Some(p) = payload {
                self.check_pattern(p, &resolved_payload);
            } else {
                self.error_span(span, format!("`{name}` pattern requires a payload"));
            }
        } else if payload.is_some() {
            self.error_span(span, format!("`{name}` pattern cannot have a payload"));
        }
    }

    fn check_binary(
        &mut self,
        op: ast::BinOp,
        left: &ast::Expr<'a>,
        right: &ast::Expr<'a>,
        span: ast::Span,
    ) -> Type<'a> {
        let left_type = self.check_expr(left);
        let right_type = self.check_expr(right);

        use ast::BinOp::*;
        match op {
            Add => {
                if left_type.is_error() || right_type.is_error() {
                    return Type::Named { name: "number" };
                }
                if Self::is_number(&left_type) && Self::is_number(&right_type) {
                    Type::Named { name: "number" }
                } else if Self::is_string(&left_type) && Self::is_string(&right_type) {
                    Type::Named { name: "string" }
                } else {
                    self.error_span(
                        span,
                        format!(
                            "cannot add types `{left_type}` and `{right_type}`"
                        ),
                    );
                    Type::Error
                }
            }
            Sub | Mul | Div | Mod => {
                self.expect_number(&left_type, left.span());
                self.expect_number(&right_type, right.span());
                Type::Named { name: "number" }
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                if left_type.is_error() || right_type.is_error() {
                    return Type::Named { name: "boolean" };
                }
                if left_type == right_type
                    && (Self::is_number(&left_type)
                        || Self::is_string(&left_type)
                        || Self::is_boolean(&left_type))
                {
                    Type::Named { name: "boolean" }
                } else {
                    self.error_span(
                        span,
                        format!(
                            "cannot compare types `{left_type}` and `{right_type}`"
                        ),
                    );
                    Type::Named { name: "boolean" }
                }
            }
            And | Or => {
                self.expect_boolean(&left_type, left.span());
                self.expect_boolean(&right_type, right.span());
                Type::Named { name: "boolean" }
            }
            _ => {
                self.error_span(span, format!("binary operator `{op:?}` is not supported in v2 typeck"));
                Type::Error
            }
        }
    }

    fn check_unary(
        &mut self,
        op: ast::UnOp,
        operand: &ast::Expr<'a>,
        _span: ast::Span,
    ) -> Type<'a> {
        let operand_type = self.check_expr(operand);
        match op {
            ast::UnOp::Neg => {
                self.expect_number(&operand_type, operand.span());
                Type::Named { name: "number" }
            }
            ast::UnOp::Not => {
                self.expect_boolean(&operand_type, operand.span());
                Type::Named { name: "boolean" }
            }
        }
    }

    fn check_call(
        &mut self,
        callee: &ast::Expr<'a>,
        args: &'a [ast::Expr<'a>],
        span: ast::Span,
    ) -> Type<'a> {
        let callee_type = self.check_expr(callee);

        match callee_type {
            Type::Function { params, ret } => {
                if params.len() != args.len() {
                    self.error_span(
                        span,
                        format!(
                            "expected {} argument{}, found {}",
                            params.len(),
                            if params.len() == 1 { "" } else { "s" },
                            args.len()
                        ),
                    );
                } else {
                    for (expected, arg) in params.iter().zip(args.iter()) {
                        let arg_type = self.check_expr(arg);
                        if !is_assignable(expected, &arg_type) {
                            self.error_at_expr(
                                arg,
                                format!(
                                    "expected argument type `{expected}`, found type `{arg_type}`"
                                ),
                            );
                        }
                    }
                }
                *ret
            }
            Type::Error => Type::Error,
            other => {
                self.error_span(span, format!("value of type `{other}` is not callable"));
                Type::Error
            }
        }
    }
}
