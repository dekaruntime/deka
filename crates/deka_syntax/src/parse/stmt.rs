//! Statement parsing.

use crate::ast::{alloc_slice, Param, Pos, Program, Stmt, TypeParam};
use crate::lexer::{TokenKind};

use super::util::token_name;
use super::Parser;

impl<'a> Parser<'a> {
    pub(super) fn parse_program(&mut self) -> Option<Program<'a>> {
        let start = self.current_span().start;
        let mut statements = Vec::new();

        while !self.at_end() {
            match self.parse_statement(false) {
                Some(stmt) => statements.push(stmt),
                None => self.synchronize(),
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

    pub(super) fn parse_block(&mut self) -> Option<&'a [Stmt<'a>]> {
        self.expect(TokenKind::LBrace)?;
        let mut statements = Vec::new();

        while !self.at(TokenKind::RBrace) && !self.at_end() {
            match self.parse_statement(true) {
                Some(stmt) => statements.push(stmt),
                None => self.synchronize(),
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
