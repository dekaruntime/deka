//! Pattern parsing for `match` arms.

use crate::ast::{MatchArm, Pattern, PatternField};
use crate::lexer::TokenKind;

use super::util::token_name;
use super::Parser;

impl<'a> Parser<'a> {
    pub(super) fn parse_match_arms(&mut self) -> Option<&'a [MatchArm<'a>]> {
        self.expect(TokenKind::LBrace)?;
        let mut arms = Vec::new();

        while !self.at(TokenKind::RBrace) && !self.at_end() {
            arms.push(self.parse_match_arm()?);
            if !self.eat(TokenKind::Comma) {
                // Allow trailing comma or newline-separated arms.
                break;
            }
        }

        self.expect(TokenKind::RBrace)?;
        Some(crate::ast::alloc_slice(self.arena, arms))
    }

    fn parse_match_arm(&mut self) -> Option<MatchArm<'a>> {
        let start = self.current_span().start;
        let pattern = self.parse_pattern()?;
        self.expect(TokenKind::FatArrow)?;
        let body = self.parse_expression()?;
        Some(MatchArm {
            pattern,
            guard: None,
            body,
            span: self.span_from(start),
        })
    }

    pub(super) fn parse_pattern(&mut self) -> Option<Pattern<'a>> {
        let start = self.current_span().start;

        match self.current_kind() {
            TokenKind::Identifier => {
                let name = self.bump_str(self.current_text());
                self.advance();

                if name == "_" {
                    return Some(Pattern::Wildcard {
                        span: self.span_from(start),
                    });
                }

                if self.at(TokenKind::LBrace) {
                    // Struct pattern: `Name { field, field: p }`
                    self.advance();
                    let mut fields = Vec::new();
                    if !self.at(TokenKind::RBrace) {
                        loop {
                            let field_start = self.current_span().start;
                            let field_name = self.expect_identifier()?;
                            let pattern = if self.eat(TokenKind::Colon) {
                                self.parse_pattern()?
                            } else {
                                Pattern::Identifier {
                                    name: field_name,
                                    span: self.span_from(field_start),
                                }
                            };
                            fields.push(PatternField {
                                name: field_name,
                                pattern,
                                span: self.span_from(field_start),
                            });
                            if !self.eat(TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(TokenKind::RBrace)?;
                    return Some(Pattern::Struct {
                        name,
                        fields: crate::ast::alloc_slice(self.arena, fields),
                        span: self.span_from(start),
                    });
                }

                if self.at(TokenKind::LParen) {
                    // Constructor pattern: `Name(p)` or `Name()`
                    self.advance();
                    let payload = if self.at(TokenKind::RParen) {
                        None
                    } else {
                        Some(crate::ast::alloc(self.arena, self.parse_pattern()?))
                    };
                    self.expect(TokenKind::RParen)?;
                    return Some(Pattern::Constructor {
                        name,
                        payload,
                        span: self.span_from(start),
                    });
                }

                Some(Pattern::Identifier {
                    name,
                    span: self.span_from(start),
                })
            }

            TokenKind::Number | TokenKind::String | TokenKind::True | TokenKind::False | TokenKind::None => {
                let expr = self.parse_expression()?;
                Some(Pattern::Literal {
                    expr,
                    span: self.span_from(start),
                })
            }

            TokenKind::LParen => {
                // Tuple pattern: `(a, b)` or grouped pattern `(p)`.
                self.advance();
                if self.at(TokenKind::RParen) {
                    self.error("empty tuple pattern is not allowed".to_string());
                    return None;
                }
                let first = self.parse_pattern()?;
                if self.eat(TokenKind::RParen) {
                    return Some(first);
                }
                let mut elements = vec![first];
                while self.eat(TokenKind::Comma) {
                    if self.at(TokenKind::RParen) {
                        break;
                    }
                    elements.push(self.parse_pattern()?);
                }
                self.expect(TokenKind::RParen)?;
                Some(Pattern::Tuple {
                    elements: crate::ast::alloc_slice(self.arena, elements),
                    span: self.span_from(start),
                })
            }

            _ => {
                self.error(format!(
                    "expected pattern, found `{}`",
                    token_name(self.current_kind())
                ));
                None
            }
        }
    }
}
