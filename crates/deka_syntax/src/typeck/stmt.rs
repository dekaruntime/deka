//! Statement typechecking.

use std::collections::{HashMap, HashSet};

use crate::ast;

use super::types::{is_assignable, Type};
use super::Checker;

impl<'a> Checker<'a> {
    pub(super) fn check_program(&mut self) {
        self.collect_declarations();
        self.collect_function_signatures();
        self.collect_receiver_methods();
        self.check_embedded_method_ambiguity();

        // Check function and receiver-method bodies first so that inferred
        // return types are available to later top-level statements.
        for stmt in self.program.statements {
            match stmt {
                ast::Stmt::Function {
                    name,
                    type_params,
                    params,
                    return_type,
                    body,
                    span,
                    is_async,
                    ..
                } => self.check_function(name, type_params, params, return_type.as_ref(), body, *is_async, *span),
                ast::Stmt::ReceiverMethod {
                    receiver_type,
                    receiver_name,
                    name,
                    params,
                    return_type,
                    body,
                    span,
                    ..
                } => self.check_receiver_method(
                    receiver_type,
                    receiver_name,
                    name,
                    params,
                    return_type.as_ref(),
                    body,
                    *span,
                ),
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
                ast::Stmt::Function { .. } | ast::Stmt::ReceiverMethod { .. } => {}
                ast::Stmt::Export {
                    decl: ast::ExportDecl::Function { .. },
                    ..
                } => {}
                _ => self.check_statement(stmt),
            }
        }
    }

    fn collect_declarations(&mut self) {
        for stmt in self.program.statements {
            if let ast::Stmt::TypeAlias { name, value, span, .. } = stmt {
                if self.aliases.insert(name, value.clone()).is_some() {
                    self.error_span(*span, format!("duplicate type alias `{name}`"));
                }
            }
            if let ast::Stmt::Enum { name, cases, span, .. } = stmt {
                if self.enums.insert(name, super::EnumInfo { cases }).is_some() {
                    self.error_span(*span, format!("duplicate enum definition `{name}`"));
                    continue;
                }
                for case in cases.iter() {
                    if self.case_to_enum.insert(case.name, name).is_some() {
                        self.error_span(
                            case.span,
                            format!("duplicate enum case name `{}`", case.name),
                        );
                    }
                }
            }
            if let ast::Stmt::Struct { name, fields, embeds, span, .. } = stmt {
                if self.structs.insert(name, super::StructInfo { fields, embeds }).is_some() {
                    self.error_span(*span, format!("duplicate struct definition `{name}`"));
                    continue;
                }

                let mut seen_fields = HashSet::new();
                for field in fields.iter() {
                    if !seen_fields.insert(field.name) {
                        self.error_span(
                            field.span,
                            format!(
                                "Duplicate field '{}' in struct '{}'.",
                                field.name, name
                            ),
                        );
                    }
                }

                let mut seen_embeds = HashSet::new();
                for embed in embeds.iter() {
                    if !seen_embeds.insert(embed.name) {
                        self.error_span(
                            embed.span,
                            format!(
                                "Duplicate embedded struct '{}'",
                                embed.name
                            ),
                        );
                    }
                }
            }
        }
    }

    fn push_type_params(&mut self, type_params: &'a [ast::TypeParam<'a>]) {
        if type_params.is_empty() {
            return;
        }
        let mut scope = HashMap::new();
        for param in type_params {
            scope.insert(param.name, Type::Param { name: param.name });
        }
        self.type_scopes.push(scope);
    }

    fn pop_type_params(&mut self) {
        self.type_scopes.pop();
    }

    fn collect_receiver_methods(&mut self) {
        for stmt in self.program.statements {
            if let ast::Stmt::ReceiverMethod {
                receiver_type,
                name,
                params,
                return_type,
                span,
                ..
            } = stmt
            {
                if !self.structs.contains_key(receiver_type) {
                    self.error_span(*span, format!("unknown receiver type `{receiver_type}`"));
                    continue;
                }
                let key = (*receiver_type, *name);
                if self.receiver_methods.insert(key, super::MethodInfo {
                    params,
                    return_type: return_type.clone(),
                }).is_some()
                {
                    self.error_span(*span, format!(
                        "duplicate receiver method `{name}` on type `{receiver_type}`"
                    ));
                }
            }
        }
    }

    fn check_embedded_method_ambiguity(&mut self) {
        // Collect errors first so we don't borrow `self` mutably while iterating
        // over `self.structs`.
        let mut errors: Vec<(ast::Span, String)> = Vec::new();
        for (struct_name, info) in self.structs.iter() {
            if info.embeds.is_empty() {
                continue;
            }

            // Map each method name to the list of embedded structs that promote it.
            let mut promoted: HashMap<&str, Vec<&str>> = HashMap::new();
            for embed in info.embeds.iter() {
                let mut seen = HashSet::new();
                self.collect_promoted_methods(embed.name, &mut promoted, embed.name, &mut seen);
            }

            for (method_name, sources) in promoted.iter() {
                if sources.len() > 1 {
                    errors.push((
                        info.embeds[0].span,
                        format!(
                            "Ambiguous method '{}' on struct '{}'; multiple embedded structs promote it",
                            method_name, struct_name
                        ),
                    ));
                }
            }
        }

        for (span, message) in errors {
            self.error_span(span, message);
        }
    }

    fn collect_promoted_methods(
        &self,
        embed_name: &'a str,
        promoted: &mut HashMap<&'a str, Vec<&'a str>>,
        source: &'a str,
        seen: &mut HashSet<&'a str>,
    ) {
        if !seen.insert(embed_name) {
            return;
        }

        // Direct methods on the embedded struct are promoted.
        for ((rt, method_name), _) in self.receiver_methods.iter() {
            if *rt == embed_name {
                promoted.entry(*method_name).or_default().push(source);
            }
        }

        // Methods promoted by nested embeds are also promoted.
        if let Some(info) = self.structs.get(embed_name) {
            for nested in info.embeds.iter() {
                self.collect_promoted_methods(nested.name, promoted, source, seen);
            }
        }
    }

    fn collect_function_signatures(&mut self) {
        for stmt in self.program.statements {
            let (name, type_params, params, return_type) = match stmt {
                ast::Stmt::Function {
                    name,
                    type_params,
                    params,
                    return_type,
                    ..
                } => (*name, *type_params, *params, return_type.as_ref()),
                ast::Stmt::Export {
                    decl:
                        ast::ExportDecl::Function {
                            name,
                            type_params,
                            params,
                            return_type,
                            ..
                        },
                    ..
                } => (*name, *type_params, *params, return_type.as_ref()),
                _ => continue,
            };

            self.push_type_params(type_params);

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

            self.pop_type_params();

            let optional = params
                .iter()
                .rev()
                .take_while(|p| p.default_value.is_some())
                .count();

            self.globals.insert(
                name,
                Type::Function {
                    params: param_types,
                    ret: Box::new(ret),
                    optional,
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
                self.check_binding(name, ty.as_ref(), value, false, *span);
            }
            ast::Stmt::Let {
                name,
                ty,
                value,
                span,
            } => {
                self.check_binding(name, ty.as_ref(), value, true, *span);
            }
            ast::Stmt::Function {
                name,
                type_params,
                params,
                return_type,
                body,
                span,
                is_async,
                ..
            } => {
                self.check_function(name, type_params, params, return_type.as_ref(), body, *is_async, *span);
            }
            ast::Stmt::Export { decl, .. } => match decl {
                ast::ExportDecl::Const {
                    name,
                    ty,
                    value,
                } => {
                    self.check_binding(name, ty.as_ref(), value, false, value.span());
                }
                ast::ExportDecl::Function { .. } => {
                    self.check_export_function(stmt);
                }
                ast::ExportDecl::NamedGroup { .. } => {
                    // Named re-exports refer to already-checked top-level
                    // declarations; nothing to validate at this scope.
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
                self.scopes.push(HashMap::new());
                for s in then_body.iter() {
                    self.check_statement(s);
                }
                self.scopes.pop();
                self.scopes.push(HashMap::new());
                for s in else_body.iter() {
                    self.check_statement(s);
                }
                self.scopes.pop();
            }
            ast::Stmt::Block { body, .. } => {
                self.scopes.push(HashMap::new());
                for s in body.iter() {
                    self.check_statement(s);
                }
                self.scopes.pop();
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
                self.loop_depth += 1;
                for s in body.iter() {
                    self.check_statement(s);
                }
                self.loop_depth -= 1;
                self.scopes.pop();
            }
            ast::Stmt::ForOf {
                name,
                is_const,
                iterable,
                body,
                ..
            } => {
                self.check_expr(iterable);
                self.scopes.push(HashMap::new());
                self.declare_var(name, Type::Infer);
                if !*is_const {
                    self.mutables.insert(*name);
                }
                self.loop_depth += 1;
                for s in body.iter() {
                    self.check_statement(s);
                }
                self.loop_depth -= 1;
                self.scopes.pop();
            }
            ast::Stmt::Break { span } => {
                if self.loop_depth == 0 {
                    self.error_span(*span, "`break` outside of loop");
                }
            }
            ast::Stmt::Continue { span } => {
                if self.loop_depth == 0 {
                    self.error_span(*span, "`continue` outside of loop");
                }
            }
            ast::Stmt::Struct {
                name,
                type_params,
                fields,
                embeds,
                span: _,
            } => {
                self.push_type_params(type_params);
                for field in *fields {
                    self.resolve_ast_type(&field.ty);
                }
                for embed in *embeds {
                    if !self.structs.contains_key(embed.name) {
                        self.error_span(
                            embed.span,
                            format!("unknown embed type `{}` in struct `{name}`", embed.name),
                        );
                    }
                }
                self.pop_type_params();
            }
            ast::Stmt::Enum { name: _, cases, span: _, .. } => {
                for case in *cases {
                    if let Some(payload) = &case.payload {
                        self.resolve_ast_type(payload);
                    }
                }
            }
            ast::Stmt::Empty { .. }
            | ast::Stmt::TypeAlias { .. }
            | ast::Stmt::ReceiverMethod { .. } => {
                // Already collected and validated lazily at use sites (or no-op).
            }
            ast::Stmt::Import { specifiers, .. } => {
                // Without a resolved module graph, imported bindings are treated
                // as externally provided. They are assigned the infer sentinel
                // so uses of them typecheck generically; a real module resolver
                // will supply concrete types later.
                for spec in specifiers.iter() {
                    self.declare_var(spec.local, Type::Infer);
                }
            }
        }
    }

    fn check_export_function(&mut self, stmt: &ast::Stmt<'a>) {
        if let ast::Stmt::Export {
            decl:
                ast::ExportDecl::Function {
                    name,
                    type_params,
                    params,
                    return_type,
                    body,
                    is_async,
                    ..
                },
            span,
        } = stmt
        {
            self.check_function(name, type_params, params, return_type.as_ref(), body, *is_async, *span);
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
                self.declare_mutable_var(name, value_type);
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
        mutable: bool,
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
        if mutable {
            self.declare_mutable_var(name, final_type);
        } else {
            self.declare_var(name, final_type);
        }
    }

    pub(super) fn check_function(
        &mut self,
        name: &'a str,
        type_params: &'a [ast::TypeParam<'a>],
        params: &'a [ast::Param<'a>],
        return_type: Option<&ast::Type<'a>>,
        body: &'a [ast::Stmt<'a>],
        is_async: bool,
        _span: ast::Span,
    ) {
        // Use the previously collected signature for parameter types so that
        // errors about missing annotations are reported exactly once.
        let (param_types, optional) = match self.globals.get(name).cloned() {
            Some(Type::Function { params, ret: _, optional }) => (params, optional),
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
                let optional = params
                    .iter()
                    .rev()
                    .take_while(|p| p.default_value.is_some())
                    .count();
                (pts, optional)
            }
        };

        self.push_type_params(type_params);

        let explicit_ret = return_type.map(|t| self.resolve_ast_type(t));
        let (body_expected_ret, final_ret) =
            self.function_return_context(is_async, explicit_ret.clone(), _span);

        self.scopes.push(HashMap::new());

        // Make the function available to its own body for recursion. Use the
        // signature collected earlier; the return type will be refined after
        // the body is checked.
        let self_type = Type::Function {
            params: param_types.clone(),
            ret: Box::new(final_ret.clone()),
            optional,
        };
        self.declare_var(name, self_type);

        for (p, t) in params.iter().zip(param_types.iter()) {
            self.declare_var(p.name, t.clone());
        }

        let saved_in_function = self.in_function;
        let saved_in_async = self.in_async_function;
        let saved_return_type = self.return_type.clone();
        self.in_function = true;
        self.in_async_function = is_async;
        self.return_type = body_expected_ret.clone();

        for stmt in body {
            self.check_statement(stmt);
        }

        self.in_async_function = saved_in_async;

        let final_ret = if is_async {
            // The public signature is always the declared Promise type (or a
            // Promise wrapping the inferred payload for unannotated functions).
            match explicit_ret {
                Some(ret) => ret,
                None => self
                    .return_type
                    .take()
                    .map(|inner| Type::Generic {
                        base: "Promise",
                        args: vec![inner],
                    })
                    .unwrap_or(Type::Generic {
                        base: "Promise",
                        args: vec![Type::Generic {
                            base: "Option",
                            args: vec![Type::Never],
                        }],
                    }),
            }
        } else {
            body_expected_ret.unwrap_or_else(|| {
                self.return_type
                    .take()
                    .unwrap_or(Type::Generic {
                        base: "Option",
                        args: vec![Type::Never],
                    })
            })
        };

        self.in_function = saved_in_function;
        self.return_type = saved_return_type;
        self.scopes.pop();

        self.pop_type_params();

        // Update the global function type with the final (possibly inferred)
        // return type.
        self.globals.insert(
            name,
            Type::Function {
                params: param_types,
                ret: Box::new(final_ret),
                optional,
            },
        );
    }

    /// Computes the return type expected from the function body and the
    /// public return type of the function. For async functions the body must
    /// produce the payload type `T`, while the public type is `Promise<T>`.
    pub(super) fn function_return_context(
        &mut self,
        is_async: bool,
        explicit_ret: Option<Type<'a>>,
        span: ast::Span,
    ) -> (Option<Type<'a>>, Type<'a>) {
        if !is_async {
            return (explicit_ret.clone(), explicit_ret.unwrap_or(Type::Infer));
        }

        match explicit_ret {
            Some(Type::Generic { base: "Promise", args }) if args.len() == 1 => {
                let inner = args[0].clone();
                let public = Type::Generic {
                    base: "Promise",
                    args: vec![inner.clone()],
                };
                (Some(inner), public)
            }
            Some(other) => {
                self.error_span(
                    span,
                    format!("async function must return Promise<T>, found type `{other}`"),
                );
                (Some(other.clone()), other)
            }
            None => (
                None,
                Type::Generic {
                    base: "Promise",
                    args: vec![Type::Generic {
                        base: "Option",
                        args: vec![Type::Never],
                    }],
                },
            ),
        }
    }

    pub(super) fn check_receiver_method(
        &mut self,
        receiver_type: &'a str,
        receiver_name: &'a str,
        name: &'a str,
        params: &'a [ast::Param<'a>],
        return_type: Option<&ast::Type<'a>>,
        body: &'a [ast::Stmt<'a>],
        _span: ast::Span,
    ) {
        let info = match self.receiver_methods.get(&(receiver_type, name)) {
            Some(i) => i.clone(),
            None => return,
        };

        let mut param_types = Vec::new();
        for p in params {
            match &p.ty {
                Some(t) => param_types.push(self.resolve_ast_type(t)),
                None => {
                    self.error_span(
                        p.span,
                        format!("parameter `{}` is missing a type annotation", p.name),
                    );
                    param_types.push(Type::Error);
                }
            }
        }

        let explicit_ret = return_type.map(|t| self.resolve_ast_type(t));

        self.scopes.push(HashMap::new());

        // Bind the receiver name to the receiver type inside the method body.
        self.declare_var(receiver_name, Type::Struct { name: receiver_type });

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

        self.in_function = saved_in_function;
        self.return_type = saved_return_type;
        self.scopes.pop();

        // Update the stored signature with resolved types.
        self.receiver_methods.insert(
            (receiver_type, name),
            super::MethodInfo {
                params,
                return_type: info.return_type.clone(),
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
            None => Type::Generic {
                base: "Option",
                args: vec![Type::Never],
            },
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
