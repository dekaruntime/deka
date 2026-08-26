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
use std::fmt;

use crate::ast;
use crate::ast::Program;
use crate::diagnostics::Diagnostic;

#[derive(Debug)]
pub struct TypeError {
    pub message: String,
}

pub struct TypeckResult<'a> {
    pub program: &'a Program<'a>,
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
}

/// Type used internally by the typechecker.
///
/// `Type::None` is the type of the literal `none`.  It is distinct from
/// `Option<T>` but assignable to any `Option<T>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type<'a> {
    /// Sentinel used for error recovery.
    Error,
    /// Placeholder used while a function's return type is being inferred.
    Infer,
    /// The bottom type (`never`). Not produced by the parser, but accepted in
    /// annotations for forward compatibility.
    Never,
    /// The type of the literal `none`.
    None,
    /// A named scalar or user-defined type.
    Named { name: &'a str },
    /// `Option<T>`.
    Option { inner: Box<Type<'a>> },
    /// Function type.
    Function {
        params: Vec<Type<'a>>,
        ret: Box<Type<'a>>,
    },
}

impl<'a> Type<'a> {
    fn is_error(&self) -> bool {
        matches!(self, Type::Error)
    }
}

impl fmt::Display for Type<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Error => write!(f, "<error>"),
            Type::Infer => write!(f, "<infer>"),
            Type::Never => write!(f, "never"),
            Type::None => write!(f, "none"),
            Type::Named { name } => write!(f, "{name}"),
            Type::Option { inner } => write!(f, "Option<{inner}>"),
            Type::Function { params, ret } => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ") => {ret}")
            }
        }
    }
}

pub fn check_program<'a>(program: &'a Program<'a>, _source: &str) -> TypeckResult<'a> {
    let mut checker = Checker::new(program);
    checker.check_program();

    TypeckResult {
        program,
        errors: checker.errors,
        warnings: checker.warnings,
    }
}

struct Checker<'a> {
    program: &'a ast::Program<'a>,
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
    /// Function and (eventually) global variable types.
    globals: HashMap<&'a str, Type<'a>>,
    /// User-defined type aliases without type parameters.
    aliases: HashMap<&'a str, ast::Type<'a>>,
    /// Local scopes. The first scope is the top-level scope.
    scopes: Vec<HashMap<&'a str, Type<'a>>>,
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
            scopes: vec![HashMap::new()],
            in_function: false,
            return_type: None,
        }
    }

    fn error_span(&mut self, span: ast::Span, message: impl Into<String>) {
        self.errors.push(Diagnostic::error(
            span.start.line,
            span.start.column,
            message,
        ));
    }

    fn error_at_expr(&mut self, expr: &ast::Expr<'a>, message: impl Into<String>) {
        self.error_span(expr.span(), message);
    }

    // ------------------------------------------------------------------
    // Program-level passes
    // ------------------------------------------------------------------

    fn check_program(&mut self) {
        self.collect_aliases();
        self.collect_function_signatures();

        // Check function bodies first so that inferred return types are
        // available to later top-level statements.
        for stmt in self.program.statements {
            match stmt {
                ast::Stmt::Function {
                    name,
                    params,
                    return_type,
                    body,
                    span,
                    ..
                } => self.check_function(name, params, return_type.as_ref(), body, *span),
                ast::Stmt::Export {
                    decl: ast::ExportDecl::Function { .. },
                    ..
                } => {
                    // Handled in the same shape as top-level functions below.
                    self.check_export_function(stmt);
                }
                _ => {}
            }
        }

        // Now check non-function top-level statements in source order.
        for stmt in self.program.statements {
            match stmt {
                ast::Stmt::Function { .. } => {}
                ast::Stmt::Export {
                    decl: ast::ExportDecl::Function { .. },
                    ..
                } => {}
                _ => self.check_statement(stmt),
            }
        }
    }

    fn collect_aliases(&mut self) {
        for stmt in self.program.statements {
            if let ast::Stmt::TypeAlias { name, value, span, .. } = stmt {
                if self.aliases.insert(name, value.clone()).is_some() {
                    self.error_span(*span, format!("duplicate type alias `{name}`"));
                }
            }
        }
    }

    fn collect_function_signatures(&mut self) {
        for stmt in self.program.statements {
            let (name, params, return_type) = match stmt {
                ast::Stmt::Function {
                    name,
                    params,
                    return_type,
                    ..
                } => (*name, *params, return_type.as_ref()),
                ast::Stmt::Export {
                    decl:
                        ast::ExportDecl::Function {
                            name,
                            params,
                            return_type,
                            ..
                        },
                    ..
                } => (*name, *params, return_type.as_ref()),
                _ => continue,
            };

            let param_types: Vec<Type<'a>> = params
                .iter()
                .map(|p| match &p.ty {
                    Some(t) => self.resolve_ast_type(t),
                    None => {
                        self.error_span(
                            p.span,
                            format!("parameter `{}` is missing a type annotation", p.name),
                        );
                        Type::Error
                    }
                })
                .collect();

            let ret = match return_type {
                Some(t) => self.resolve_ast_type(t),
                None => Type::Infer,
            };

            self.globals.insert(
                name,
                Type::Function {
                    params: param_types,
                    ret: Box::new(ret),
                },
            );
        }
    }

    // ------------------------------------------------------------------
    // Type resolution
    // ------------------------------------------------------------------

    fn resolve_ast_type(&mut self, ty: &ast::Type<'a>) -> Type<'a> {
        self.resolve_ast_type_rec(ty, &mut HashSet::new())
    }

    fn resolve_ast_type_rec(
        &mut self,
        ty: &ast::Type<'a>,
        seen: &mut HashSet<&'a str>,
    ) -> Type<'a> {
        match ty {
            ast::Type::Named { name, span } => match *name {
                "number" | "string" | "boolean" | "never" => Type::Named { name },
                "Option" => {
                    self.error_span(
                        *span,
                        "Option requires a type argument, e.g. Option<number>",
                    );
                    Type::Error
                }
                _ => {
                    if let Some(alias) = self.aliases.get(name).cloned() {
                        if !seen.insert(name) {
                            self.error_span(*span, format!("cyclic type alias `{name}`"));
                            return Type::Error;
                        }
                        let resolved = self.resolve_ast_type_rec(&alias, seen);
                        seen.remove(name);
                        resolved
                    } else {
                        self.error_span(*span, format!("unknown type `{name}`"));
                        Type::Error
                    }
                }
            },

            ast::Type::Generic { base, args, span } => {
                if base == &"Option" {
                    if args.len() == 1 {
                        Type::Option {
                            inner: Box::new(self.resolve_ast_type_rec(&args[0], seen)),
                        }
                    } else {
                        self.error_span(*span, "Option requires exactly one type argument");
                        Type::Error
                    }
                } else {
                    self.error_span(*span, format!("unsupported generic type `{base}`"));
                    Type::Error
                }
            }

            ast::Type::Function { params, ret, .. } => Type::Function {
                params: params
                    .iter()
                    .map(|p| self.resolve_ast_type_rec(p, seen))
                    .collect(),
                ret: Box::new(self.resolve_ast_type_rec(ret, seen)),
            },

            ast::Type::Option { inner, .. } => Type::Option {
                inner: Box::new(self.resolve_ast_type_rec(inner, seen)),
            },

            ast::Type::Tuple { span, .. } | ast::Type::Record { span, .. } => {
                self.error_span(*span, "tuple/record types are not supported in v2 typeck");
                Type::Error
            }
        }
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    fn check_statement(&mut self, stmt: &ast::Stmt<'a>) {
        match stmt {
            ast::Stmt::Const {
                name,
                ty,
                value,
                span,
            } => {
                self.check_binding(name, ty.as_ref(), value, *span);
            }
            ast::Stmt::Let {
                name,
                ty,
                value,
                span,
            } => {
                self.check_binding(name, ty.as_ref(), value, *span);
            }
            ast::Stmt::Function {
                name,
                params,
                return_type,
                body,
                span,
                ..
            } => {
                self.check_function(name, params, return_type.as_ref(), body, *span);
            }
            ast::Stmt::Export { decl, .. } => match decl {
                ast::ExportDecl::Const {
                    name,
                    ty,
                    value,
                } => {
                    self.check_binding(name, ty.as_ref(), value, value.span());
                }
                ast::ExportDecl::Function { .. } => {
                    self.check_export_function(stmt);
                }
            },
            ast::Stmt::Expr { expr, .. } => {
                self.check_expr(expr);
            }
            ast::Stmt::Return { value, span } => {
                self.check_return(value.as_ref(), *span);
            }
            ast::Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                let cond_type = self.check_expr(condition);
                self.expect_boolean(&cond_type, condition.span());
                for s in then_body.iter() {
                    self.check_statement(s);
                }
                for s in else_body.iter() {
                    self.check_statement(s);
                }
            }
            ast::Stmt::For {
                init,
                condition,
                step,
                body,
                ..
            } => {
                self.scopes.push(HashMap::new());
                if let Some(init) = init {
                    self.check_for_init(init);
                }
                if let Some(cond) = condition {
                    let cond_type = self.check_expr(cond);
                    self.expect_boolean(&cond_type, cond.span());
                }
                if let Some(step) = step {
                    self.check_expr(step);
                }
                for s in body.iter() {
                    self.check_statement(s);
                }
                self.scopes.pop();
            }
            ast::Stmt::TypeAlias { .. } => {
                // Already collected and validated lazily at use sites.
            }
            ast::Stmt::Import { span, .. } => {
                self.error_span(*span, "imports are not supported in v2 typeck");
            }
            ast::Stmt::ReceiverMethod { span, .. }
            | ast::Stmt::Struct { span, .. }
            | ast::Stmt::Enum { span, .. } => {
                self.error_span(*span, "this statement kind is not supported in v2 typeck");
            }
        }
    }

    fn check_export_function(&mut self, stmt: &ast::Stmt<'a>) {
        if let ast::Stmt::Export {
            decl:
                ast::ExportDecl::Function {
                    name,
                    params,
                    return_type,
                    body,
                    ..
                },
            span,
        } = stmt
        {
            self.check_function(name, params, return_type.as_ref(), body, *span);
        }
    }

    fn check_for_init(&mut self, init: &ast::ForInit<'a>) {
        match init {
            ast::ForInit::Const { name, value } => {
                let value_type = self.check_expr(value);
                self.declare_var(name, value_type);
            }
            ast::ForInit::Let { name, value } => {
                let value_type = self.check_expr(value);
                self.declare_var(name, value_type);
            }
            ast::ForInit::Expr(expr) => {
                self.check_expr(expr);
            }
        }
    }

    fn check_binding(
        &mut self,
        name: &'a str,
        ty: Option<&ast::Type<'a>>,
        value: &ast::Expr<'a>,
        _span: ast::Span,
    ) {
        let value_type = self.check_expr(value);
        let final_type = if let Some(annot) = ty {
            let expected = self.resolve_ast_type(annot);
            if !is_assignable(&expected, &value_type) {
                self.error_at_expr(
                    value,
                    format!("expected type `{expected}`, found type `{value_type}`"),
                );
            }
            expected
        } else {
            value_type
        };
        self.declare_var(name, final_type);
    }

    fn check_function(
        &mut self,
        name: &'a str,
        params: &'a [ast::Param<'a>],
        return_type: Option<&ast::Type<'a>>,
        body: &'a [ast::Stmt<'a>],
        _span: ast::Span,
    ) {
        // Use the previously collected signature for parameter types so that
        // errors about missing annotations are reported exactly once.
        let param_types = match self.globals.get(name).cloned() {
            Some(Type::Function { params, ret: _ }) => params,
            _ => {
                let mut pts = Vec::new();
                for p in params {
                    match &p.ty {
                        Some(t) => pts.push(self.resolve_ast_type(t)),
                        None => {
                            self.error_span(
                                p.span,
                                format!("parameter `{}` is missing a type annotation", p.name),
                            );
                            pts.push(Type::Error);
                        }
                    }
                }
                pts
            }
        };

        let explicit_ret = return_type.map(|t| self.resolve_ast_type(t));

        self.scopes.push(HashMap::new());
        for (p, t) in params.iter().zip(param_types.iter()) {
            self.declare_var(p.name, t.clone());
        }

        let saved_in_function = self.in_function;
        let saved_return_type = self.return_type.clone();
        self.in_function = true;
        self.return_type = explicit_ret.clone();

        for stmt in body {
            self.check_statement(stmt);
        }

        let final_ret = self.return_type.take().unwrap_or(Type::None);

        self.in_function = saved_in_function;
        self.return_type = saved_return_type;
        self.scopes.pop();

        // Update the global function type with the final (possibly inferred)
        // return type.
        self.globals.insert(
            name,
            Type::Function {
                params: param_types,
                ret: Box::new(final_ret),
            },
        );
    }

    fn check_return(&mut self, value: Option<&ast::Expr<'a>>, span: ast::Span) {
        if !self.in_function {
            self.error_span(span, "return outside of function");
            return;
        }

        let value_type = match value {
            Some(expr) => self.check_expr(expr),
            None => Type::None,
        };

        if let Some(expected) = self.return_type.as_ref() {
            if !is_assignable(expected, &value_type) {
                self.error_span(
                    span,
                    format!("expected return type `{expected}`, found type `{value_type}`"),
                );
            }
        } else {
            self.return_type = Some(value_type);
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn check_expr(&mut self, expr: &ast::Expr<'a>) -> Type<'a> {
        match expr {
            ast::Expr::Number { .. } => Type::Named { name: "number" },
            ast::Expr::String { .. } => Type::Named { name: "string" },
            ast::Expr::Boolean { value: true, .. } | ast::Expr::Boolean { value: false, .. } => {
                Type::Named { name: "boolean" }
            }
            ast::Expr::None { .. } => Type::None,
            ast::Expr::Identifier { name, span } => {
                self.lookup_var(name).unwrap_or_else(|| {
                    self.error_span(*span, format!("unknown identifier `{name}`"));
                    Type::Error
                })
            }
            ast::Expr::Binary {
                op,
                left,
                right,
                span,
            } => self.check_binary(*op, left, right, *span),
            ast::Expr::Unary {
                op,
                operand,
                span,
            } => self.check_unary(*op, operand, *span),
            ast::Expr::Call {
                callee,
                args,
                span,
                ..
            } => self.check_call(callee, args, *span),
            ast::Expr::FieldAccess { span, .. } => {
                self.error_span(*span, "field access is not yet supported in v2 typeck");
                Type::Error
            }
            ast::Expr::Paren { expr, .. } => self.check_expr(expr),
            _ => {
                self.error_at_expr(expr, "unsupported expression in v2 typeck");
                Type::Error
            }
        }
    }

    fn check_binary(
        &mut self,
        op: ast::BinOp,
        left: &ast::Expr<'a>,
        right: &ast::Expr<'a>,
        span: ast::Span,
    ) -> Type<'a> {
        let left_type = self.check_expr(left);
        let right_type = self.check_expr(right);

        use ast::BinOp::*;
        match op {
            Add => {
                if left_type.is_error() || right_type.is_error() {
                    return Type::Named { name: "number" };
                }
                if Self::is_number(&left_type) && Self::is_number(&right_type) {
                    Type::Named { name: "number" }
                } else if Self::is_string(&left_type) && Self::is_string(&right_type) {
                    Type::Named { name: "string" }
                } else {
                    self.error_span(
                        span,
                        format!(
                            "cannot add types `{left_type}` and `{right_type}`"
                        ),
                    );
                    Type::Error
                }
            }
            Sub | Mul | Div | Mod => {
                self.expect_number(&left_type, left.span());
                self.expect_number(&right_type, right.span());
                Type::Named { name: "number" }
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                if left_type.is_error() || right_type.is_error() {
                    return Type::Named { name: "boolean" };
                }
                if left_type == right_type
                    && (Self::is_number(&left_type)
                        || Self::is_string(&left_type)
                        || Self::is_boolean(&left_type))
                {
                    Type::Named { name: "boolean" }
                } else {
                    self.error_span(
                        span,
                        format!(
                            "cannot compare types `{left_type}` and `{right_type}`"
                        ),
                    );
                    Type::Named { name: "boolean" }
                }
            }
            And | Or => {
                self.expect_boolean(&left_type, left.span());
                self.expect_boolean(&right_type, right.span());
                Type::Named { name: "boolean" }
            }
            _ => {
                self.error_span(span, format!("binary operator `{op:?}` is not supported in v2 typeck"));
                Type::Error
            }
        }
    }

    fn check_unary(
        &mut self,
        op: ast::UnOp,
        operand: &ast::Expr<'a>,
        _span: ast::Span,
    ) -> Type<'a> {
        let operand_type = self.check_expr(operand);
        match op {
            ast::UnOp::Neg => {
                self.expect_number(&operand_type, operand.span());
                Type::Named { name: "number" }
            }
            ast::UnOp::Not => {
                self.expect_boolean(&operand_type, operand.span());
                Type::Named { name: "boolean" }
            }
        }
    }

    fn check_call(
        &mut self,
        callee: &ast::Expr<'a>,
        args: &'a [ast::Expr<'a>],
        span: ast::Span,
    ) -> Type<'a> {
        let callee_type = self.check_expr(callee);

        match callee_type {
            Type::Function { params, ret } => {
                if params.len() != args.len() {
                    self.error_span(
                        span,
                        format!(
                            "expected {} argument{}, found {}",
                            params.len(),
                            if params.len() == 1 { "" } else { "s" },
                            args.len()
                        ),
                    );
                } else {
                    for (expected, arg) in params.iter().zip(args.iter()) {
                        let arg_type = self.check_expr(arg);
                        if !is_assignable(expected, &arg_type) {
                            self.error_at_expr(
                                arg,
                                format!(
                                    "expected argument type `{expected}`, found type `{arg_type}`"
                                ),
                            );
                        }
                    }
                }
                *ret
            }
            Type::Error => Type::Error,
            other => {
                self.error_span(span, format!("value of type `{other}` is not callable"));
                Type::Error
            }
        }
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

/// Assignment / subtyping check. `actual` must be assignable to `expected`.
fn is_assignable<'a>(expected: &Type<'a>, actual: &Type<'a>) -> bool {
    if expected.is_error() || actual.is_error() {
        return true;
    }
    if expected == actual {
        return true;
    }
    // `none` is assignable to any Option<T>.
    if matches!(expected, Type::Option { .. }) && matches!(actual, Type::None) {
        return true;
    }
    false
}

// ===================================================================
// Tests
// ===================================================================

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
            typeck("function add(a: number, b: number): number { return a + b; }").is_empty()
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
            typeck("function add(a: number, b: number): number { return a + b; } add(\"one\", 2);");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("number"), "{}", errors[0].message);
        assert!(errors[0].message.contains("string"), "{}", errors[0].message);
    }

    #[test]
    fn return_wrong_type_fails() {
        let errors = typeck("function f(): number { return \"x\"; }");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("number"), "{}", errors[0].message);
        assert!(errors[0].message.contains("string"), "{}", errors[0].message);
    }
}
