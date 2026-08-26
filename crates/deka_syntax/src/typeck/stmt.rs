//! Statement typechecking.

use std::collections::HashMap;

use crate::ast;

use super::types::{is_assignable, Type};
use super::Checker;

impl<'a> Checker<'a> {
    pub(super) fn check_program(&mut self) {
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

    pub(super) fn check_statement(&mut self, stmt: &ast::Stmt<'a>) {
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

    pub(super) fn check_function(
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

        // Make the function available to its own body for recursion. Use the
        // signature collected earlier; the return type will be refined after
        // the body is checked.
        let self_type = Type::Function {
            params: param_types.clone(),
            ret: Box::new(explicit_ret.clone().unwrap_or(Type::Infer)),
        };
        self.declare_var(name, self_type);

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

    pub(super) fn check_return(&mut self, value: Option<&ast::Expr<'a>>, span: ast::Span) {
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
}
