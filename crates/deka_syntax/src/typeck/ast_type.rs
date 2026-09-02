//! AST type -> typechecker type resolution.

use std::collections::HashSet;

use crate::ast;
use crate::diagnostics::Diagnostic;

use super::types::Type;
use super::Checker;

impl<'a> Checker<'a> {
    pub(super) fn resolve_ast_type(&mut self, ty: &ast::Type<'a>) -> Type<'a> {
        self.resolve_ast_type_rec(ty, &mut HashSet::new())
    }

    fn resolve_ast_type_rec(
        &mut self,
        ty: &ast::Type<'a>,
        seen: &mut HashSet<&'a str>,
    ) -> Type<'a> {
        match ty {
            ast::Type::Named { name, span } => match *name {
                "number" | "string" | "boolean" | "never" | "void" | "bytes" | "Component" | "JsError" => {
                    Type::Named { name }
                }
                "Option" => {
                    self.error_span(
                        *span,
                        "Option requires a type argument, e.g. Option<number>",
                    );
                    Type::Error
                }
                _ => {
                    if let Some(param) = self.lookup_type_param(name) {
                        return param;
                    }
                    if self.structs.contains_key(name) {
                        Type::Struct { name }
                    } else if self.enums.contains_key(name) {
                        Type::Named { name }
                    } else if self.interfaces.contains_key(name) {
                        Type::Interface { name }
                    } else if let Some(info) = self.newtypes.get(name).cloned() {
                        Type::Newtype { name, repr: info.repr }
                    } else if let Some(alias) = self.aliases.get(name).cloned() {
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
                } else if base == &"Promise" {
                    if args.len() == 1 {
                        Type::Generic {
                            base: "Promise",
                            args: vec![self.resolve_ast_type_rec(&args[0], seen)],
                        }
                    } else {
                        self.error_span(*span, "Promise requires exactly one type argument");
                        Type::Error
                    }
                } else if base == &"Array" {
                    if args.len() == 1 {
                        Type::Array {
                            elem: Box::new(self.resolve_ast_type_rec(&args[0], seen)),
                        }
                    } else {
                        self.error_span(*span, "Array requires exactly one type argument");
                        Type::Error
                    }
                } else if base == &"Result" {
                    if args.len() == 2 {
                        Type::Generic {
                            base: "Result",
                            args: args
                                .iter()
                                .map(|arg| self.resolve_ast_type_rec(arg, seen))
                                .collect(),
                        }
                    } else {
                        self.error_span(*span, "Result requires exactly two type arguments");
                        Type::Error
                    }
                } else if self.enums.contains_key(base) || self.structs.contains_key(base) {
                    Type::Generic {
                        base,
                        args: args
                            .iter()
                            .map(|arg| self.resolve_ast_type_rec(arg, seen))
                            .collect(),
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
                optional: 0,
            },

            ast::Type::Option { inner, .. } => Type::Option {
                inner: Box::new(self.resolve_ast_type_rec(inner, seen)),
            },

            // Membership and overlap validation for unions lives in
            // `check_union_members` (deka#530, phase 3); here we only resolve
            // members so aliases and type params keep working through them.
            ast::Type::Union { members, .. } => Type::Union {
                members: members
                    .iter()
                    .map(|member| self.resolve_ast_type_rec(member, seen))
                    .collect(),
            },

            ast::Type::Tuple { span, .. } | ast::Type::Record { span, .. } => {
                self.error_span(*span, "tuple/record types are not supported in v2 typeck");
                Type::Error
            }
        }
    }

    pub(super) fn error_span(&mut self, span: ast::Span, message: impl Into<String>) {
        if self.infer_only {
            return;
        }
        self.errors.push(Diagnostic::error(
            span.start.line,
            span.start.column,
            message,
        ));
    }

    pub(super) fn lookup_type_param(&self, name: &'a str) -> Option<Type<'a>> {
        for scope in self.type_scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None
    }
}
