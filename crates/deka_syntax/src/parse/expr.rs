//! Expression parsing (Pratt parser).

use crate::ast::{alloc, alloc_slice, Expr, StructLiteralField, Type, UnOp};
use crate::lexer::TokenKind;

use super::util::{infix_info, token_name};
use super::Parser;

impl<'a> Parser<'a> {
    pub(super) fn parse_expression(&mut self) -> Option<Expr<'a>> {
        self.skip_newlines();
        self.parse_expr(0)
    }

    fn parse_expr(&mut self, min_prec: u8) -> Option<Expr<'a>> {
        self.skip_newlines();
        let (start, start_byte) = self.span_start();
        let mut left = self.parse_prefix()?;

        loop {
            // Optional-semicolon rule: do not eagerly skip newlines here.
            // A newline terminates the expression unless the following token
            // clearly continues it (a leading `.` for method chaining, or a
            // binary operator). We peek past newlines to make that decision
            // without consuming a statement terminator.
            let next_kind = self.peek_after_newlines();

            // Postfix member access: allow newline before `.` for chaining.
            if next_kind == TokenKind::Dot {
                self.skip_newlines();
                self.advance(); // `.`
                let field = self.expect_identifier()?;
                let span = self.span_from(start, start_byte);
                left = Expr::FieldAccess {
                    object: alloc(self.arena, left),
                    field,
                    span,
                };
                continue;
            }

            // Explicit type arguments: `id<number>(args)`. Do not span newlines
            // before `<`; a newline should terminate the statement instead.
            let type_args = if self.at(TokenKind::Lt) && self.next_lt_is_type_args() {
                self.try_parse_type_args().unwrap_or(&[])
            } else {
                &[]
            };

            if self.at(TokenKind::LParen) {
                self.advance();
                let mut args = Vec::new();
                if !self.at(TokenKind::RParen) {
                    loop {
                        args.push(self.parse_expression()?);
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                        self.skip_newlines();
                    }
                }
                self.expect(TokenKind::RParen)?;
                let span = self.span_from(start, start_byte);

                // Built-in prelude enum constructors: Some/Ok/Err take one payload.
                if type_args.is_empty() {
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
                }

                left = Expr::Call {
                    callee: alloc(self.arena, left),
                    type_args,
                    args: alloc_slice(self.arena, args),
                    span,
                };
                continue;
            }

            // Index access: `arr[0]` or `obj["key"]`.
            if self.at(TokenKind::LBracket) {
                self.advance();
                let index = self.parse_expression()?;
                self.expect(TokenKind::RBracket)?;
                let span = self.span_from(start, start_byte);
                left = Expr::IndexAccess {
                    object: alloc(self.arena, left),
                    index: alloc(self.arena, index),
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
                            let (field_start, field_start_byte) = self.span_start();
                            let field_name = self.expect_identifier()?;
                            self.expect(TokenKind::Colon)?;
                            let value = self.parse_expression()?;
                            fields.push(StructLiteralField {
                                name: field_name,
                                value,
                                span: self.span_from(field_start, field_start_byte),
                            });
                            if !self.eat(TokenKind::Comma) {
                                break;
                            }
                            self.skip_newlines();
                        }
                    }
                    self.expect(TokenKind::RBrace)?;
                    let span = self.span_from(start, start_byte);
                    left = Expr::StructLiteral {
                        name: struct_name,
                        fields: alloc_slice(self.arena, fields),
                        span,
                    };
                    continue;
                }
            }

            let (lbp, rbp, op) = match infix_info(next_kind) {
                Some(info) => info,
                None => break,
            };

            if lbp < min_prec {
                break;
            }

            // Commit to the binary operator: skip any newlines before it, then
            // the operator itself, then any newlines after it, then the RHS.
            self.skip_newlines();
            self.advance();
            self.skip_newlines();
            let right = self.parse_expr(rbp)?;
            let span = self.span_from(start, start_byte);

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
        self.skip_newlines();
        let (start, start_byte) = self.span_start();

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
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::String => {
                let value = self.bump_str(self.current_text());
                self.advance();
                Some(Expr::String {
                    value,
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::BacktickString => {
                let value = self.bump_str(self.current_text());
                self.advance();
                Some(Expr::TemplateLiteral {
                    parts: alloc_slice(self.arena, vec![crate::ast::TemplatePart::Text(value)]),
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::True => {
                self.advance();
                Some(Expr::Boolean {
                    value: true,
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::False => {
                self.advance();
                Some(Expr::Boolean {
                    value: false,
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::None => {
                self.advance();
                Some(Expr::None {
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::Identifier => {
                let name = self.bump_str(self.current_text());
                self.advance();
                Some(Expr::Identifier {
                    name,
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(TokenKind::RParen)?;
                Some(Expr::Paren {
                    expr: alloc(self.arena, expr),
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_expr(12)?;
                Some(Expr::Unary {
                    op: UnOp::Neg,
                    operand: alloc(self.arena, operand),
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::Not => {
                self.advance();
                let operand = self.parse_expr(12)?;
                Some(Expr::Unary {
                    op: UnOp::Not,
                    operand: alloc(self.arena, operand),
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::Match => {
                self.advance();
                let scrutinee = alloc(self.arena, self.parse_expression()?);
                let arms = self.parse_match_arms()?;
                Some(Expr::Match {
                    scrutinee,
                    arms,
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::Await => {
                self.advance();
                let operand = self.parse_expr(12)?;
                Some(Expr::Await {
                    expr: alloc(self.arena, operand),
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::Fn => self.parse_fn_expression(start, start_byte),
            TokenKind::Unsafe => self.parse_unsafe_expression(start, start_byte),
            TokenKind::Lt => self.parse_jsx(start, start_byte),
            TokenKind::LBracket => {
                self.advance();
                let mut elements = Vec::new();
                if !self.at(TokenKind::RBracket) {
                    loop {
                        elements.push(self.parse_spreadable_expr()?);
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                        self.skip_newlines();
                        if self.at(TokenKind::RBracket) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RBracket)?;
                Some(Expr::Array {
                    elements: alloc_slice(self.arena, elements),
                    span: self.span_from(start, start_byte),
                })
            }
            TokenKind::LBrace => {
                self.advance();
                let mut fields = Vec::new();
                if !self.at(TokenKind::RBrace) {
                    loop {
                        let (field_start, field_start_byte) = self.span_start();
                        if self.eat(TokenKind::Spread) {
                            let expr = self.parse_expression()?;
                            fields.push(crate::ast::ObjectField {
                                key: "",
                                value: expr,
                                span: self.span_from(field_start, field_start_byte),
                            });
                        } else {
                            let key = self.expect_object_key()?;
                            self.expect(TokenKind::Colon)?;
                            let value = self.parse_expression()?;
                            fields.push(crate::ast::ObjectField {
                                key,
                                value,
                                span: self.span_from(field_start, field_start_byte),
                            });
                        }
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                        self.skip_newlines();
                        if self.at(TokenKind::RBrace) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RBrace)?;
                Some(Expr::Object {
                    fields: alloc_slice(self.arena, fields),
                    span: self.span_from(start, start_byte),
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

    /// Parse an `unsafe { ... }` raw JavaScript block.
    ///
    /// The lexer has already tokenised the contents; we skip tokens until we
    /// find the matching `}` and extract the literal source bytes between the
    /// braces. The contents are not parsed as DekaScript.
    fn parse_unsafe_expression(
        &mut self,
        start: crate::ast::Pos,
        start_byte: usize,
    ) -> Option<Expr<'a>> {
        self.advance(); // `unsafe`

        if !self.at(TokenKind::LBrace) {
            self.error("expected `{` after `unsafe`");
            return None;
        }

        let body_start_byte = self.current_span().byte_start + 1; // after `{`
        self.advance(); // `{`

        let mut depth = 1;
        let mut body_end_byte = body_start_byte;
        while depth > 0 && !self.at_end() {
            if self.at(TokenKind::LBrace) {
                depth += 1;
                self.advance();
            } else if self.at(TokenKind::RBrace) {
                depth -= 1;
                if depth == 0 {
                    body_end_byte = self.current_span().byte_start;
                }
                self.advance();
            } else {
                self.advance();
            }
        }

        if depth != 0 {
            self.error("unterminated `unsafe` block; expected `}`");
            return None;
        }

        let source = &self.source[body_start_byte..body_end_byte];
        Some(Expr::Unsafe {
            source: self.bump_str(source),
            span: self.span_from(start, start_byte),
        })
    }

    /// Parse an anonymous function expression: `fn (x: number) number { ... }`.
    fn parse_fn_expression(
        &mut self,
        start: crate::ast::Pos,
        start_byte: usize,
    ) -> Option<Expr<'a>> {
        self.advance(); // `fn`
        self.expect(TokenKind::LParen)?;
        let params = self.parse_params()?;
        self.expect(TokenKind::RParen)?;

        let return_type = if self.at(TokenKind::LBrace) {
            None
        } else if self.eat(TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            Some(self.parse_type()?)
        };

        let body = self.parse_block()?;
        Some(Expr::Function {
            params,
            return_type,
            body,
            span: self.span_from(start, start_byte),
        })
    }

    /// Parse an expression that may be a spread element (`...expr`).
    fn parse_spreadable_expr(&mut self) -> Option<Expr<'a>> {
        let (start, start_byte) = self.span_start();
        if self.eat(TokenKind::Spread) {
            let expr = self.parse_expression()?;
            Some(Expr::Spread {
                expr: alloc(self.arena, expr),
                span: self.span_from(start, start_byte),
            })
        } else {
            self.parse_expression()
        }
    }

    /// Parse an object literal key: identifier or string.
    fn expect_object_key(&mut self) -> Option<&'a str> {
        match self.current_kind() {
            TokenKind::Identifier => {
                let key = self.bump_str(self.current_text());
                self.advance();
                Some(key)
            }
            TokenKind::String => {
                let key = self.bump_str(self.current_text());
                self.advance();
                Some(key)
            }
            _ => {
                self.error(format!(
                    "expected object key, found `{}`",
                    token_name(self.current_kind())
                ));
                None
            }
        }
    }

    /// Peek at the `<` at the current position and decide whether it opens an
    /// explicit type argument list that is immediately followed by a call `(`.
    /// This prevents `<` in comparison expressions (`a < b`) from being parsed
    /// as (and erroring during) a speculative type argument list.
    fn next_lt_is_type_args(&self) -> bool {
        if !self.at(TokenKind::Lt) {
            return false;
        }
        let mut depth = 1usize;
        let mut i = self.pos + 1;
        while i < self.tokens.len() {
            match self.tokens[i].kind {
                TokenKind::Lt => depth += 1,
                TokenKind::Gt => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return i + 1 < self.tokens.len()
                            && self.tokens[i + 1].kind == TokenKind::LParen;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// Try to parse explicit type arguments `<T, U>`.
    /// On success, returns the parsed types and advances the cursor.
    /// On failure, leaves the cursor unchanged.
    fn try_parse_type_args(&mut self) -> Option<&'a [Type<'a>]> {
        let saved_pos = self.pos;
        let saved_prev = self.prev.clone();

        if !self.eat(TokenKind::Lt) {
            return None;
        }

        let mut args = Vec::new();
        if !self.at(TokenKind::Gt) {
            loop {
                match self.parse_type() {
                    Some(ty) => args.push(ty),
                    None => {
                        self.pos = saved_pos;
                        self.prev = saved_prev;
                        return None;
                    }
                }
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }

        if !self.eat(TokenKind::Gt) {
            self.pos = saved_pos;
            self.prev = saved_prev;
            return None;
        }

        Some(alloc_slice(self.arena, args))
    }
}
