//! DekaScript recursive-descent parser (Compiler v2).

use bumpalo::Bump;

use crate::ast::{
    alloc, alloc_slice, BinOp, Expr, Param, Pos, Program, Span, Stmt, Type, TypeParam, UnOp,
};
use crate::diagnostics::Diagnostic;
use crate::lexer::{Lexer, Token, TokenKind};

pub struct ParseResult<'a> {
    pub program: Option<Program<'a>>,
    pub errors: Vec<Diagnostic>,
}

/// Parse a full `.ds` source file into a DekaScript AST.
pub fn parse<'a>(source: &'a str, arena: &'a Bump) -> ParseResult<'a> {
    let mut lexer = Lexer::new(source);
    let mut tokens: Vec<Token> = Vec::new();

    loop {
        let tok = lexer.next_token();
        // Comments and newlines are treated as whitespace for this subset.
        if tok.kind == TokenKind::Comment || tok.kind == TokenKind::Newline {
            continue;
        }
        let is_eof = tok.kind == TokenKind::Eof;
        tokens.push(tok);
        if is_eof {
            break;
        }
    }

    let mut errors: Vec<Diagnostic> = lexer.diagnostics().to_vec();
    let mut parser = Parser::new(arena, tokens);
    let program = parser.parse_program();
    errors.extend(parser.errors);

    ParseResult {
        program: if errors.is_empty() { program } else { None },
        errors,
    }
}

struct Parser<'a> {
    arena: &'a Bump,
    tokens: Vec<Token<'a>>,
    pos: usize,
    prev: Token<'a>,
    errors: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn new(arena: &'a Bump, tokens: Vec<Token<'a>>) -> Self {
        Self {
            arena,
            pos: 0,
            prev: eof_token(),
            errors: Vec::new(),
            tokens,
        }
    }

    // ------------------------------------------------------------------
    // Token helpers
    // ------------------------------------------------------------------

    fn current(&self) -> &Token<'a> {
        &self.tokens[self.pos]
    }

    fn current_kind(&self) -> TokenKind {
        self.current().kind
    }

    fn current_span(&self) -> Span {
        self.current().span
    }

    fn current_text(&self) -> &'a str {
        self.current().text
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.current_kind() == kind
    }

    fn at_end(&self) -> bool {
        self.at(TokenKind::Eof)
    }

    fn advance(&mut self) {
        self.prev = self.current().clone();
        if !self.at_end() {
            self.pos += 1;
        }
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Option<()> {
        if self.eat(kind) {
            Some(())
        } else {
            self.error(format!(
                "expected `{}`, found `{}`",
                token_name(kind),
                token_name(self.current_kind())
            ));
            None
        }
    }

    fn expect_identifier(&mut self) -> Option<&'a str> {
        if self.at(TokenKind::Identifier) {
            let name = self.bump_str(self.current_text());
            self.advance();
            Some(name)
        } else {
            self.error(format!(
                "expected identifier, found `{}`",
                token_name(self.current_kind())
            ));
            None
        }
    }

    fn span_from(&self, start: Pos) -> Span {
        Span {
            start,
            end: self.prev.span.end,
        }
    }

    fn bump_str(&self, s: &str) -> &'a str {
        self.arena.alloc_str(s)
    }

    fn error(&mut self, message: impl Into<String>) {
        let pos = self.current_span().start;
        self.errors
            .push(Diagnostic::error(pos.line, pos.column, message));
    }

    fn synchronize(&mut self) {
        while !self.at_end() && !self.at(TokenKind::Semicolon) && !self.at(TokenKind::RBrace) {
            self.advance();
        }
        if self.at(TokenKind::Semicolon) {
            self.advance();
        }
    }

    // ------------------------------------------------------------------
    // Program / statements
    // ------------------------------------------------------------------

    fn parse_program(&mut self) -> Option<Program<'a>> {
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

    fn parse_statement(&mut self, in_block: bool) -> Option<Stmt<'a>> {
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

    fn parse_block(&mut self) -> Option<&'a [Stmt<'a>]> {
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

    fn parse_params(&mut self) -> Option<&'a [Param<'a>]> {
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

    fn parse_type_params(&mut self) -> Option<&'a [TypeParam<'a>]> {
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

    // ------------------------------------------------------------------
    // Types
    // ------------------------------------------------------------------

    fn parse_type(&mut self) -> Option<Type<'a>> {
        let start = self.current_span().start;
        let mut ty = self.parse_type_primary()?;

        while self.eat(TokenKind::Question) {
            let span = self.span_from(start);
            ty = Type::Option {
                inner: alloc(self.arena, ty),
                span,
            };
        }

        Some(ty)
    }

    fn parse_type_primary(&mut self) -> Option<Type<'a>> {
        let start = self.current_span().start;

        if self.eat(TokenKind::LParen) {
            // Either a function type `(T, U) => R` or a grouped type `(T)`.
            let mut params = Vec::new();
            if !self.at(TokenKind::RParen) {
                loop {
                    params.push(self.parse_type()?);
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(TokenKind::RParen)?;

            if self.eat(TokenKind::FatArrow) {
                let ret = self.parse_type()?;
                Some(Type::Function {
                    params: alloc_slice(self.arena, params),
                    ret: alloc(self.arena, ret),
                    span: self.span_from(start),
                })
            } else if params.len() == 1 {
                Some(params.into_iter().next().unwrap())
            } else {
                self.error("expected function arrow `=>` or a single grouped type".to_string());
                None
            }
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
                }
                self.expect(TokenKind::Gt)?;
                Some(Type::Generic {
                    base: name,
                    args: alloc_slice(self.arena, args),
                    span: self.span_from(start),
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

    // ------------------------------------------------------------------
    // Expressions (Pratt parser)
    // ------------------------------------------------------------------

    fn parse_expression(&mut self) -> Option<Expr<'a>> {
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
                left = Expr::Call {
                    callee: alloc(self.arena, left),
                    type_args: &[],
                    args: alloc_slice(self.arena, args),
                    span,
                };
                continue;
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

// ------------------------------------------------------------------
// Infix handling
// ------------------------------------------------------------------

fn infix_info(kind: TokenKind) -> Option<(u8, u8, BinOp)> {
    use TokenKind::*;
    Some(match kind {
        Or => (1, 2, BinOp::Or),
        And => (3, 4, BinOp::And),
        EqEq => (5, 6, BinOp::Eq),
        NotEq => (5, 6, BinOp::Ne),
        Lt => (7, 8, BinOp::Lt),
        Le => (7, 8, BinOp::Le),
        Gt => (7, 8, BinOp::Gt),
        Ge => (7, 8, BinOp::Ge),
        Plus => (9, 10, BinOp::Add),
        Minus => (9, 10, BinOp::Sub),
        Star => (11, 12, BinOp::Mul),
        Slash => (11, 12, BinOp::Div),
        Percent => (11, 12, BinOp::Mod),
        _ => return std::option::Option::None,
    })
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

fn eof_token<'a>() -> Token<'a> {
    Token {
        kind: TokenKind::Eof,
        text: "",
        span: Span::dummy(),
    }
}

fn token_name(kind: TokenKind) -> &'static str {
    use TokenKind::*;
    match kind {
        Number => "number",
        BigInt => "bigint",
        String => "string",
        True => "`true`",
        False => "`false`",
        None => "`none`",
        Identifier => "identifier",
        Const => "`const`",
        Let => "`let`",
        Mut => "`mut`",
        Function => "`function`",
        Fn => "`fn`",
        Struct => "`struct`",
        Enum => "`enum`",
        Type => "`type`",
        Import => "`import`",
        Export => "`export`",
        From => "`from`",
        As => "`as`",
        If => "`if`",
        Else => "`else`",
        For => "`for`",
        Return => "`return`",
        Match => "`match`",
        Unsafe => "`unsafe`",
        Await => "`await`",
        Async => "`async`",
        Pub => "`pub`",
        Plus => "`+`",
        Minus => "`-`",
        Star => "`*`",
        Slash => "`/`",
        Percent => "`%`",
        Eq => "`=`",
        EqEq => "`==`",
        NotEq => "`!=`",
        Lt => "`<`",
        Le => "`<=`",
        Gt => "`>`",
        Ge => "`>=`",
        And => "`&&`",
        Or => "`||`",
        Not => "`!`",
        Ampersand => "`&`",
        Pipe => "`|`",
        Caret => "`^`",
        Shl => "`<<`",
        Shr => "`>>`",
        LParen => "`(`",
        RParen => "`)`",
        LBrace => "`{`",
        RBrace => "`}`",
        LBracket => "`[`",
        RBracket => "`]`",
        Comma => "`,`",
        Semicolon => "`;`",
        Colon => "`:`",
        DoubleColon => "`::`",
        Dot => "`.`",
        Arrow => "`->`",
        FatArrow => "`=>`",
        Question => "`?`",
        Spread => "`...`",
        LtJsx => "`<`",
        GtJsx => "`>`",
        SlashJsx => "`/`",
        Newline => "newline",
        Comment => "comment",
        Eof => "end of file",
        Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinOp, Expr, Stmt, Type, UnOp};

    #[test]
    fn parse_const_number() {
        let arena = Bump::new();
        let result = parse("const x = 42;", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        assert_eq!(program.statements.len(), 1);

        match &program.statements[0] {
            Stmt::Const {
                name, ty, value, ..
            } => {
                assert_eq!(name.to_string(), "x");
                assert!(ty.is_none());
                match value {
                    Expr::Number { value, .. } => assert_eq!(*value, 42.0),
                    _ => panic!("expected number literal"),
                }
            }
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_function_add() {
        let arena = Bump::new();
        let result = parse(
            "function add(a: number, b: number): number { return a + b; }",
            &arena,
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        assert_eq!(program.statements.len(), 1);

        match &program.statements[0] {
            Stmt::Function {
                name,
                params,
                return_type,
                body,
                ..
            } => {
                assert_eq!(name.to_string(), "add");
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].name.to_string(), "a");
                assert_eq!(params[1].name.to_string(), "b");
                assert!(matches!(
                    &params[0].ty,
                    Some(Type::Named { name, .. }) if name.to_string() == "number"
                ));
                assert!(matches!(
                    &params[1].ty,
                    Some(Type::Named { name, .. }) if name.to_string() == "number"
                ));
                assert!(matches!(
                    return_type,
                    Some(Type::Named { name, .. }) if name.to_string() == "number"
                ));
                assert_eq!(body.len(), 1);

                match &body[0] {
                    Stmt::Return {
                        value:
                            Some(Expr::Binary {
                                op: BinOp::Add,
                                left,
                                right,
                                ..
                            }),
                        ..
                    } => {
                        match *left {
                            Expr::Identifier { name, .. } => assert_eq!(name.to_string(), "a"),
                            _ => panic!("expected identifier `a`"),
                        }
                        match *right {
                            Expr::Identifier { name, .. } => assert_eq!(name.to_string(), "b"),
                            _ => panic!("expected identifier `b`"),
                        }
                    }
                    _ => panic!("expected return a + b"),
                }
            }
            _ => panic!("expected function declaration"),
        }
    }

    #[test]
    fn parse_console_log_call() {
        let arena = Bump::new();
        let result = parse("console.log(\"hello\");", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        assert_eq!(program.statements.len(), 1);

        match &program.statements[0] {
            Stmt::Expr { expr, .. } => match expr {
                Expr::Call {
                    callee,
                    type_args,
                    args,
                    ..
                } => {
                    assert!(type_args.is_empty());
                    assert_eq!(args.len(), 1);
                    match &args[0] {
                        Expr::String { value, .. } => assert_eq!(value.to_string(), "hello"),
                        _ => panic!("expected string argument"),
                    }
                    match callee {
                        Expr::FieldAccess { object, field, .. } => {
                            match object {
                                Expr::Identifier { name, .. } => {
                                    assert_eq!(name.to_string(), "console")
                                }
                                _ => panic!("expected identifier `console`"),
                            }
                            assert_eq!(field.to_string(), "log");
                        }
                        _ => panic!("expected field access callee"),
                    }
                }
                _ => panic!("expected call expression"),
            },
            _ => panic!("expected expression statement"),
        }
    }

    #[test]
    fn parse_error_missing_expression() {
        let arena = Bump::new();
        let result = parse("const x = ;", &arena);
        assert!(result.program.is_none());
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn parse_let_with_type_and_optional() {
        let arena = Bump::new();
        let result = parse("let y: string? = \"hi\";", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Let {
                name, ty, value, ..
            } => {
                assert_eq!(name.to_string(), "y");
                match ty {
                    Some(Type::Option { inner, .. }) => match inner {
                        Type::Named { name, .. } => assert_eq!(name.to_string(), "string"),
                        _ => panic!("expected string"),
                    },
                    _ => panic!("expected optional type"),
                }
                match value {
                    Expr::String { value, .. } => assert_eq!(value.to_string(), "hi"),
                    _ => panic!("expected string literal"),
                }
            }
            _ => panic!("expected let declaration"),
        }
    }

    #[test]
    fn parse_function_type() {
        let arena = Bump::new();
        let result = parse("const f: (number) => string = none;", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { ty, .. } => match ty {
                Some(Type::Function { params, ret, .. }) => {
                    assert_eq!(params.len(), 1);
                    match &params[0] {
                        Type::Named { name, .. } => assert_eq!(name.to_string(), "number"),
                        _ => panic!("expected number param type"),
                    }
                    match ret {
                        Type::Named { name, .. } => assert_eq!(name.to_string(), "string"),
                        _ => panic!("expected string return type"),
                    }
                }
                _ => panic!("expected function type"),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_unary_and_binary_precedence() {
        let arena = Bump::new();
        let result = parse("const z = -a.b + c * d;", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { value, .. } => match value {
                Expr::Binary {
                    op: BinOp::Add,
                    left,
                    right,
                    ..
                } => {
                    // left: -(a.b)
                    match *left {
                        Expr::Unary {
                            op: UnOp::Neg,
                            operand,
                            ..
                        } => match operand {
                            Expr::FieldAccess { object, field, .. } => {
                                match object {
                                    Expr::Identifier { name, .. } => {
                                        assert_eq!(name.to_string(), "a")
                                    }
                                    _ => panic!("expected identifier `a`"),
                                }
                                assert_eq!(field.to_string(), "b");
                            }
                            _ => panic!("expected field access inside unary"),
                        },
                        _ => panic!("expected unary on left"),
                    }
                    // right: c * d
                    match *right {
                        Expr::Binary {
                            op: BinOp::Mul,
                            left,
                            right,
                            ..
                        } => {
                            match left {
                                Expr::Identifier { name, .. } => {
                                    assert_eq!(name.to_string(), "c")
                                }
                                _ => panic!("expected identifier `c`"),
                            }
                            match right {
                                Expr::Identifier { name, .. } => {
                                    assert_eq!(name.to_string(), "d")
                                }
                                _ => panic!("expected identifier `d`"),
                            }
                        }
                        _ => panic!("expected multiplication on right"),
                    }
                }
                _ => panic!("expected binary add"),
            },
            _ => panic!("expected const declaration"),
        }
    }
}
