//! Statement parsing.

use crate::ast::{alloc_slice, EnumCase, Param, Pos, Program, StructField, Stmt, TypeParam};
use crate::lexer::TokenKind;

use super::util::token_name;
use super::Parser;

impl<'a> Parser<'a> {
    pub(super) fn parse_program(&mut self) -> Option<Program<'a>> {
        let start = self.current_span().start;
        let mut statements = Vec::new();

        while !self.at_end() {
            let before = self.pos;
            match self.parse_statement(false) {
                Some(stmt) => statements.push(stmt),
                None => {
                    self.synchronize();
                    // If synchronize() made no progress, force advancement
                    // so we don't loop forever on unexpected tokens.
                    if self.pos == before && !self.at_end() {
                        self.advance();
                    }
                }
            }
        }

        Some(Program {
            statements: alloc_slice(self.arena, statements),
            span: self.span_from(start),
        })
    }

    pub(super) fn parse_statement(&mut self, in_block: bool) -> Option<Stmt<'a>> {
        let start = self.current_span().start;

        match self.current_kind() {
            TokenKind::Const | TokenKind::Let => {
                let is_const = self.current_kind() == TokenKind::Const;
                self.advance();

                let name = self.expect_identifier()?;
                let ty = if self.eat(TokenKind::Colon) {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.expect(TokenKind::Eq)?;
                let value = self.parse_expression()?;
                self.expect_statement_end(in_block)?;

                let span = self.span_from(start);
                if is_const {
                    Some(Stmt::Const {
                        name,
                        ty,
                        value,
                        span,
                    })
                } else {
                    Some(Stmt::Let {
                        name,
                        ty,
                        value,
                        span,
                    })
                }
            }

            TokenKind::Function => self.parse_function_statement(start),

            TokenKind::Fn => self.parse_receiver_method_statement(start),

            TokenKind::Struct => self.parse_struct_statement(start),

            TokenKind::Enum => self.parse_enum_statement(start),

            TokenKind::Type => self.parse_type_alias_statement(start),

            TokenKind::Return => {
                self.advance();
                let value =
                    if self.at(TokenKind::Semicolon) || (in_block && self.at(TokenKind::RBrace)) {
                        None
                    } else {
                        Some(self.parse_expression()?)
                    };
                self.expect_statement_end(in_block)?;
                Some(Stmt::Return {
                    value,
                    span: self.span_from(start),
                })
            }

            _ => {
                let expr = self.parse_expression()?;
                self.expect_statement_end(in_block)?;
                Some(Stmt::Expr {
                    expr,
                    span: self.span_from(start),
                })
            }
        }
    }

    fn parse_function_statement(&mut self, start: Pos) -> Option<Stmt<'a>> {
        self.advance(); // `function`

        let name = self.expect_identifier()?;
        let type_params = if self.at(TokenKind::Lt) {
            self.parse_type_params()?
        } else {
            &[]
        };

        self.expect(TokenKind::LParen)?;
        let params = self.parse_params()?;
        self.expect(TokenKind::RParen)?;

        let return_type = if self.eat(TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = self.parse_block()?;

        Some(Stmt::Function {
            name,
            type_params,
            params,
            return_type,
            body,
            span: self.span_from(start),
        })
    }

    fn parse_receiver_method_statement(&mut self, start: Pos) -> Option<Stmt<'a>> {
        self.advance(); // `fn`

        let receiver_type = self.expect_identifier()?;
        self.expect(TokenKind::Dot)?;
        let name = self.expect_identifier()?;

        let type_params = if self.at(TokenKind::Lt) {
            self.parse_type_params()?
        } else {
            &[]
        };

        self.expect(TokenKind::LParen)?;
        let params = self.parse_params()?;
        self.expect(TokenKind::RParen)?;

        let return_type = if self.eat(TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = self.parse_block()?;

        Some(Stmt::ReceiverMethod {
            receiver_type,
            name,
            type_params,
            params,
            return_type,
            body,
            span: self.span_from(start),
        })
    }

    fn parse_struct_statement(&mut self, start: Pos) -> Option<Stmt<'a>> {
        self.advance(); // `struct`

        let name = self.expect_identifier()?;
        let type_params = if self.at(TokenKind::Lt) {
            self.parse_type_params()?
        } else {
            &[]
        };

        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();

        while !self.at(TokenKind::RBrace) && !self.at_end() {
            let field_start = self.current_span().start;
            let field_name = self.expect_identifier()?;
            self.expect(TokenKind::Colon)?;
            let field_type = self.parse_type()?;
            let default_value = if self.eat(TokenKind::Eq) {
                Some(self.parse_expression()?)
            } else {
                None
            };
            fields.push(StructField {
                name: field_name,
                ty: field_type,
                default_value,
                span: self.span_from(field_start),
            });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }

        self.expect(TokenKind::RBrace)?;

        Some(Stmt::Struct {
            name,
            type_params,
            fields: alloc_slice(self.arena, fields),
            embeds: alloc_slice(self.arena, Vec::new()),
            span: self.span_from(start),
        })
    }

    fn parse_enum_statement(&mut self, start: Pos) -> Option<Stmt<'a>> {
        self.advance(); // `enum`

        let name = self.expect_identifier()?;
        let type_params = if self.at(TokenKind::Lt) {
            self.parse_type_params()?
        } else {
            &[]
        };

        self.expect(TokenKind::LBrace)?;
        let mut cases = Vec::new();

        while !self.at(TokenKind::RBrace) && !self.at_end() {
            let case_start = self.current_span().start;
            let case_name = self.expect_identifier()?;
            let payload = if self.eat(TokenKind::LParen) {
                let ty = self.parse_type()?;
                self.expect(TokenKind::RParen)?;
                Some(ty)
            } else {
                None
            };
            cases.push(EnumCase {
                name: case_name,
                payload,
                span: self.span_from(case_start),
            });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }

        self.expect(TokenKind::RBrace)?;

        Some(Stmt::Enum {
            name,
            type_params,
            cases: alloc_slice(self.arena, cases),
            span: self.span_from(start),
        })
    }

    fn parse_type_alias_statement(&mut self, start: Pos) -> Option<Stmt<'a>> {
        self.advance(); // `type`

        let name = self.expect_identifier()?;
        let type_params = if self.at(TokenKind::Lt) {
            self.parse_type_params()?
        } else {
            &[]
        };

        self.expect(TokenKind::Eq)?;
        let value = self.parse_type()?;
        self.expect_statement_end(false)?;

        Some(Stmt::TypeAlias {
            name,
            type_params,
            value,
            span: self.span_from(start),
        })
    }

    pub(super) fn parse_block(&mut self) -> Option<&'a [Stmt<'a>]> {
        self.expect(TokenKind::LBrace)?;
        let mut statements = Vec::new();

        while !self.at(TokenKind::RBrace) && !self.at_end() {
            let before = self.pos;
            match self.parse_statement(true) {
                Some(stmt) => statements.push(stmt),
                None => {
                    self.synchronize();
                    if self.pos == before && !self.at_end() {
                        self.advance();
                    }
                }
            }
        }

        self.expect(TokenKind::RBrace)?;
        Some(alloc_slice(self.arena, statements))
    }

    pub(super) fn parse_params(&mut self) -> Option<&'a [Param<'a>]> {
        let mut params = Vec::new();

        if !self.at(TokenKind::RParen) {
            loop {
                let param_start = self.current_span().start;
                let name = self.expect_identifier()?;
                let ty = if self.eat(TokenKind::Colon) {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let default_value = if self.eat(TokenKind::Eq) {
                    Some(self.parse_expression()?)
                } else {
                    None
                };
                params.push(Param {
                    name,
                    ty,
                    default_value,
                    span: self.span_from(param_start),
                });

                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }

        Some(alloc_slice(self.arena, params))
    }

    pub(super) fn parse_type_params(&mut self) -> Option<&'a [TypeParam<'a>]> {
        self.expect(TokenKind::Lt)?;
        let mut params = Vec::new();

        loop {
            let name = self.expect_identifier()?;
            let span = self.span_from(self.prev.span.start);
            params.push(TypeParam { name, span });
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }

        self.expect(TokenKind::Gt)?;
        Some(alloc_slice(self.arena, params))
    }

    fn expect_statement_end(&mut self, in_block: bool) -> Option<()> {
        if self.eat(TokenKind::Semicolon) {
            Some(())
        } else if in_block && self.at(TokenKind::RBrace) {
            // Optional semicolon before a closing brace.
            Some(())
        } else {
            self.error(format!(
                "expected `;`, found `{}`",
                token_name(self.current_kind())
            ));
            None
        }
    }
}
