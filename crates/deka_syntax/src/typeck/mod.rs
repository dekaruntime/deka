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

use std::collections::{HashMap, HashSet};

use bumpalo::Bump;

use crate::ast;
use crate::ast::{MethodTarget, Program};
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
    /// Map from method call expression pointer to the lowering target for the
    /// receiver method that should replace it during lowering.
    pub method_calls: HashMap<*const ast::Expr<'a>, MethodTarget<'a>>,
}

pub fn check_program<'a>(program: &'a Program<'a>, _source: &str) -> TypeckResult<'a> {
    let imports = HashMap::new();
    check_program_with_imports(program, _source, &imports)
}

/// Type information exported by a compiled module, used to seed the
/// typechecker of its importers.
#[derive(Clone, Debug)]
pub struct ModuleExports<'a> {
    pub structs: HashMap<&'a str, StructInfo<'a>>,
    pub enums: HashMap<&'a str, EnumInfo<'a>>,
    pub aliases: HashMap<&'a str, ast::Type<'a>>,
    pub receiver_methods: HashMap<(&'a str, &'a str), MethodInfo<'a>>,
    /// Value bindings (functions / constants) exported by the module.
    /// Currently stored as `Type::Infer` so uses typecheck generically.
    pub values: HashMap<&'a str, Type<'a>>,
}

impl<'a> Default for ModuleExports<'a> {
    fn default() -> Self {
        Self {
            structs: HashMap::new(),
            enums: HashMap::new(),
            aliases: HashMap::new(),
            receiver_methods: HashMap::new(),
            values: HashMap::new(),
        }
    }
}

/// Typecheck a program with imported module signatures available.
pub fn check_program_with_imports<'a>(
    program: &'a Program<'a>,
    _source: &str,
    imports: &HashMap<&str, &ModuleExports<'a>>,
) -> TypeckResult<'a> {
    let mut checker = Checker::new(program, imports);
    checker.check_program();

    TypeckResult {
        program,
        errors: checker.errors,
        warnings: checker.warnings,
        method_calls: checker.method_calls,
    }
}

/// Collect the exported type information from a parsed module.
///
/// The returned `ModuleExports` references AST nodes allocated in `arena` (and
/// in the source strings), so `arena` must outlive any importer that consumes
/// these exports.
pub fn collect_module_exports<'a>(program: &'a Program<'a>, _arena: &'a Bump) -> ModuleExports<'a> {
    let mut declared_structs: HashMap<&'a str, StructInfo<'a>> = HashMap::new();
    let mut declared_enums: HashMap<&'a str, EnumInfo<'a>> = HashMap::new();
    let mut declared_aliases: HashMap<&'a str, ast::Type<'a>> = HashMap::new();
    let mut receiver_methods: HashMap<(&'a str, &'a str), MethodInfo<'a>> = HashMap::new();

    for stmt in program.statements.iter() {
        match stmt {
            ast::Stmt::Struct {
                name, fields, embeds, ..
            } => {
                declared_structs.insert(
                    *name,
                    StructInfo {
                        fields: *fields,
                        embeds: *embeds,
                    },
                );
            }
            ast::Stmt::Enum { name, cases, .. } => {
                declared_enums.insert(*name, EnumInfo { cases: *cases });
            }
            ast::Stmt::TypeAlias { name, value, .. } => {
                declared_aliases.insert(*name, value.clone());
            }
            ast::Stmt::ReceiverMethod {
                receiver_type,
                name,
                params,
                return_type,
                ..
            } => {
                receiver_methods.insert(
                    (*receiver_type, *name),
                    MethodInfo {
                        params: *params,
                        return_type: return_type.clone(),
                    },
                );
            }
            _ => {}
        }
    }

    let mut exports = ModuleExports::default();

    for stmt in program.statements.iter() {
        let ast::Stmt::Export { decl, .. } = stmt else {
            continue;
        };
        match decl {
            ast::ExportDecl::Const { name, .. } => {
                exports.values.insert(*name, Type::Infer);
            }
            ast::ExportDecl::Function { name, .. } => {
                exports.values.insert(*name, Type::Infer);
            }
            ast::ExportDecl::NamedGroup { names } => {
                for export_name in names.iter() {
                    let local = export_name.name;
                    let external = export_name.alias.unwrap_or(local);

                    if let Some(info) = declared_structs.get(local) {
                        exports.structs.insert(external, info.clone());
                        // Promote receiver methods declared on the local
                        // struct to the exported name.
                        for ((rt, mn), mi) in receiver_methods.iter() {
                            if *rt == local {
                                exports
                                    .receiver_methods
                                    .insert((external, *mn), mi.clone());
                            }
                        }
                    }
                    if let Some(info) = declared_enums.get(local) {
                        exports.enums.insert(external, info.clone());
                    }
                    if let Some(ty) = declared_aliases.get(local) {
                        exports.aliases.insert(external, ty.clone());
                    }
                    if let Some(ty) = exports.values.get(local).cloned() {
                        exports.values.insert(external, ty);
                    }
                }
            }
        }
    }

    exports
}

/// Information about an enum's cases, collected before typechecking bodies.
#[derive(Clone, Debug)]
pub struct EnumInfo<'a> {
    pub cases: &'a [ast::EnumCase<'a>],
}

/// Information about a struct's fields and embedded structs, collected before
/// typechecking bodies.
#[derive(Clone, Debug)]
pub struct StructInfo<'a> {
    pub fields: &'a [ast::StructField<'a>],
    pub embeds: &'a [ast::Embed<'a>],
}

/// Information about a receiver method declared on a struct.
#[derive(Clone, Debug)]
pub struct MethodInfo<'a> {
    pub params: &'a [ast::Param<'a>],
    pub return_type: Option<ast::Type<'a>>,
}

/// Information about an interface's declared members.
#[derive(Clone, Debug)]
pub struct InterfaceInfo<'a> {
    pub members: &'a [ast::InterfaceMember<'a>],
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
    /// User-defined interfaces.
    interfaces: HashMap<&'a str, InterfaceInfo<'a>>,
    /// Receiver methods keyed by `(receiver_type, method_name)`.
    receiver_methods: HashMap<(&'a str, &'a str), MethodInfo<'a>>,
    /// Method call sites to lower, keyed by call expression pointer.
    method_calls: HashMap<*const ast::Expr<'a>, MethodTarget<'a>>,
    /// Local scopes. The first scope is the top-level scope.
    scopes: Vec<HashMap<&'a str, Type<'a>>>,
    /// Bindings that were introduced with `let` and may be reassigned.
    mutables: HashSet<&'a str>,
    /// Type parameter scopes. Each generic binding introduces a new scope.
    type_scopes: Vec<HashMap<&'a str, Type<'a>>>,
    /// Are we currently inside a function body?
    in_function: bool,
    /// Are we currently inside an async function body?
    in_async_function: bool,
    /// Expected / inferred return type of the current function.
    return_type: Option<Type<'a>>,
    /// How many nested loops currently enclose the checked statement?
    loop_depth: usize,
}

impl<'a> Checker<'a> {
    fn new(program: &'a ast::Program<'a>, imports: &HashMap<&str, &ModuleExports<'a>>) -> Self {
        let mut this = Self {
            program,
            errors: Vec::new(),
            warnings: Vec::new(),
            globals: HashMap::new(),
            aliases: HashMap::new(),
            enums: HashMap::new(),
            case_to_enum: HashMap::new(),
            structs: HashMap::new(),
            interfaces: HashMap::new(),
            receiver_methods: HashMap::new(),
            method_calls: HashMap::new(),
            scopes: vec![HashMap::new()],
            mutables: HashSet::new(),
            type_scopes: Vec::new(),
            in_function: false,
            in_async_function: false,
            return_type: None,
            loop_depth: 0,
        };
        this.seed_imports(imports);
        this.seed_builtins();
        this
    }

    fn seed_builtins(&mut self) {
        // Host-provided JavaScript globals that the test suite (and v1) rely on.
        // They are typed opaquely as Infer; field/method access on Infer is
        // allowed and returns Infer. Console is intentionally excluded per RFD 32.
        for name in ["Math", "Date", "JSON", "Object", "Promise", "crypto", "parseInt", "process", "isset"] {
            self.globals.insert(name, Type::Infer);
        }
    }

    fn seed_imports(&mut self, imports: &HashMap<&str, &ModuleExports<'a>>) {
        for stmt in self.program.statements.iter() {
            let ast::Stmt::Import { specifiers, source, .. } = stmt else {
                continue;
            };
            let Some(exports) = imports.get(source) else {
                continue;
            };
            for spec in specifiers.iter() {
                let imported = spec.imported;
                let local = spec.local;

                if let Some(info) = exports.structs.get(imported) {
                    self.structs.insert(local, info.clone());
                    for ((rt, mn), mi) in exports.receiver_methods.iter() {
                        if *rt == imported {
                            self.receiver_methods.insert((local, *mn), mi.clone());
                        }
                    }
                }

                if let Some(info) = exports.enums.get(imported) {
                    self.enums.insert(local, info.clone());
                    for case in info.cases.iter() {
                        self.case_to_enum.insert(case.name, local);
                    }
                }

                if let Some(ty) = exports.aliases.get(imported) {
                    self.aliases.insert(local, ty.clone());
                }

                if let Some(ty) = exports.values.get(imported) {
                    self.declare_var(local, ty.clone());
                }
            }
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

    fn declare_mutable_var(&mut self, name: &'a str, ty: Type<'a>) {
        self.scopes.last_mut().unwrap().insert(name, ty);
        self.mutables.insert(name);
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

    fn is_promise(ty: &Type<'_>) -> bool {
        matches!(ty, Type::Generic { base: "Promise", .. })
    }

    fn is_hole_expr(expr: &ast::Expr<'_>) -> bool {
        matches!(expr, ast::Expr::Identifier { name: "_", .. })
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

    #[test]
    fn fn_expression_literal_passes() {
        assert!(typeck("const double = fn (x: number) number { return x * 2 }; const y: number = double(5);").is_empty());
    }

    #[test]
    fn fn_expression_return_type_mismatch_fails() {
        let errors = typeck("const double = fn (x: number) number { return \"oops\" };");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("number"), "{}", errors[0].message);
        assert!(errors[0].message.contains("string"), "{}", errors[0].message);
    }

    #[test]
    fn for_loop_break_continue_passes() {
        assert!(typeck("for (let i = 0; i < 10; i = i + 1) { if (i == 5) { break } else { continue } }").is_empty());
    }

    #[test]
    fn break_outside_loop_fails() {
        let errors = typeck("break;");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("outside of loop"), "{}", errors[0].message);
    }

    #[test]
    fn struct_embed_field_access_passes() {
        assert!(typeck(
            "struct Label { name: string } struct Person { Label } const p: Person = Person { Label: Label { name: \"Ada\" } }; const n: string = p.name;"
        ).is_empty());
    }

    #[test]
    fn struct_embed_method_call_passes() {
        assert!(typeck(
            "struct Legs {} fn (l Legs) move() string { return \"walk\" } struct Robot { Legs } const r: Robot = Robot { Legs: Legs {} }; const m: string = r.move();"
        ).is_empty());
    }

    #[test]
    fn async_function_passes() {
        assert!(typeck("async fn value() Promise<number> { return 1 } const p: Promise<number> = value();").is_empty());
    }

    #[test]
    fn top_level_await_passes() {
        assert!(typeck("async fn main() Promise<number> { return 1 } const n: number = await main();").is_empty());
    }

    #[test]
    fn await_in_sync_function_fails() {
        let errors = typeck("fn f() { await 1 }");
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.message.contains("await")), "{:?}", errors);
    }

    #[test]
    fn unsafe_block_match_result_passes() {
        let errors = typeck(
            "const r = match (unsafe { console.log(1) }) { Ok(v) => v, Err(e) => e };",
        );
        assert!(errors.is_empty(), "{:?}", errors);
    }
}
