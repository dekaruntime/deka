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
                "number" | "string" | "boolean" | "never" => Type::Named { name },
                "Option" => {
                    self.error_span(
                        *span,
                        "Option requires a type argument, e.g. Option<number>",
                    );
                    Type::Error
                }
                _ => {
                    if self.structs.contains_key(name) {
                        Type::Struct { name }
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

    pub(super) fn error_span(&mut self, span: ast::Span, message: impl Into<String>) {
        self.errors.push(Diagnostic::error(
            span.start.line,
            span.start.column,
            message,
        ));
    }
}
