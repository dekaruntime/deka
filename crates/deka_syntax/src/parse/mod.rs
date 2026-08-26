//! DekaScript recursive-descent parser (Compiler v2).

use bumpalo::Bump;

use crate::ast::{Program, Span};
use crate::diagnostics::Diagnostic;
use crate::lexer::{Lexer, Token, TokenKind};

mod expr;
mod pattern;
mod stmt;
mod ty;
mod util;

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
    let mut program = parser.parse_program();
    errors.extend(parser.errors);

    // Resolve syntactic ambiguities (e.g. enum member access) before handing
    // the AST to consumers.
    if let Some(ref mut program) = program {
        crate::canonicalize::resolve_enum_constructors(program, arena);
    }

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
            prev: util::eof_token(),
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
                util::token_name(kind),
                util::token_name(self.current_kind())
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
                util::token_name(self.current_kind())
            ));
            None
        }
    }

    fn span_from(&self, start: crate::ast::Pos) -> Span {
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
            "fn add(a: number, b: number): number { return a + b; }",
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
        let result = parse("const f: fn(number) string = None;", &arena);
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

    #[test]
    fn parse_match_expression() {
        let arena = Bump::new();
        let result = parse("const x = match o { Some(n) => n, None => 0 };", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { value, .. } => match value {
                Expr::Match { arms, .. } => {
                    assert_eq!(arms.len(), 2);
                }
                _ => panic!("expected match expression"),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_enum_constructor() {
        let arena = Bump::new();
        let result = parse("const o = Some(5);", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { value, .. } => match value {
                Expr::EnumConstructor { enum_name, case_name, .. } => {
                    assert_eq!(enum_name.to_string(), "Option");
                    assert_eq!(case_name.to_string(), "Some");
                }
                _ => panic!("expected enum constructor, got {:?}", value),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_struct_declaration_and_literal() {
        let arena = Bump::new();
        let result = parse(
            "struct Point { x: number, y: number } const p = Point { x: 1, y: 2 };",
            &arena,
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        assert_eq!(program.statements.len(), 2);
        match &program.statements[1] {
            Stmt::Const { value, .. } => match value {
                Expr::StructLiteral { name, fields, .. } => {
                    assert_eq!(name.to_string(), "Point");
                    assert_eq!(fields.len(), 2);
                }
                _ => panic!("expected struct literal, got {:?}", value),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_user_defined_enum_constructor() {
        let arena = Bump::new();
        let result = parse(
            "enum Color { Red, Green, Blue } const c = Color.Red;",
            &arena,
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[1] {
            Stmt::Const { value, .. } => match value {
                Expr::EnumConstructor {
                    enum_name,
                    case_name,
                    payload,
                    ..
                } => {
                    assert_eq!(enum_name.to_string(), "Color");
                    assert_eq!(case_name.to_string(), "Red");
                    assert!(payload.is_none());
                }
                _ => panic!("expected enum constructor, got {:?}", value),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_receiver_method() {
        let arena = Bump::new();
        let result = parse(
            "struct Point { x: number, y: number } fn (p Point) distance(other: Point): number { return 0; }",
            &arena,
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[1] {
            Stmt::ReceiverMethod {
                receiver_type,
                name,
                params,
                return_type,
                ..
            } => {
                assert_eq!(receiver_type.to_string(), "Point");
                assert_eq!(name.to_string(), "distance");
                assert_eq!(params.len(), 1);
                assert!(return_type.is_some());
            }
            _ => panic!("expected receiver method"),
        }
    }

    #[test]
    fn parse_generic_function() {
        let arena = Bump::new();
        let result = parse("fn id<T>(x: T): T { return x; }", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Function {
                name,
                type_params,
                params,
                return_type,
                ..
            } => {
                assert_eq!(name.to_string(), "id");
                assert_eq!(type_params.len(), 1);
                assert_eq!(params.len(), 1);
                assert!(return_type.is_some());
            }
            _ => panic!("expected function"),
        }
    }

    #[test]
    fn parse_import_named() {
        let arena = Bump::new();
        let result = parse("import { add } from \"./math.ds\";", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Import { specifiers, source, .. } => {
                assert_eq!(source, &"./math.ds");
                assert_eq!(specifiers.len(), 1);
                assert_eq!(specifiers[0].imported, "add");
                assert_eq!(specifiers[0].local, "add");
            }
            _ => panic!("expected import"),
        }
    }

    #[test]
    fn parse_import_aliased() {
        let arena = Bump::new();
        let result = parse("import { add as plus } from \"./math.ds\";", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Import { specifiers, .. } => {
                assert_eq!(specifiers.len(), 1);
                assert_eq!(specifiers[0].imported, "add");
                assert_eq!(specifiers[0].local, "plus");
            }
            _ => panic!("expected import"),
        }
    }

    #[test]
    fn parse_import_side_effect() {
        let arena = Bump::new();
        let result = parse("import \"./side-effects.ds\";", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Import { specifiers, source, .. } => {
                assert!(specifiers.is_empty());
                assert_eq!(source, &"./side-effects.ds");
            }
            _ => panic!("expected import"),
        }
    }

    #[test]
    fn parse_export_const() {
        let arena = Bump::new();
        let result = parse("export const x: number = 42;", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Export { decl, .. } => match decl {
                crate::ast::ExportDecl::Const { name, ty, .. } => {
                    assert_eq!(name, &"x");
                    assert!(ty.is_some());
                }
                _ => panic!("expected const export"),
            },
            _ => panic!("expected export"),
        }
    }

    #[test]
    fn parse_export_function() {
        let arena = Bump::new();
        let result = parse("export fn add(a: number, b: number): number { return a + b; }", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Export { decl, .. } => match decl {
                crate::ast::ExportDecl::Function { name, params, .. } => {
                    assert_eq!(name, &"add");
                    assert_eq!(params.len(), 2);
                }
                _ => panic!("expected function export"),
            },
            _ => panic!("expected export"),
        }
    }

    #[test]
    fn parse_array_literal() {
        let arena = Bump::new();
        let result = parse("const a = [1, 2, 3];", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { value, .. } => match value {
                Expr::Array { elements, .. } => {
                    assert_eq!(elements.len(), 3);
                }
                _ => panic!("expected array literal"),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_array_spread() {
        let arena = Bump::new();
        let result = parse("const a = [...b];", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { value, .. } => match value {
                Expr::Array { elements, .. } => {
                    assert_eq!(elements.len(), 1);
                    assert!(matches!(elements[0], Expr::Spread { .. }));
                }
                _ => panic!("expected array literal"),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_object_literal() {
        let arena = Bump::new();
        let result = parse("const o = { a: 1, b: \"two\" };", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { value, .. } => match value {
                Expr::Object { fields, .. } => {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(fields[0].key, "a");
                    assert_eq!(fields[1].key, "b");
                }
                _ => panic!("expected object literal"),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_object_spread() {
        let arena = Bump::new();
        let result = parse("const o = { ...base, x: 1 };", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { value, .. } => match value {
                Expr::Object { fields, .. } => {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(fields[0].key, "");
                    assert_eq!(fields[1].key, "x");
                }
                _ => panic!("expected object literal"),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_index_access() {
        let arena = Bump::new();
        let result = parse("const x = arr[0];", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { value, .. } => match value {
                Expr::IndexAccess { object, index, .. } => {
                    assert!(matches!(object, Expr::Identifier { name, .. } if name == &"arr"));
                    assert!(matches!(index, Expr::Number { value, .. } if *value == 0.0));
                }
                _ => panic!("expected index access"),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_await() {
        let arena = Bump::new();
        let result = parse("const x = await fetch();", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { value, .. } => match value {
                Expr::Await { expr, .. } => {
                    assert!(matches!(expr, Expr::Call { .. }));
                }
                _ => panic!("expected await expression"),
            },
            _ => panic!("expected const declaration"),
        }
    }

    #[test]
    fn parse_pipe() {
        let arena = Bump::new();
        let result = parse("const y = x |> double;", &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.unwrap();
        match &program.statements[0] {
            Stmt::Const { value, .. } => match value {
                Expr::Binary { op: BinOp::Pipe, left, right, .. } => {
                    assert!(matches!(left, Expr::Identifier { name, .. } if name == &"x"));
                    assert!(matches!(right, Expr::Identifier { name, .. } if name == &"double"));
                }
                _ => panic!("expected pipe expression, got {:?}", value),
            },
            _ => panic!("expected const declaration"),
        }
    }
}
