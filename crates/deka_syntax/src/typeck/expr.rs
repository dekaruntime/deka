//! Expression typechecking.

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
            ast::Expr::FieldAccess { span, .. } => {
                self.error_span(*span, "field access is not yet supported in v2 typeck");
                Type::Error
            }
            ast::Expr::Paren { expr, .. } => self.check_expr(expr),
            _ => {
                self.error_at_expr(expr, "unsupported expression in v2 typeck");
                Type::Error
            }
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
