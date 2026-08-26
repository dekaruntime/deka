//! Expression parsing (Pratt parser).

use crate::ast::{alloc, alloc_slice, Expr, StructLiteralField, UnOp};
use crate::lexer::TokenKind;

use super::util::{infix_info, token_name};
use super::Parser;

impl<'a> Parser<'a> {
    pub(super) fn parse_expression(&mut self) -> Option<Expr<'a>> {
        self.parse_expr(0)
    }

    fn parse_expr(&mut self, min_prec: u8) -> Option<Expr<'a>> {
        let start = self.current_span().start;
        let mut left = self.parse_prefix()?;

        loop {
            // Postfix member access and calls bind tighter than any binary operator.
            if self.eat(TokenKind::Dot) {
                let field = self.expect_identifier()?;
                let span = self.span_from(start);
                left = Expr::FieldAccess {
                    object: alloc(self.arena, left),
                    field,
                    span,
                };
                continue;
            }

            if self.at(TokenKind::LParen) {
                self.advance();
                let mut args = Vec::new();
                if !self.at(TokenKind::RParen) {
                    loop {
                        args.push(self.parse_expression()?);
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RParen)?;
                let span = self.span_from(start);

                // Built-in prelude enum constructors: Some/Ok/Err take one payload.
                if let Expr::Identifier { name, .. } = &left {
                    if let Some((enum_name, _requires_payload)) = builtin_enum_constructor(name) {
                        if args.len() == 1 {
                            left = Expr::EnumConstructor {
                                enum_name: self.bump_str(enum_name),
                                case_name: name,
                                payload: Some(alloc(self.arena, args.into_iter().next().unwrap())),
                                span,
                            };
                            continue;
                        }
                    }
                }

                left = Expr::Call {
                    callee: alloc(self.arena, left),
                    type_args: &[],
                    args: alloc_slice(self.arena, args),
                    span,
                };
                continue;
            }

            // Struct literal: `Name { field: expr, ... }`.
            // We peek ahead to confirm this is really a struct literal and not
            // a block/record-like construct (e.g. a match body after the
            // scrutinee). It must be empty `{}` or start with `field: expr`.
            if self.at(TokenKind::LBrace) && self.looks_like_struct_literal() {
                if let Expr::Identifier { name, .. } = &left {
                    let struct_name = *name;
                    self.advance();
                    let mut fields = Vec::new();
                    if !self.at(TokenKind::RBrace) {
                        loop {
                            let field_start = self.current_span().start;
                            let field_name = self.expect_identifier()?;
                            self.expect(TokenKind::Colon)?;
                            let value = self.parse_expression()?;
                            fields.push(StructLiteralField {
                                name: field_name,
                                value,
                                span: self.span_from(field_start),
                            });
                            if !self.eat(TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(TokenKind::RBrace)?;
                    let span = self.span_from(start);
                    left = Expr::StructLiteral {
                        name: struct_name,
                        fields: alloc_slice(self.arena, fields),
                        span,
                    };
                    continue;
                }
            }

            let (lbp, rbp, op) = match infix_info(self.current_kind()) {
                Some(info) => info,
                None => break,
            };

            if lbp < min_prec {
                break;
            }

            self.advance();
            let right = self.parse_expr(rbp)?;
            let span = self.span_from(start);

            left = Expr::Binary {
                op,
                left: alloc(self.arena, left),
                right: alloc(self.arena, right),
                span,
            };
        }

        Some(left)
    }

    fn parse_prefix(&mut self) -> Option<Expr<'a>> {
        let start = self.current_span().start;

        match self.current_kind() {
            TokenKind::Number => {
                let text = self.current_text();
                let value = match text.parse::<f64>() {
                    Ok(v) => v,
                    Err(_) => {
                        self.error(format!("invalid number literal `{}`", text));
                        return None;
                    }
                };
                self.advance();
                Some(Expr::Number {
                    value,
                    span: self.span_from(start),
                })
            }
            TokenKind::String => {
                let value = self.bump_str(self.current_text());
                self.advance();
                Some(Expr::String {
                    value,
                    span: self.span_from(start),
                })
            }
            TokenKind::True => {
                self.advance();
                Some(Expr::Boolean {
                    value: true,
                    span: self.span_from(start),
                })
            }
            TokenKind::False => {
                self.advance();
                Some(Expr::Boolean {
                    value: false,
                    span: self.span_from(start),
                })
            }
            TokenKind::None => {
                self.advance();
                Some(Expr::None {
                    span: self.span_from(start),
                })
            }
            TokenKind::Identifier => {
                let name = self.bump_str(self.current_text());
                self.advance();
                Some(Expr::Identifier {
                    name,
                    span: self.span_from(start),
                })
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                Some(Expr::Paren {
                    expr: alloc(self.arena, expr),
                    span: self.span_from(start),
                })
            }
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_expr(12)?;
                Some(Expr::Unary {
                    op: UnOp::Neg,
                    operand: alloc(self.arena, operand),
                    span: self.span_from(start),
                })
            }
            TokenKind::Not => {
                self.advance();
                let operand = self.parse_expr(12)?;
                Some(Expr::Unary {
                    op: UnOp::Not,
                    operand: alloc(self.arena, operand),
                    span: self.span_from(start),
                })
            }
            TokenKind::Match => {
                self.advance();
                let scrutinee = alloc(self.arena, self.parse_expression()?);
                let arms = self.parse_match_arms()?;
                Some(Expr::Match {
                    scrutinee,
                    arms,
                    span: self.span_from(start),
                })
            }
            _ => {
                self.error(format!(
                    "expected expression, found `{}`",
                    token_name(self.current_kind())
                ));
                None
            }
        }
    }
}

/// Returns `(enum_name, requires_payload)` for built-in prelude enum
/// constructors that are parsed specially instead of as function calls.
fn builtin_enum_constructor(name: &str) -> Option<(&'static str, bool)> {
    match name {
        "Some" => Some(("Option", true)),
        "Ok" => Some(("Result", true)),
        "Err" => Some(("Result", true)),
        _ => None,
    }
}

impl<'a> Parser<'a> {
    /// Peek at the tokens after the current `{` to decide whether this is a
    /// struct literal (`Name {}` or `Name { a: 1 }`) or something else.
    fn looks_like_struct_literal(&self) -> bool {
        let next = self.tokens.get(self.pos + 1).map(|t| t.kind);
        match next {
            Some(TokenKind::RBrace) => true,
            Some(TokenKind::Identifier) => {
                let next_next = self.tokens.get(self.pos + 2).map(|t| t.kind);
                matches!(next_next, Some(TokenKind::Colon))
            }
            _ => false,
        }
    }
}
