//! Expression typechecking.

use std::collections::{HashMap, HashSet};

use crate::ast;

use super::types::{is_assignable, Type};
use super::Checker;

impl<'a> Checker<'a> {
    pub(super) fn check_expr(&mut self, expr: &ast::Expr<'a>) -> Type<'a> {
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
                type_args,
                args,
                span,
                ..
            } => {
                if let Some(ret) = self.try_check_method_call(expr, callee, args, *span) {
                    ret
                } else {
                    self.check_call(callee, type_args, args, *span)
                }
            }
            ast::Expr::FieldAccess { object, field, span } => {
                self.check_field_access(object, field, *span)
            }
            ast::Expr::StructLiteral {
                name,
                fields,
                span,
            } => self.check_struct_literal(name, fields, *span),
            ast::Expr::Paren { expr, .. } => self.check_expr(expr),
            ast::Expr::Match {
                scrutinee,
                arms,
                span,
            } => self.check_match(scrutinee, arms, *span),
            ast::Expr::EnumConstructor {
                enum_name,
                case_name,
                payload,
                span,
            } => self.check_enum_constructor(enum_name, case_name, payload.as_deref(), *span),
            ast::Expr::Array { elements, .. } => {
                let mut elem_type = None;
                for element in elements.iter() {
                    let ty = self.check_expr(element);
                    if ty.is_error() {
                        return Type::Error;
                    }
                    if elem_type.is_none() {
                        elem_type = Some(ty);
                    }
                }
                Type::Array {
                    elem: Box::new(elem_type.unwrap_or(Type::Infer)),
                }
            }
            ast::Expr::Object { fields, .. } => {
                let mut field_types = Vec::new();
                for field in fields.iter() {
                    let ty = self.check_expr(&field.value);
                    if ty.is_error() {
                        return Type::Error;
                    }
                    field_types.push((field.key, ty));
                }
                Type::Object { fields: field_types }
            }
            ast::Expr::IndexAccess { object, index, .. } => {
                self.check_expr(object);
                self.check_expr(index);
                // TODO: return element type once collection types are modeled.
                Type::Infer
            }
            ast::Expr::Spread { expr, .. } => {
                self.check_expr(expr);
                Type::Infer
            }
            ast::Expr::Await { expr, span } => {
                if self.in_function && !self.in_async_function {
                    self.error_span(
                        *span,
                        "`await` is only allowed inside async functions or at the top level",
                    );
                }
                let operand_type = self.check_expr(expr);
                match operand_type {
                    Type::Generic { base: "Promise", args } if args.len() == 1 => args.into_iter().next().unwrap(),
                    Type::Infer | Type::Error => Type::Infer,
                    other => {
                        self.error_span(*span, format!("`await` expected Promise<T>, found type `{other}`"));
                        Type::Infer
                    }
                }
            }
            ast::Expr::JsxElement { element, .. } => {
                for attr in element.attributes.iter() {
                    if let Some(value) = &attr.value {
                        self.check_expr(value);
                    }
                }
                for child in element.children.iter() {
                    self.check_expr(child);
                }
                Type::Infer
            }
            ast::Expr::JsxFragment { children, .. } => {
                for child in children.iter() {
                    self.check_expr(child);
                }
                Type::Infer
            }
            ast::Expr::JsxText { .. } => Type::Infer,
            ast::Expr::Unsafe { .. } => {
                // Raw JavaScript block. The emitter wraps it as a Result, so
                // the typechecker exposes it as Result<Infer, Infer> so
                // match arms can bind Ok/Err payloads.
                Type::Generic {
                    base: "Result",
                    args: vec![Type::Infer, Type::Infer],
                }
            }
            ast::Expr::Ternary {
                condition,
                then_branch,
                else_branch,
                span,
            } => {
                let cond_type = self.check_expr(condition);
                if !Self::is_boolean(&cond_type) && !cond_type.is_error() && !matches!(cond_type, Type::Infer) {
                    self.error_span(*span, format!("ternary condition must be boolean, found type `{cond_type}`"));
                }
                let then_type = self.check_expr(then_branch);
                let else_type = self.check_expr(else_branch);
                if is_assignable(&then_type, &else_type) {
                    then_type
                } else if is_assignable(&else_type, &then_type) {
                    else_type
                } else {
                    self.error_span(
                        *span,
                        format!("ternary branches have incompatible types `{then_type}` and `{else_type}`"),
                    );
                    Type::Error
                }
            }
            ast::Expr::TemplateLiteral { .. } => Type::Named { name: "string" },
            ast::Expr::Function {
                params,
                return_type,
                body,
                is_async,
                span,
            } => self.check_function_expr(params, return_type.as_ref(), body, *is_async, *span),
            _ => {
                self.error_at_expr(expr, "unsupported expression in v2 typeck");
                Type::Error
            }
        }
    }

    fn check_function_expr(
        &mut self,
        params: &'a [ast::Param<'a>],
        return_type: Option<&ast::Type<'a>>,
        body: &'a [ast::Stmt<'a>],
        is_async: bool,
        span: ast::Span,
    ) -> Type<'a> {
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
        let (body_expected_ret, _final_ret) =
            self.function_return_context(is_async, explicit_ret.clone(), span);

        self.scopes.push(HashMap::new());

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
                        args: vec![Type::None],
                    }),
            }
        } else {
            body_expected_ret.unwrap_or_else(|| self.return_type.take().unwrap_or(Type::None))
        };

        self.in_function = saved_in_function;
        self.return_type = saved_return_type;
        self.scopes.pop();

        let optional = params
            .iter()
            .rev()
            .take_while(|p| p.default_value.is_some())
            .count();

        Type::Function {
            params: param_types,
            ret: Box::new(final_ret),
            optional,
        }
    }

    fn check_struct_literal(
        &mut self,
        name: &'a str,
        fields: &'a [ast::StructLiteralField<'a>],
        span: ast::Span,
    ) -> Type<'a> {
        let info = match self.structs.get(name).cloned() {
            Some(info) => info,
            None => {
                self.error_span(span, format!("unknown struct `{name}`"));
                return Type::Error;
            }
        };

        let fields: Vec<( &'a str, &ast::Expr<'a>, ast::Span)> = fields
            .iter()
            .map(|f| (f.name, &f.value, f.span))
            .collect();
        self.check_struct_literal_fields(name, &info, &fields, span);
        Type::Struct { name }
    }

    fn check_struct_literal_fields(
        &mut self,
        name: &'a str,
        info: &super::StructInfo<'a>,
        fields: &[( &'a str, &ast::Expr<'a>, ast::Span)],
        span: ast::Span,
    ) {
        let embed_names: HashSet<&str> = info.embeds.iter().map(|e| e.name).collect();
        let mut seen_fields = HashSet::new();
        for (field_name, value, field_span) in fields {
            if !seen_fields.insert(*field_name) {
                self.error_span(
                    *field_span,
                    format!("duplicate field `{}` in struct literal", field_name),
                );
            }

            let expected_type = if let Some(f) = info.fields.iter().find(|f| f.name == *field_name) {
                self.resolve_ast_type(&f.ty)
            } else if embed_names.contains(field_name) {
                Type::Struct { name: field_name }
            } else {
                self.error_span(
                    *field_span,
                    format!("struct `{name}` has no field or embed `{}`", field_name),
                );
                Type::Error
            };

            let value_type = self.check_expr(value);
            if !is_assignable(&expected_type, &value_type) {
                self.error_span(
                    *field_span,
                    format!(
                        "field `{}` expected type `{expected_type}`, found type `{value_type}`",
                        field_name
                    ),
                );
            }
        }

        for field in info.fields {
            let is_optional_type = matches!(field.ty, ast::Type::Option { .. });
            if field.default_value.is_none()
                && !field.optional
                && !is_optional_type
                && !seen_fields.contains(field.name)
            {
                self.error_span(
                    span,
                    format!(
                        "missing required field `{}` in struct literal for `{name}`",
                        field.name
                    ),
                );
            }
        }

        for embed in info.embeds {
            if seen_fields.contains(embed.name) {
                continue;
            }
            // Empty embedded structs (no fields and only empty embeds) are
            // auto-filled by the emitter, so they need not be supplied literally.
            if self.is_empty_embed_struct(embed.name) {
                continue;
            }
            self.error_span(
                span,
                format!(
                    "missing embedded struct `{}` in struct literal for `{name}`",
                    embed.name
                ),
            );
        }
    }

    fn is_empty_embed_struct(&self, name: &'a str) -> bool {
        let info = match self.structs.get(name) {
            Some(i) => i,
            None => return false,
        };
        if !info.fields.is_empty() {
            return false;
        }
        info.embeds.iter().all(|e| self.is_empty_embed_struct(e.name))
    }

    fn check_field_access(
        &mut self,
        object: &ast::Expr<'a>,
        field: &'a str,
        span: ast::Span,
    ) -> Type<'a> {
        let object_type = self.check_expr(object);
        if object_type.is_error() {
            return Type::Error;
        }

        match &object_type {
            Type::Infer => {
                // An externally-provided or unresolved value may have any field.
                // Returning Infer preserves the opaque type through the access.
                Type::Infer
            }
            Type::Struct { name } => {
                let struct_name = *name;
                match self.resolve_field_type(struct_name, field) {
                    Some(ty) => ty,
                    None => {
                        self.error_span(
                            span,
                            format!("struct `{struct_name}` has no field `{field}`"),
                        );
                        Type::Error
                    }
                }
            }
            Type::Object { fields } => {
                if let Some((_, ty)) = fields.iter().find(|(name, _)| *name == field) {
                    ty.clone()
                } else {
                    self.error_span(
                        span,
                        format!("object has no field `{field}`"),
                    );
                    Type::Error
                }
            }
            Type::Array { elem } => self.resolve_array_field(field, elem, span),
            Type::Interface { name } => self.resolve_interface_field(name, field).unwrap_or_else(|| {
                self.error_span(span, format!("interface `{name}` has no field `{field}`"));
                Type::Error
            }),
            Type::Named { name } => self.resolve_primitive_field(name, field, span),
            _ => {
                // Enum namespace access: `Color.Red` where `Color` is an enum name.
                if let ast::Expr::Identifier { name: enum_name, .. } = object {
                    if let Some(info) = self.enums.get(enum_name).cloned() {
                        if info.cases.iter().any(|c| c.name == field) {
                            return Type::Named { name: enum_name };
                        }
                        self.error_span(
                            span,
                            format!("case `{field}` not found in enum `{enum_name}`"),
                        );
                        return Type::Error;
                    }
                }
                self.error_span(
                    span,
                    format!("cannot access field `{field}` on type `{object_type}`"),
                );
                Type::Error
            }
        }
    }

    fn resolve_primitive_field(
        &mut self,
        type_name: &'a str,
        field: &'a str,
        span: ast::Span,
    ) -> Type<'a> {
        let string_ty = Type::Named { name: "string" };
        let number_ty = Type::Named { name: "number" };
        let boolean_ty = Type::Named { name: "boolean" };

        let fn0 = |ret: Type<'a>| Type::Function {
            params: Vec::new(),
            ret: Box::new(ret),
            optional: 0,
        };
        let fn1 = |p: Type<'a>, ret: Type<'a>| Type::Function {
            params: vec![p],
            ret: Box::new(ret),
            optional: 0,
        };
        let fn2 = |p1: Type<'a>, p2: Type<'a>, ret: Type<'a>| Type::Function {
            params: vec![p1, p2],
            ret: Box::new(ret),
            optional: 0,
        };

        match type_name {
            "string" => match field {
                "length" => number_ty,
                "toUpperCase" | "toLowerCase" | "trim" => fn0(string_ty.clone()),
                "charAt" | "indexOf" | "lastIndexOf" => fn1(number_ty.clone(), number_ty.clone()),
                "includes" | "startsWith" | "endsWith" => fn1(string_ty.clone(), boolean_ty.clone()),
                "slice" => fn2(number_ty.clone(), number_ty.clone(), string_ty.clone()),
                "split" => fn1(string_ty.clone(), Type::Array { elem: Box::new(string_ty.clone()) }),
                "replace" | "replaceAll" | "concat" => fn2(string_ty.clone(), string_ty.clone(), string_ty.clone()),
                "substring" => fn2(number_ty.clone(), number_ty.clone(), string_ty.clone()),
                _ => {
                    self.error_span(span, format!("string has no field `{field}`"));
                    Type::Error
                }
            },
            "number" | "boolean" => {
                self.error_span(span, format!("{type_name} has no field `{field}`"));
                Type::Error
            }
            _ => {
                // Enum values expose a small reflective surface.
                if self.enums.contains_key(type_name) {
                    return match field {
                        "name" => string_ty,
                        "index" => number_ty,
                        _ => {
                            self.error_span(
                                span,
                                format!("enum `{type_name}` has no field `{field}`"),
                            );
                            Type::Error
                        }
                    };
                }
                self.error_span(
                    span,
                    format!("cannot access field `{field}` on type `{type_name}`"),
                );
                Type::Error
            }
        }
    }

    fn resolve_interface_field(
        &mut self,
        interface_name: &'a str,
        field: &'a str,
    ) -> Option<Type<'a>> {
        let info = self.interfaces.get(interface_name)?;
        for member in info.members.iter() {
            match member {
                ast::InterfaceMember::Field { name, ty, .. } if *name == field => {
                    return Some(self.resolve_ast_type(ty));
                }
                ast::InterfaceMember::Method { name, params, return_type, .. } if *name == field => {
                    let param_types: Vec<Type<'a>> = params
                        .iter()
                        .map(|p| {
                            p.ty.as_ref()
                                .map(|t| self.resolve_ast_type(t))
                                .unwrap_or(Type::Infer)
                        })
                        .collect();
                    let ret = return_type
                        .as_ref()
                        .map(|t| self.resolve_ast_type(t))
                        .unwrap_or(Type::Named { name: "void" });
                    return Some(Type::Function {
                        params: param_types,
                        ret: Box::new(ret),
                        optional: 0,
                    });
                }
                _ => {}
            }
        }
        None
    }

    fn resolve_array_field(
        &mut self,
        field: &'a str,
        elem: &Type<'a>,
        span: ast::Span,
    ) -> Type<'a> {
        let number_ty = Type::Named { name: "number" };
        let boolean_ty = Type::Named { name: "boolean" };
        let array_ty = Type::Array { elem: Box::new(elem.clone()) };

        let fn0 = |ret: Type<'a>| Type::Function {
            params: Vec::new(),
            ret: Box::new(ret),
            optional: 0,
        };
        let fn1 = |p: Type<'a>, ret: Type<'a>| Type::Function {
            params: vec![p],
            ret: Box::new(ret),
            optional: 0,
        };
        let fn2 = |p1: Type<'a>, p2: Type<'a>, ret: Type<'a>| Type::Function {
            params: vec![p1, p2],
            ret: Box::new(ret),
            optional: 0,
        };

        match field {
            "length" => number_ty,
            "includes" => fn2(elem.clone(), number_ty.clone(), boolean_ty.clone()),
            "slice" => fn2(number_ty.clone(), number_ty.clone(), array_ty.clone()),
            "push" => fn1(elem.clone(), number_ty.clone()),
            "pop" => fn0(Type::Option { inner: Box::new(elem.clone()) }),
            "shift" => fn0(Type::Option { inner: Box::new(elem.clone()) }),
            "unshift" => fn1(elem.clone(), number_ty.clone()),
            "concat" => fn1(array_ty.clone(), array_ty.clone()),
            "join" => fn1(Type::Named { name: "string" }, Type::Named { name: "string" }),
            "reverse" | "sort" => fn0(array_ty.clone()),
            "filter" => fn1(
                Type::Function {
                    params: vec![elem.clone()],
                    ret: Box::new(boolean_ty.clone()),
                    optional: 0,
                },
                array_ty.clone(),
            ),
            "map" => Type::Function {
                params: vec![Type::Function {
                    params: vec![elem.clone()],
                    ret: Box::new(Type::Infer),
                    optional: 0,
                }],
                ret: Box::new(Type::Array { elem: Box::new(Type::Infer) }),
                optional: 0,
            },
            "find" => fn1(
                Type::Function {
                    params: vec![elem.clone()],
                    ret: Box::new(boolean_ty.clone()),
                    optional: 0,
                },
                Type::Option { inner: Box::new(elem.clone()) },
            ),
            "forEach" => fn1(
                Type::Function {
                    params: vec![elem.clone()],
                    ret: Box::new(Type::None),
                    optional: 0,
                },
                Type::None,
            ),
            "reduce" => Type::Function {
                params: vec![Type::Function {
                    params: vec![Type::Infer, elem.clone()],
                    ret: Box::new(Type::Infer),
                    optional: 0,
                }],
                ret: Box::new(Type::Infer),
                optional: 0,
            },
            _ => {
                self.error_span(span, format!("array has no field `{field}`"));
                Type::Error
            }
        }
    }

    /// Resolve a field's type, recursively searching embedded structs.
    fn resolve_field_type(&mut self, struct_name: &'a str, field: &'a str) -> Option<Type<'a>> {
        let info = self.structs.get(struct_name)?;
        if let Some(f) = info.fields.iter().find(|f| f.name == field) {
            return Some(self.resolve_ast_type(&f.ty));
        }
        for embed in info.embeds {
            if let Some(ty) = self.resolve_field_type(embed.name, field) {
                return Some(ty);
            }
        }
        None
    }

    fn check_enum_constructor(
        &mut self,
        enum_name: &'a str,
        case_name: &'a str,
        payload: Option<&ast::Expr<'a>>,
        span: ast::Span,
    ) -> Type<'a> {
        let payload_type = payload.map(|expr| self.check_expr(expr));

        if enum_name == "Option" {
            return self.check_option_constructor(case_name, payload, payload_type, span);
        }

        if enum_name == "Result" {
            return self.check_result_constructor(case_name, payload, payload_type, span);
        }

        // User-defined enum.
        let info = match self.enums.get(enum_name) {
            Some(i) => i,
            None => {
                self.error_span(span, format!("unknown enum `{enum_name}`"));
                return Type::Error;
            }
        };

        let case = match info.cases.iter().find(|c| c.name == case_name) {
            Some(c) => c,
            None => {
                self.error_span(
                    span,
                    format!("case `{case_name}` not found in enum `{enum_name}`"),
                );
                return Type::Error;
            }
        };

        match (&case.payload, payload_type) {
            (Some(expected), Some(actual)) => {
                let expected_ty = self.resolve_ast_type(expected);
                if !is_assignable(&expected_ty, &actual) {
                    self.error_span(
                        span,
                        format!(
                            "enum case `{case_name}` expected payload type `{expected_ty}`, found type `{actual}`"
                        ),
                    );
                }
            }
            (Some(_), None) => {
                self.error_span(span, format!("`{case_name}` requires a payload"));
            }
            (None, Some(_)) => {
                self.error_span(span, format!("`{case_name}` cannot have a payload"));
            }
            (None, None) => {}
        }

        Type::Named { name: enum_name }
    }

    fn check_option_constructor(
        &mut self,
        case_name: &'a str,
        payload: Option<&ast::Expr<'a>>,
        payload_type: Option<Type<'a>>,
        span: ast::Span,
    ) -> Type<'a> {
        match case_name {
            "Some" => match payload_type {
                Some(t) => Type::Option { inner: Box::new(t) },
                None => {
                    self.error_span(span, "`Some` requires a payload");
                    Type::Error
                }
            },
            "None" => {
                if payload.is_some() {
                    self.error_span(span, "`None` cannot have a payload");
                }
                // `None` is polymorphic; return Option<Infer> so it can match
                // any Option<T> in the surrounding context.
                Type::Option { inner: Box::new(Type::Infer) }
            }
            _ => {
                self.error_span(span, format!("unknown Option case `{case_name}`"));
                Type::Error
            }
        }
    }

    fn check_result_constructor(
        &mut self,
        case_name: &'a str,
        _payload: Option<&ast::Expr<'a>>,
        payload_type: Option<Type<'a>>,
        span: ast::Span,
    ) -> Type<'a> {
        match case_name {
            "Ok" => match payload_type {
                Some(t) => Type::Generic {
                    base: "Result",
                    args: vec![t, Type::Never],
                },
                None => {
                    self.error_span(span, "`Ok` requires a payload");
                    Type::Error
                }
            },
            "Err" => match payload_type {
                Some(e) => Type::Generic {
                    base: "Result",
                    args: vec![Type::Never, e],
                },
                None => {
                    self.error_span(span, "`Err` requires a payload");
                    Type::Error
                }
            },
            _ => {
                self.error_span(span, format!("unknown Result case `{case_name}`"));
                Type::Error
            }
        }
    }

    fn check_match(
        &mut self,
        scrutinee: &ast::Expr<'a>,
        arms: &'a [ast::MatchArm<'a>],
        span: ast::Span,
    ) -> Type<'a> {
        if arms.is_empty() {
            self.error_span(span, "match expression must have at least one arm");
            return Type::Error;
        }

        let scrutinee_type = self.check_expr(scrutinee);
        let mut result_type: Option<Type<'a>> = None;

        for arm in arms {
            self.scopes.push(HashMap::new());
            self.check_pattern(&arm.pattern, &scrutinee_type);
            let arm_type = self.check_expr(&arm.body);
            self.scopes.pop();

            match &result_type {
                Some(expected) => {
                    if !is_assignable(expected, &arm_type) {
                        self.error_at_expr(
                            &arm.body,
                            format!(
                                "match arm has type `{arm_type}`, expected type `{expected}`"
                            ),
                        );
                    }
                }
                None => result_type = Some(arm_type),
            }
        }

        result_type.unwrap_or(Type::None)
    }

    fn check_pattern(&mut self, pattern: &ast::Pattern<'a>, scrutinee_type: &Type<'a>) {
        match pattern {
            ast::Pattern::Wildcard { .. } => {}
            ast::Pattern::Identifier { name, .. } => {
                self.declare_var(name, scrutinee_type.clone());
            }
            ast::Pattern::Literal { expr, span } => {
                let literal_type = self.check_expr(expr);
                if !is_assignable(scrutinee_type, &literal_type) {
                    self.error_span(
                        *span,
                        format!(
                            "literal pattern has type `{literal_type}`, expected type `{scrutinee_type}`"
                        ),
                    );
                }
            }
            ast::Pattern::Constructor {
                name,
                payload,
                span,
            } => {
                self.check_constructor_pattern(name, payload.as_deref(), *span, scrutinee_type);
            }
            ast::Pattern::Struct { span, .. } | ast::Pattern::Tuple { span, .. } => {
                self.error_span(*span, "struct/tuple patterns are not supported in v2 typeck");
            }
        }
    }

    fn check_constructor_pattern(
        &mut self,
        name: &'a str,
        payload: Option<&ast::Pattern<'a>>,
        span: ast::Span,
        scrutinee_type: &Type<'a>,
    ) {
        // Built-in Option cases.
        if name == "Some" || name == "None" {
            match scrutinee_type {
                Type::Option { inner } => {
                    if name == "None" {
                        if payload.is_some() {
                            self.error_span(span, "`None` pattern cannot have a payload");
                        }
                    } else if let Some(p) = payload {
                        self.check_pattern(p, inner);
                    } else {
                        self.error_span(span, "`Some` pattern requires a payload");
                    }
                    return;
                }
                _ if scrutinee_type.is_error() => return,
                _ => {
                    self.error_span(
                        span,
                        format!(
                            "`{name}` is not a case of type `{scrutinee_type}`"
                        ),
                    );
                    return;
                }
            }
        }

        // Built-in Result cases.
        if name == "Ok" || name == "Err" {
            match scrutinee_type {
                Type::Generic { base: "Result", args } if args.len() == 2 => {
                    let expected_payload = if name == "Ok" { &args[0] } else { &args[1] };
                    if let Some(p) = payload {
                        self.check_pattern(p, expected_payload);
                    } else {
                        self.error_span(span, format!("`{name}` pattern requires a payload"));
                    }
                    return;
                }
                _ if scrutinee_type.is_error() => return,
                _ => {
                    self.error_span(
                        span,
                        format!(
                            "`{name}` is not a case of type `{scrutinee_type}`"
                        ),
                    );
                    return;
                }
            }
        }

        // User-defined enum cases.
        let enum_name = match self.case_to_enum.get(name).copied() {
            Some(n) => n,
            None => {
                self.error_span(span, format!("unknown constructor `{name}`"));
                return;
            }
        };

        let info = match self.enums.get(enum_name) {
            Some(i) => i,
            None => return,
        };

        if !scrutinee_type.is_error() && !matches!(scrutinee_type, Type::Named { name } if *name == enum_name) {
            self.error_span(
                span,
                format!("`{name}` is not a case of type `{scrutinee_type}`"),
            );
            return;
        }

        let case = match info.cases.iter().find(|c| c.name == name) {
            Some(c) => c,
            None => {
                self.error_span(span, format!("case `{name}` not found in enum `{enum_name}`"));
                return;
            }
        };

        if let Some(payload_type) = &case.payload {
            let resolved_payload = self.resolve_ast_type(payload_type);
            if let Some(p) = payload {
                self.check_pattern(p, &resolved_payload);
            } else {
                self.error_span(span, format!("`{name}` pattern requires a payload"));
            }
        } else if payload.is_some() {
            self.error_span(span, format!("`{name}` pattern cannot have a payload"));
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
        // Pipe checks its right-hand side specially (it desugars into a call),
        // so avoid the generic check_expr here.
        let right_type = if op == ast::BinOp::Pipe {
            Type::Infer
        } else {
            self.check_expr(right)
        };

        use ast::BinOp::*;
        match op {
            Add => {
                if left_type.is_error() || right_type.is_error() {
                    return Type::Named { name: "number" };
                }
                if matches!(left_type, Type::Infer) || matches!(right_type, Type::Infer) {
                    return Type::Infer;
                }
                if Self::is_number(&left_type) && Self::is_number(&right_type) {
                    Type::Named { name: "number" }
                } else if Self::is_string(&left_type) || Self::is_string(&right_type) {
                    // String concatenation: JS coerces the other operand to string.
                    Type::Named { name: "string" }
                } else if Self::is_promise(&left_type)
                    || Self::is_promise(&right_type)
                {
                    // Promise<T> + primitive coerces to string in JS.
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
                if !matches!(left_type, Type::Infer) {
                    self.expect_number(&left_type, left.span());
                }
                if !matches!(right_type, Type::Infer) {
                    self.expect_number(&right_type, right.span());
                }
                Type::Named { name: "number" }
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                if left_type.is_error() || right_type.is_error() {
                    return Type::Named { name: "boolean" };
                }
                if matches!(left_type, Type::Infer) || matches!(right_type, Type::Infer) {
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
                if !matches!(left_type, Type::Infer) {
                    self.expect_boolean(&left_type, left.span());
                }
                if !matches!(right_type, Type::Infer) {
                    self.expect_boolean(&right_type, right.span());
                }
                Type::Named { name: "boolean" }
            }
            Pipe => {
                // Pipe desugars at emit time. Type-check the effective call.
                match right {
                    ast::Expr::Identifier { name, span } => {
                        let callee_type = self.lookup_var(name).unwrap_or(Type::Infer);
                        if let Type::Function { params, ret, optional } = callee_type {
                            let required = params.len().saturating_sub(optional);
                            if params.len() < 1 || required > 1 {
                                self.error_span(
                                    *span,
                                    format!(
                                        "pipe right-hand side expects 1 argument, found {} parameters",
                                        params.len()
                                    ),
                                );
                            }
                            if !params.is_empty()
                                && !is_assignable(&params[0], &left_type)
                                && !matches!(left_type, Type::Infer)
                                && !matches!(params[0], Type::Infer)
                            {
                                self.error_span(
                                    *span,
                                    format!(
                                        "pipe expected argument type `{}`, found type `{left_type}`",
                                        params[0]
                                    ),
                                );
                            }
                            *ret
                        } else {
                            Type::Infer
                        }
                    }
                    ast::Expr::Call { callee, type_args, args, span } => {
                        let callee_type = self.check_expr(callee);
                        if let Type::Function { params, ret, optional } = callee_type {
                            let subst = if params.iter().any(|p| contains_param(p)) || contains_param(&ret) {
                                self.infer_substitution(type_args, &params, args)
                            } else {
                                HashMap::new()
                            };
                            let substituted_params: Vec<Type<'a>> = params
                                .iter()
                                .map(|p| substitute_type(p, &subst))
                                .collect();
                            let substituted_ret = substitute_type(&ret, &subst);

                            let has_hole = args.iter().any(|a| Self::is_hole_expr(a));
                            let provided: Vec<&ast::Expr<'a>> = args.iter().collect();
                            let expected_provided: Vec<&Type<'a>> = if has_hole {
                                substituted_params.iter().collect()
                            } else {
                                substituted_params.iter().skip(1).collect()
                            };

                            for (expected, arg) in expected_provided.iter().zip(provided.iter()) {
                                if Self::is_hole_expr(arg) {
                                    continue;
                                }
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

                            if !has_hole {
                                if substituted_params.is_empty() {
                                    self.error_span(*span, "pipe right-hand call takes no arguments");
                                } else if !is_assignable(&substituted_params[0], &left_type)
                                    && !matches!(left_type, Type::Infer)
                                    && !matches!(substituted_params[0], Type::Infer)
                                {
                                    self.error_span(
                                        left.span(),
                                        format!(
                                            "pipe expected argument type `{}`, found type `{left_type}`",
                                            substituted_params[0]
                                        ),
                                    );
                                }
                            }

                            let required = substituted_params.len().saturating_sub(optional);
                            let effective_count = if has_hole { args.len() } else { args.len() + 1 };
                            if effective_count < required || effective_count > substituted_params.len() {
                                self.error_span(
                                    *span,
                                    format!(
                                        "pipe right-hand call expected {} to {} arguments, found {}",
                                        required,
                                        substituted_params.len(),
                                        effective_count
                                    ),
                                );
                            }

                            substituted_ret
                        } else if matches!(callee_type, Type::Infer) {
                            for arg in args.iter() {
                                self.check_expr(arg);
                            }
                            Type::Infer
                        } else {
                            self.error_span(*span, format!("value of type `{callee_type}` is not callable"));
                            Type::Error
                        }
                    }
                    _ => {
                        self.error_span(span, "pipe right-hand side must be a function or call");
                        Type::Error
                    }
                }
            }
            Assign => {
                match left {
                    ast::Expr::Identifier { name, .. } => {
                        if !self.mutables.contains(*name) {
                            self.error_span(
                                left.span(),
                                format!("cannot assign to immutable variable `{name}`"),
                            );
                        }
                    }
                    ast::Expr::IndexAccess { object, .. } => {
                        self.check_expr(object);
                    }
                    ast::Expr::FieldAccess { object, .. } => {
                        self.check_expr(object);
                    }
                    _ => {
                        self.error_span(
                            span,
                            "assignment target must be a mutable local variable, field, or index",
                        );
                        return right_type;
                    }
                }
                if !left_type.is_error()
                    && !right_type.is_error()
                    && !is_assignable(&left_type, &right_type)
                {
                    self.error_span(
                        span,
                        format!("cannot assign type `{right_type}` to `{left_type}`"),
                    );
                }
                right_type
            }
            AddAssign | SubAssign | MulAssign | DivAssign | ModAssign => {
                if let ast::Expr::Identifier { name, .. } = left {
                    if !self.mutables.contains(*name) {
                        self.error_span(
                            left.span(),
                            format!("cannot assign to immutable variable `{name}`"),
                        );
                    }
                } else {
                    self.error_span(
                        span,
                        "compound assignment target must be a mutable local variable",
                    );
                }
                if !matches!(left_type, Type::Infer | Type::Error) {
                    self.expect_number(&left_type, left.span());
                }
                if !matches!(right_type, Type::Infer | Type::Error) {
                    self.expect_number(&right_type, right.span());
                }
                left_type
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
            ast::UnOp::Plus => {
                self.expect_number(&operand_type, operand.span());
                Type::Named { name: "number" }
            }
        }
    }

    fn try_check_method_call(
        &mut self,
        call_expr: &ast::Expr<'a>,
        callee: &ast::Expr<'a>,
        args: &'a [ast::Expr<'a>],
        span: ast::Span,
    ) -> Option<Type<'a>> {
        let (object, method_name) = match callee {
            ast::Expr::FieldAccess { object, field, .. } => (object, *field),
            _ => return None,
        };

        let object_type = self.check_expr(object);
        let receiver_type = match &object_type {
            Type::Struct { name } => *name,
            _ => return None,
        };

        let mut embed_path = Vec::new();
        let info = self.find_receiver_method(receiver_type, method_name, &mut embed_path)?;

        // Record this call site so the emitter can lower it to a mangled call.
        // The owner of the method is the embedded struct (or the receiver itself).
        let owner = embed_path.last().copied().unwrap_or(receiver_type);
        let mangled = format!("{owner}_{method_name}");
        self.method_calls.insert(
            call_expr as *const ast::Expr<'a>,
            ast::MethodTarget { mangled, embed_path },
        );

        let expected_params: Vec<Type<'a>> = info
            .params
            .iter()
            .map(|p| match &p.ty {
                Some(t) => self.resolve_ast_type(t),
                None => Type::Error,
            })
            .collect();

        if expected_params.len() != args.len() {
            self.error_span(
                span,
                format!(
                    "method `{method_name}` on `{receiver_type}` expected {} argument{}, found {}",
                    expected_params.len(),
                    if expected_params.len() == 1 { "" } else { "s" },
                    args.len()
                ),
            );
        } else {
            for (expected, arg) in expected_params.iter().zip(args.iter()) {
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

        info.return_type
            .as_ref()
            .map(|t| self.resolve_ast_type(t))
            .unwrap_or(Type::None)
            .into()
    }

    /// Look up a receiver method on a struct type, recursively searching
    /// embedded structs. On success, returns the method info and the path of
    /// embed names that must be traversed to reach the method's owner.
    fn find_receiver_method(
        &self,
        receiver_type: &'a str,
        method_name: &'a str,
        path: &mut Vec<&'a str>,
    ) -> Option<super::MethodInfo<'a>> {
        if let Some(info) = self.receiver_methods.get(&(receiver_type, method_name)) {
            return Some(info.clone());
        }
        let info = self.structs.get(receiver_type)?;
        for embed in info.embeds {
            path.push(embed.name);
            if let Some(found) = self.find_receiver_method(embed.name, method_name, path) {
                return Some(found);
            }
            path.pop();
        }
        None
    }

    fn check_call(
        &mut self,
        callee: &ast::Expr<'a>,
        type_args: &'a [ast::Type<'a>],
        args: &'a [ast::Expr<'a>],
        span: ast::Span,
    ) -> Type<'a> {
        // Built-in special forms that need a concrete return type.
        if let ast::Expr::Identifier { name: "isset", .. } = callee {
            for arg in args.iter() {
                self.check_expr(arg);
            }
            return Type::Named { name: "boolean" };
        }

        let callee_type = self.check_expr(callee);

        match callee_type {
            Type::Function { params, ret, optional } => {
                // Build a substitution for any type parameters appearing in the
                // function signature. Explicit type args are used when present;
                // otherwise we try to infer from the first argument.
                let subst = if params.iter().any(|p| contains_param(p)) || contains_param(&ret) {
                    self.infer_substitution(type_args, &params, args)
                } else {
                    HashMap::new()
                };

                let substituted_params: Vec<Type<'a>> = params
                    .iter()
                    .map(|p| substitute_type(p, &subst))
                    .collect();
                let substituted_ret = substitute_type(&ret, &subst);

                let hole_positions: Vec<usize> = args
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| Self::is_hole_expr(a))
                    .map(|(i, _)| i)
                    .collect();

                if !hole_positions.is_empty() {
                    // Partial application: `add(1, _)` becomes a function that
                    // takes the hole arguments and forwards them.
                    for (i, (expected, arg)) in substituted_params.iter().zip(args.iter()).enumerate() {
                        if Self::is_hole_expr(arg) {
                            continue;
                        }
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
                    let hole_types: Vec<Type<'a>> = hole_positions
                        .iter()
                        .map(|i| substituted_params[*i].clone())
                        .collect();
                    return Type::Function {
                        params: hole_types,
                        ret: Box::new(substituted_ret),
                        optional: 0,
                    };
                }

                let required = substituted_params.len().saturating_sub(optional);
                if args.len() < required || args.len() > substituted_params.len() {
                    let expected_msg = if optional > 0 {
                        format!("{} to {} arguments", required, substituted_params.len())
                    } else {
                        format!(
                            "{} argument{}",
                            substituted_params.len(),
                            if substituted_params.len() == 1 { "" } else { "s" }
                        )
                    };
                    self.error_span(
                        span,
                        format!("expected {}, found {}", expected_msg, args.len()),
                    );
                } else {
                    for (expected, arg) in substituted_params.iter().zip(args.iter()) {
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
                substituted_ret
            }
            Type::Error => Type::Error,
            Type::Infer => {
                // Imported or otherwise externally-provided binding with no
                // known type. Treat the call as opaque rather than erroring.
                for arg in args.iter() {
                    self.check_expr(arg);
                }
                Type::Infer
            }
            Type::Struct { name } => {
                // Factory-call syntax: Person({ name: "Ada" }) is equivalent to
                // Person { name: "Ada" }.
                if args.len() != 1 {
                    self.error_span(
                        span,
                        format!("struct factory `{name}` expects exactly one argument"),
                    );
                    return Type::Error;
                }
                let info = match self.structs.get(name).cloned() {
                    Some(info) => info,
                    None => {
                        self.error_span(span, format!("unknown struct `{name}`"));
                        return Type::Error;
                    }
                };
                let arg = &args[0];
                match arg {
                    ast::Expr::Object { fields, .. } => {
                        let mapped: Vec<(&'a str, &ast::Expr<'a>, ast::Span)> = fields
                            .iter()
                            .map(|f| (f.key, &f.value, f.span))
                            .collect();
                        self.check_struct_literal_fields(name, &info, &mapped, span);
                    }
                    _ => {
                        self.error_at_expr(
                            arg,
                            format!(
                                "struct factory `{name}` expects an object literal argument"
                            ),
                        );
                    }
                }
                Type::Struct { name }
            }
            other => {
                self.error_span(span, format!("value of type `{other}` is not callable"));
                Type::Error
            }
        }
    }
}

impl<'a> Checker<'a> {
    fn infer_substitution(
        &mut self,
        explicit_type_args: &'a [ast::Type<'a>],
        function_params: &[Type<'a>],
        call_args: &'a [ast::Expr<'a>],
    ) -> HashMap<&'a str, Type<'a>> {
        let param_names: Vec<&'a str> = collect_param_names(function_params);

        if !explicit_type_args.is_empty() {
            let mut subst = HashMap::new();
            if explicit_type_args.len() != param_names.len() {
                // Error reported at call site; return empty substitution.
                return subst;
            }
            for (name, ty) in param_names.iter().zip(explicit_type_args.iter()) {
                subst.insert(*name, self.resolve_ast_type(ty));
            }
            return subst;
        }

        // No explicit type args: infer from arguments.
        let mut subst = HashMap::new();
        for (param_ty, arg) in function_params.iter().zip(call_args.iter()) {
            if let Type::Param { name } = param_ty {
                if subst.contains_key(*name) {
                    continue;
                }
                let arg_type = self.check_expr(arg);
                subst.insert(*name, arg_type);
            }
        }
        subst
    }
}

fn collect_param_names<'a>(tys: &[Type<'a>]) -> Vec<&'a str> {
    let mut names = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for ty in tys {
        collect_param_names_rec(ty, &mut names, &mut seen);
    }
    names
}

fn collect_param_names_rec<'a>(ty: &Type<'a>, names: &mut Vec<&'a str>, seen: &mut std::collections::HashSet<&'a str>) {
    match ty {
        Type::Param { name } => {
            if seen.insert(*name) {
                names.push(*name);
            }
        }
        Type::Option { inner } => collect_param_names_rec(inner, names, seen),
        Type::Function { params, ret, .. } => {
            for p in params {
                collect_param_names_rec(p, names, seen);
            }
            collect_param_names_rec(ret, names, seen);
        }
        Type::Generic { args, .. } => {
            for a in args {
                collect_param_names_rec(a, names, seen);
            }
        }
        _ => {}
    }
}

fn contains_param(ty: &Type<'_>) -> bool {
    match ty {
        Type::Param { .. } => true,
        Type::Option { inner } => contains_param(inner),
        Type::Function { params, ret, .. } => {
            params.iter().any(contains_param) || contains_param(ret)
        }
        Type::Generic { args, .. } => args.iter().any(contains_param),
        _ => false,
    }
}

/// Replace type parameters according to `subst`.
fn substitute_type<'a>(ty: &Type<'a>, subst: &HashMap<&'a str, Type<'a>>) -> Type<'a> {
    match ty {
        Type::Param { name } => subst.get(name).cloned().unwrap_or_else(|| Type::Param { name }),
        Type::Option { inner } => Type::Option {
            inner: Box::new(substitute_type(inner, subst)),
        },
        Type::Function { params, ret, optional } => Type::Function {
            params: params.iter().map(|p| substitute_type(p, subst)).collect(),
            ret: Box::new(substitute_type(ret, subst)),
            optional: *optional,
        },
        Type::Generic { base, args } => Type::Generic {
            base,
            args: args.iter().map(|a| substitute_type(a, subst)).collect(),
        },
        other => other.clone(),
    }
}
