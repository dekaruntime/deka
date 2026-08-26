//! Type parsing.

use crate::ast::{alloc, alloc_slice, Type};
use crate::lexer::TokenKind;

use super::util::token_name;
use super::Parser;

impl<'a> Parser<'a> {
    pub(super) fn parse_type(&mut self) -> Option<Type<'a>> {
        self.parse_type_primary()
    }

    fn parse_type_primary(&mut self) -> Option<Type<'a>> {
        self.skip_newlines();
        let (start, start_byte) = self.span_start();

        if self.eat(TokenKind::Fn) {
            // Function type: `fn(T, U) R`.
            self.expect(TokenKind::LParen)?;
            let mut params = Vec::new();
            if !self.at(TokenKind::RParen) {
                loop {
                    params.push(self.parse_type()?);
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                    self.skip_newlines();
                }
                self.skip_newlines();
            }
            self.skip_newlines();
            self.expect(TokenKind::RParen)?;
            let ret = self.parse_type()?;
            Some(Type::Function {
                params: alloc_slice(self.arena, params),
                ret: alloc(self.arena, ret),
                span: self.span_from(start, start_byte),
            })
        } else if self.eat(TokenKind::LParen) {
            // Grouped type `(T)`.
            let ty = self.parse_type()?;
            self.expect(TokenKind::RParen)?;
            Some(ty)
        } else if self.at(TokenKind::Identifier) {
            let name = self.bump_str(self.current_text());
            let span = self.current_span();
            self.advance();

            if self.eat(TokenKind::Lt) {
                let mut args = Vec::new();
                loop {
                    args.push(self.parse_type()?);
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                    self.skip_newlines();
                }
                self.skip_newlines();
                self.expect(TokenKind::Gt)?;
                Some(Type::Generic {
                    base: name,
                    args: alloc_slice(self.arena, args),
                    span: self.span_from(start, start_byte),
                })
            } else {
                Some(Type::Named { name, span })
            }
        } else {
            self.error(format!(
                "expected type, found `{}`",
                token_name(self.current_kind())
            ));
            None
        }
    }
}
