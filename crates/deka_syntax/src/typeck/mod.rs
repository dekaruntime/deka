//! DekaScript typechecker (Compiler v2).
//!
//! This is a baseline typechecker for the parser's supported subset.  It is
//! intentionally simple: structural checks for the built-in scalar types,
//! generic `Option<T>` (with `none` represented by a dedicated `NoneType`),
//! function types, and local/top-level bindings.
//!
//! Design choice for `none`:
//! `none` is given a fresh built-in type `Type::None` (displayed as `none`).
//! `Type::None` is assignable to any `Option<T>` because it is the empty
//! option payload.

use std::collections::HashMap;

use crate::ast;
use crate::ast::Program;
use crate::diagnostics::Diagnostic;

mod ast_type;
mod expr;
mod stmt;
mod types;

pub use types::Type;

#[derive(Debug)]
pub struct TypeError {
    pub message: String,
}

pub struct TypeckResult<'a> {
    pub program: &'a Program<'a>,
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
    /// Map from method call expression pointer to the mangled top-level
    /// function name that should replace it during lowering.
    pub method_calls: HashMap<*const ast::Expr<'a>, String>,
}

pub fn check_program<'a>(program: &'a Program<'a>, _source: &str) -> TypeckResult<'a> {
    let mut checker = Checker::new(program);
    checker.check_program();

    TypeckResult {
        program,
        errors: checker.errors,
        warnings: checker.warnings,
        method_calls: checker.method_calls,
    }
}

/// Information about an enum's cases, collected before typechecking bodies.
struct EnumInfo<'a> {
    cases: &'a [ast::EnumCase<'a>],
}

/// Information about a struct's fields, collected before typechecking bodies.
#[derive(Clone)]
struct StructInfo<'a> {
    fields: &'a [ast::StructField<'a>],
}

/// Information about a receiver method declared on a struct.
#[derive(Clone)]
struct MethodInfo<'a> {
    params: &'a [ast::Param<'a>],
    return_type: Option<ast::Type<'a>>,
}

struct Checker<'a> {
    program: &'a ast::Program<'a>,
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
    /// Function and (eventually) global variable types.
    globals: HashMap<&'a str, Type<'a>>,
    /// User-defined type aliases without type parameters.
    aliases: HashMap<&'a str, ast::Type<'a>>,
    /// User-defined enums.
    enums: HashMap<&'a str, EnumInfo<'a>>,
    /// Map from enum case name back to the enum that defines it.
    case_to_enum: HashMap<&'a str, &'a str>,
    /// User-defined structs.
    structs: HashMap<&'a str, StructInfo<'a>>,
    /// Receiver methods keyed by `(receiver_type, method_name)`.
    receiver_methods: HashMap<(&'a str, &'a str), MethodInfo<'a>>,
    /// Method call sites to lower, keyed by call expression pointer.
    method_calls: HashMap<*const ast::Expr<'a>, String>,
    /// Local scopes. The first scope is the top-level scope.
    scopes: Vec<HashMap<&'a str, Type<'a>>>,
    /// Type parameter scopes. Each generic binding introduces a new scope.
    type_scopes: Vec<HashMap<&'a str, Type<'a>>>,
    /// Are we currently inside a function body?
    in_function: bool,
    /// Expected / inferred return type of the current function.
    return_type: Option<Type<'a>>,
}

impl<'a> Checker<'a> {
    fn new(program: &'a ast::Program<'a>) -> Self {
        Self {
            program,
            errors: Vec::new(),
            warnings: Vec::new(),
            globals: HashMap::new(),
            aliases: HashMap::new(),
            enums: HashMap::new(),
            case_to_enum: HashMap::new(),
            structs: HashMap::new(),
            receiver_methods: HashMap::new(),
            method_calls: HashMap::new(),
            scopes: vec![HashMap::new()],
            type_scopes: Vec::new(),
            in_function: false,
            return_type: None,
        }
    }

    fn error_at_expr(&mut self, expr: &ast::Expr<'a>, message: impl Into<String>) {
        self.error_span(expr.span(), message);
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn declare_var(&mut self, name: &'a str, ty: Type<'a>) {
        self.scopes.last_mut().unwrap().insert(name, ty);
    }

    fn lookup_var(&self, name: &'a str) -> Option<Type<'a>> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        self.globals.get(name).cloned()
    }

    fn is_number(ty: &Type<'_>) -> bool {
        matches!(ty, Type::Named { name: "number" })
    }

    fn is_string(ty: &Type<'_>) -> bool {
        matches!(ty, Type::Named { name: "string" })
    }

    fn is_boolean(ty: &Type<'_>) -> bool {
        matches!(ty, Type::Named { name: "boolean" })
    }

    fn expect_number(&mut self, ty: &Type<'a>, span: ast::Span) {
        if !ty.is_error() && !Self::is_number(ty) {
            self.error_span(span, format!("expected type `number`, found type `{ty}`"));
        }
    }

    fn expect_boolean(&mut self, ty: &Type<'a>, span: ast::Span) {
        if !ty.is_error() && !Self::is_boolean(ty) {
            self.error_span(span, format!("expected type `boolean`, found type `{ty}`"));
        }
    }
}

#[cfg(test)]
mod tests {
    use bumpalo::Bump;

    use super::*;
    use crate::parse::parse;

    fn typeck(source: &str) -> Vec<Diagnostic> {
        let arena = Bump::new();
        let result = parse(source, &arena);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let program = result.program.expect("parse produced no program");
        check_program(&program, source).errors
    }

    #[test]
    fn const_number_passes() {
        assert!(typeck("const x: number = 42;").is_empty());
    }

    #[test]
    fn function_add_passes() {
        assert!(
            typeck("fn add(a: number, b: number): number { return a + b; }").is_empty()
        );
    }

    #[test]
    fn const_string_mismatch_fails() {
        let errors = typeck("const x: string = 42;");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("string"), "{}", errors[0].message);
        assert!(errors[0].message.contains("number"), "{}", errors[0].message);
    }

    #[test]
    fn call_wrong_arg_type_fails() {
        let errors =
            typeck("fn add(a: number, b: number): number { return a + b; } add(\"one\", 2);");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("number"), "{}", errors[0].message);
        assert!(errors[0].message.contains("string"), "{}", errors[0].message);
    }

    #[test]
    fn return_wrong_type_fails() {
        let errors = typeck("fn f(): number { return \"x\"; }");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("number"), "{}", errors[0].message);
        assert!(errors[0].message.contains("string"), "{}", errors[0].message);
    }

    #[test]
    fn recursive_function_passes() {
        assert!(typeck(
            "fn forever(n: number): number { return forever(n); }"
        )
        .is_empty());
    }

    #[test]
    fn match_option_number_passes() {
        assert!(typeck("const o = Some(5); const x: number = match o { Some(n) => n, None => 0 };").is_empty());
    }

    #[test]
    fn match_arm_type_mismatch_fails() {
        let errors = typeck("const o = Some(5); const x: number = match o { Some(n) => n, None => \"oops\" };");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("number"), "{}", errors[0].message);
        assert!(errors[0].message.contains("string"), "{}", errors[0].message);
    }

    #[test]
    fn option_some_constructor_passes() {
        assert!(typeck("const o: Option<number> = Some(5);").is_empty());
    }

    #[test]
    fn result_ok_constructor_passes() {
        assert!(typeck("const r: Result<number, string> = Ok(5);").is_empty());
    }

    #[test]
    fn struct_literal_and_field_access_passes() {
        assert!(typeck("struct Point { x: number, y: number } const p: Point = Point { x: 1, y: 2 }; const x: number = p.x;").is_empty());
    }

    #[test]
    fn user_defined_enum_constructor_passes() {
        assert!(typeck("enum Color { Red, Green, Blue } const c: Color = Color.Red;").is_empty());
    }

    #[test]
    fn user_defined_enum_payload_constructor_passes() {
        assert!(typeck("enum Shape { Circle(number), Label(string) } const s: Shape = Shape.Circle(5); const t: Shape = Shape.Label(\"hello\");").is_empty());
    }

    #[test]
    fn user_defined_enum_wrong_payload_type_fails() {
        let errors = typeck("enum Shape { Circle(number) } const s: Shape = Shape.Circle(\"oops\");");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("number"), "{}", errors[0].message);
        assert!(errors[0].message.contains("string"), "{}", errors[0].message);
    }

    #[test]
    fn user_defined_enum_match_passes() {
        assert!(typeck("enum Color { Red, Green, Blue } const c: Color = Color.Red; const x: number = match c { Red => 1, Green => 2, Blue => 3 };").is_empty());
    }

    #[test]
    fn receiver_method_passes() {
        assert!(typeck(
            "struct Point { x: number, y: number } fn (p Point) distance(other: Point): number { return 0; } const p1: Point = Point { x: 0, y: 0 }; const p2: Point = Point { x: 3, y: 4 }; const d: number = p1.distance(p2);"
        ).is_empty());
    }

    #[test]
    fn generic_function_inferred_passes() {
        assert!(typeck("fn id<T>(x: T): T { return x; } const n: number = id(5); const s: string = id(\"hi\");").is_empty());
    }

    #[test]
    fn generic_function_explicit_type_args_passes() {
        assert!(typeck("fn id<T>(x: T): T { return x; } const n: number = id<number>(5);").is_empty());
    }

    #[test]
    fn generic_function_wrong_arg_type_fails() {
        let errors = typeck("fn id<T>(x: T): T { return x; } const n: number = id(\"hi\");");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("number"), "{}", errors[0].message);
        assert!(errors[0].message.contains("string"), "{}", errors[0].message);
    }
}
