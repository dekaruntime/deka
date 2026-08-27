//! Statement parsing.

use crate::ast::{alloc, alloc_slice, EnumCase, ForInit, Param, Pos, Program, StructField, Stmt, Type, TypeParam};
use crate::lexer::TokenKind;

use super::util::token_name;
use super::Parser;

impl<'a> Parser<'a> {
    pub(super) fn parse_program(&mut self) -> Option<Program<'a>> {
        self.skip_newlines();
        let (start, start_byte) = self.span_start();
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
            self.skip_newlines();
        }

        Some(Program {
            statements: alloc_slice(self.arena, statements),
            span: self.span_from(start, start_byte),
        })
    }

    pub(super) fn parse_statement(&mut self, in_block: bool) -> Option<Stmt<'a>> {
        self.skip_newlines();
        let (start, start_byte) = self.span_start();

        if self.eat(TokenKind::Semicolon) {
            return Some(Stmt::Empty {
                span: self.span_from(start, start_byte),
            });
        }

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

                let span = self.span_from(start, start_byte);
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

            TokenKind::Fn => self.parse_fn_statement(start, start_byte),

            TokenKind::Async => {
                if self.tokens.get(self.pos + 1).map(|t| t.kind) == Some(TokenKind::Fn) {
                    self.parse_fn_statement(start, start_byte)
                } else {
                    self.error("expected `fn` after `async`");
                    None
                }
            }

            TokenKind::For => self.parse_for_statement(start, start_byte),

            TokenKind::If => self.parse_if_statement(start, start_byte),

            TokenKind::LBrace => {
                let body = self.parse_block()?;
                Some(Stmt::Block {
                    body,
                    span: self.span_from(start, start_byte),
                })
            }

            TokenKind::Break => {
                self.advance();
                self.expect_statement_end(in_block)?;
                Some(Stmt::Break {
                    span: self.span_from(start, start_byte),
                })
            }

            TokenKind::Continue => {
                self.advance();
                self.expect_statement_end(in_block)?;
                Some(Stmt::Continue {
                    span: self.span_from(start, start_byte),
                })
            }

            TokenKind::Struct => self.parse_struct_statement(start, start_byte),

            TokenKind::Enum => self.parse_enum_statement(start, start_byte),

            TokenKind::Type => self.parse_type_alias_statement(start, start_byte),

            TokenKind::Import => self.parse_import_statement(start, start_byte),

            TokenKind::Export => self.parse_export_statement(start, start_byte),

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
                    span: self.span_from(start, start_byte),
                })
            }

            _ => {
                let expr = self.parse_expression()?;
                self.expect_statement_end(in_block)?;
                Some(Stmt::Expr {
                    expr,
                    span: self.span_from(start, start_byte),
                })
            }
        }
    }

    fn parse_fn_statement(&mut self, start: Pos, start_byte: usize) -> Option<Stmt<'a>> {
        let is_async = self.eat(TokenKind::Async);
        self.advance(); // `fn`

        // Receiver method: `fn (p Point) distance<T>(...): Ret { ... }`
        if self.at(TokenKind::LParen) {
            self.advance(); // `(`
            let receiver_name = self.expect_identifier()?;
            let receiver_type = self.expect_identifier()?;
            self.expect(TokenKind::RParen)?;

            let name = self.expect_identifier()?;

            let type_params = if self.at(TokenKind::Lt) {
                self.parse_type_params()?
            } else {
                &[]
            };

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

            return Some(Stmt::ReceiverMethod {
                receiver_type,
                receiver_name,
                name,
                type_params,
                params,
                return_type,
                body,
                is_async,
                span: self.span_from(start, start_byte),
            });
        }

        // Regular function: `fn add<T>(...): Ret { ... }`
        let name = self.expect_identifier()?;
        let type_params = if self.at(TokenKind::Lt) {
            self.parse_type_params()?
        } else {
            &[]
        };

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

        Some(Stmt::Function {
            name,
            type_params,
            params,
            return_type,
            body,
            is_async,
            span: self.span_from(start, start_byte),
        })
    }

    fn parse_for_statement(&mut self, start: Pos, start_byte: usize) -> Option<Stmt<'a>> {
        self.advance(); // `for`
        self.expect(TokenKind::LParen)?;

        let init = if self.at(TokenKind::Semicolon) {
            None
        } else if self.at(TokenKind::Const) {
            self.advance();
            let name = self.expect_identifier()?;
            self.expect(TokenKind::Eq)?;
            let value = self.parse_expression()?;
            Some(ForInit::Const { name, value })
        } else if self.at(TokenKind::Let) {
            self.advance();
            let name = self.expect_identifier()?;
            self.expect(TokenKind::Eq)?;
            let value = self.parse_expression()?;
            Some(ForInit::Let { name, value })
        } else {
            Some(ForInit::Expr(self.parse_expression()?))
        };

        self.expect(TokenKind::Semicolon)?;

        let condition = if self.at(TokenKind::Semicolon) {
            None
        } else {
            Some(self.parse_expression()?)
        };

        self.expect(TokenKind::Semicolon)?;

        let step = if self.at(TokenKind::RParen) {
            None
        } else {
            Some(self.parse_expression()?)
        };

        self.expect(TokenKind::RParen)?;
        let body = self.parse_block()?;

        Some(Stmt::For {
            init,
            condition,
            step,
            body,
            span: self.span_from(start, start_byte),
        })
    }

    fn parse_if_statement(&mut self, start: Pos, start_byte: usize) -> Option<Stmt<'a>> {
        self.advance(); // `if`
        self.expect(TokenKind::LParen)?;
        let condition = self.parse_expression()?;
        self.expect(TokenKind::RParen)?;
        let then_body = self.parse_block()?;

        let else_body = if self.eat(TokenKind::Else) {
            if self.at(TokenKind::If) {
                let else_start = self.span_start();
                let else_if = self.parse_if_statement(else_start.0, else_start.1)?;
                alloc_slice(self.arena, vec![else_if])
            } else {
                self.parse_block()?
            }
        } else {
            &[]
        };

        Some(Stmt::If {
            condition,
            then_body,
            else_body,
            span: self.span_from(start, start_byte),
        })
    }

    fn parse_struct_statement(&mut self, start: Pos, start_byte: usize) -> Option<Stmt<'a>> {
        self.advance(); // `struct`

        let name = self.expect_identifier()?;
        let type_params = if self.at(TokenKind::Lt) {
            self.parse_type_params()?
        } else {
            &[]
        };

        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        let mut embeds = Vec::new();

        while !self.at(TokenKind::RBrace) && !self.at_end() {
            let (field_start, field_start_byte) = self.span_start();
            let field_name = self.expect_identifier()?;

            // If the identifier is followed by `:` or `?:`, this is a regular field.
            // Otherwise it names an embedded struct (e.g. `struct Outer { Inner }`).
            if self.at(TokenKind::Colon) || self.at(TokenKind::Question) {
                let is_optional = if self.eat(TokenKind::Question) {
                    self.expect(TokenKind::Colon)?;
                    true
                } else {
                    self.expect(TokenKind::Colon)?;
                    false
                };
                let field_type = self.parse_type()?;
                let field_span = field_type.span();
                let field_type = if is_optional {
                    Type::Option {
                        inner: alloc(self.arena, field_type),
                        span: field_span,
                    }
                } else {
                    field_type
                };
                let default_value = if self.eat(TokenKind::Eq) {
                    Some(self.parse_expression()?)
                } else {
                    None
                };
                fields.push(StructField {
                    name: field_name,
                    ty: field_type,
                    default_value,
                    optional: is_optional,
                    span: self.span_from(field_start, field_start_byte),
                });
            } else {
                embeds.push(crate::ast::Embed {
                    name: field_name,
                    span: self.span_from(field_start, field_start_byte),
                });
            }

            if self.at(TokenKind::RBrace) {
                break;
            }
            if self.eat(TokenKind::Comma) {
                self.skip_newlines();
                if self.at(TokenKind::RBrace) {
                    break;
                }
                continue;
            }
            if self.eat(TokenKind::Semicolon) {
                self.skip_newlines();
                if self.at(TokenKind::RBrace) {
                    break;
                }
                continue;
            }
            if self.at(TokenKind::Newline) {
                self.skip_newlines();
                if self.at(TokenKind::RBrace) {
                    break;
                }
                continue;
            }
            self.error("expected `,` or newline between struct fields");
            break;
        }

        self.skip_newlines();
        self.expect(TokenKind::RBrace)?;

        Some(Stmt::Struct {
            name,
            type_params,
            fields: alloc_slice(self.arena, fields),
            embeds: alloc_slice(self.arena, embeds),
            span: self.span_from(start, start_byte),
        })
    }

    fn parse_enum_statement(&mut self, start: Pos, start_byte: usize) -> Option<Stmt<'a>> {
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
            let (case_start, case_start_byte) = self.span_start();
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
                span: self.span_from(case_start, case_start_byte),
            });

            if self.at(TokenKind::RBrace) {
                break;
            }
            if self.eat(TokenKind::Comma) {
                self.skip_newlines();
                if self.at(TokenKind::RBrace) {
                    break;
                }
                continue;
            }
            if self.eat(TokenKind::Semicolon) {
                self.skip_newlines();
                if self.at(TokenKind::RBrace) {
                    break;
                }
                continue;
            }
            if self.at(TokenKind::Newline) {
                self.skip_newlines();
                if self.at(TokenKind::RBrace) {
                    break;
                }
                continue;
            }
            self.error("expected `,` or newline between enum cases");
            break;
        }

        self.skip_newlines();
        self.expect(TokenKind::RBrace)?;

        Some(Stmt::Enum {
            name,
            type_params,
            cases: alloc_slice(self.arena, cases),
            span: self.span_from(start, start_byte),
        })
    }

    fn parse_type_alias_statement(&mut self, start: Pos, start_byte: usize) -> Option<Stmt<'a>> {
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
            span: self.span_from(start, start_byte),
        })
    }

    fn parse_import_statement(&mut self, start: Pos, start_byte: usize) -> Option<Stmt<'a>> {
        self.advance(); // `import`

        // Side-effect import: `import "./mod.ds";`
        if self.at(TokenKind::String) {
            let source = self.bump_str(self.current_text());
            self.advance();
            self.expect_statement_end(false)?;
            return Some(Stmt::Import {
                specifiers: alloc_slice(self.arena, Vec::new()),
                source,
                span: self.span_from(start, start_byte),
            });
        }

        self.expect(TokenKind::LBrace)?;
        let mut specs = Vec::new();
        if !self.at(TokenKind::RBrace) {
            loop {
                let (spec_start, spec_start_byte) = self.span_start();
                let imported = self.expect_identifier()?;
                let local = if self.eat(TokenKind::As) {
                    self.expect_identifier()?
                } else {
                    imported
                };
                specs.push(crate::ast::ImportSpec {
                    imported,
                    local,
                    span: self.span_from(spec_start, spec_start_byte),
                });
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RBrace)?;
        self.expect(TokenKind::From)?;

        if !self.at(TokenKind::String) {
            self.error(format!(
                "expected module path string, found `{}`",
                token_name(self.current_kind())
            ));
            return None;
        }
        let source = self.bump_str(self.current_text());
        self.advance();
        self.expect_statement_end(false)?;

        Some(Stmt::Import {
            specifiers: alloc_slice(self.arena, specs),
            source,
            span: self.span_from(start, start_byte),
        })
    }

    fn parse_export_statement(&mut self, start: Pos, start_byte: usize) -> Option<Stmt<'a>> {
        self.advance(); // `export`

        match self.current_kind() {
            TokenKind::Const => {
                self.advance();

                let name = self.expect_identifier()?;
                let ty = if self.eat(TokenKind::Colon) {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.expect(TokenKind::Eq)?;
                let value = self.parse_expression()?;
                self.expect_statement_end(false)?;

                let span = self.span_from(start, start_byte);
                let decl = crate::ast::ExportDecl::Const { name, ty, value };
                Some(Stmt::Export { decl, span })
            }
            TokenKind::Fn => {
                let fn_stmt = self.parse_fn_statement(start, start_byte)?;
                let span = self.span_from(start, start_byte);
                let decl = match fn_stmt {
                    Stmt::Function {
                        name,
                        type_params,
                        params,
                        return_type,
                        body,
                        is_async,
                        ..
                    } => crate::ast::ExportDecl::Function {
                        name,
                        type_params,
                        params,
                        return_type,
                        body,
                        is_async,
                    },
                    Stmt::ReceiverMethod { .. } => {
                        self.error("cannot export a receiver method");
                        return None;
                    }
                    _ => unreachable!(),
                };
                Some(Stmt::Export { decl, span })
            }
            TokenKind::Identifier if self.current_text() == "default" => {
                self.error("unsupported export syntax: default exports are not allowed");
                None
            }
            TokenKind::LBrace => {
                self.advance(); // `{`
                let mut names = Vec::new();
                if !self.at(TokenKind::RBrace) {
                    loop {
                        let (name_start, name_start_byte) = self.span_start();
                        let name = self.expect_identifier()?;
                        let alias = if self.eat(TokenKind::As) {
                            Some(self.expect_identifier()?)
                        } else {
                            None
                        };
                        names.push(crate::ast::ExportName {
                            name,
                            alias,
                            span: self.span_from(name_start, name_start_byte),
                        });
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RBrace)?;
                self.expect_statement_end(false)?;
                Some(Stmt::Export {
                    decl: crate::ast::ExportDecl::NamedGroup {
                        names: alloc_slice(self.arena, names),
                    },
                    span: self.span_from(start, start_byte),
                })
            }
            _ => {
                self.error(format!(
                    "expected `const`, `fn`, `{{` or `default` after `export`, found `{}`",
                    token_name(self.current_kind())
                ));
                None
            }
        }
    }

    pub(super) fn parse_block(&mut self) -> Option<&'a [Stmt<'a>]> {
        self.expect(TokenKind::LBrace)?;
        let mut statements = Vec::new();

        self.skip_newlines();
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
            self.skip_newlines();
        }

        self.expect(TokenKind::RBrace)?;
        Some(alloc_slice(self.arena, statements))
    }

    pub(super) fn parse_params(&mut self) -> Option<&'a [Param<'a>]> {
        let mut params = Vec::new();

        if !self.at(TokenKind::RParen) {
            loop {
                let (param_start, param_start_byte) = self.span_start();
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
                    span: self.span_from(param_start, param_start_byte),
                });

                if !self.eat(TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
            }
        }

        self.skip_newlines();
        Some(alloc_slice(self.arena, params))
    }

    pub(super) fn parse_type_params(&mut self) -> Option<&'a [TypeParam<'a>]> {
        self.expect(TokenKind::Lt)?;
        let mut params = Vec::new();

        loop {
            let name = self.expect_identifier()?;
            let span = self.span_from(self.prev.span.start, self.prev.span.byte_start);
            params.push(TypeParam { name, span });
            if !self.eat(TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
        }

        self.skip_newlines();
        self.expect(TokenKind::Gt)?;
        Some(alloc_slice(self.arena, params))
    }

    fn expect_statement_end(&mut self, in_block: bool) -> Option<()> {
        if self.eat(TokenKind::Semicolon) {
            Some(())
        } else if in_block && self.at(TokenKind::RBrace) {
            // Optional semicolon before a closing brace.
            Some(())
        } else if self.at(TokenKind::Newline) || self.at(TokenKind::Eof) {
            // Optional semicolon: a newline or end-of-file terminates the
            // statement. Consume any following newlines as well.
            self.skip_newlines();
            Some(())
        } else {
            self.error(format!(
                "expected `;` or newline, found `{}`",
                token_name(self.current_kind())
            ));
            None
        }
    }
}
