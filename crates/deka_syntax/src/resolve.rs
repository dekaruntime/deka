//! AST resolution passes.
//!
//! This module runs between parsing and typechecking to resolve syntactic
//! ambiguities that cannot be decided by the parser alone.

use std::collections::{HashMap, HashSet};

use bumpalo::Bump;

use crate::ast;
use crate::ast::{Expr, Program, Stmt};

/// Rewrite enum member access into explicit enum constructor expressions.
///
/// The parser cannot distinguish `Color.Red` (enum constructor) from
/// `value.field` (field access), because it does not know which identifiers
/// name enums. This pass runs after the whole program is parsed, collects
/// enum declarations, and resugars:
///
/// - `EnumName.Case` -> `EnumConstructor { enum_name, case_name, payload: None }`
/// - `EnumName.Case(payload)` -> `EnumConstructor { enum_name, case_name, payload: Some(payload) }`
///
/// Prelude enums (`Option`, `Result`) are included so `Option.Some(5)` and
/// `Result.Ok(x)` work as well as the bare `Some(5)` / `Ok(x)` shorthand.
pub fn resolve_enum_constructors<'a>(program: &mut Program<'a>, arena: &'a Bump) {
    let mut enums: HashMap<&'a str, HashSet<&'a str>> = HashMap::new();

    // Collect user-defined enums.
    for stmt in program.statements.iter() {
        if let Stmt::Enum { name, cases, .. } = stmt {
            let set: HashSet<&'a str> = cases.iter().map(|c| c.name).collect();
            enums.insert(name, set);
        }
    }

    // Seed prelude enums.
    let mut option = HashSet::new();
    option.insert("Some");
    option.insert("None");
    enums.insert("Option", option);

    let mut result = HashSet::new();
    result.insert("Ok");
    result.insert("Err");
    enums.insert("Result", result);

    let transformed: Vec<Stmt<'a>> = program
        .statements
        .iter()
        .map(|stmt| transform_stmt(stmt, arena, &enums))
        .collect();

    program.statements = ast::alloc_slice(arena, transformed);
}

fn transform_stmt<'a>(
    stmt: &'a Stmt<'a>,
    arena: &'a Bump,
    enums: &HashMap<&'a str, HashSet<&'a str>>,
) -> Stmt<'a> {
    match stmt {
        Stmt::Export { decl, span } => {
            let new_decl = match decl {
                ast::ExportDecl::Const { name, ty, value } => {
                    let new_value = transform_expr(value, arena, enums);
                    ast::ExportDecl::Const {
                        name,
                        ty: ty.clone(),
                        value: new_value.clone(),
                    }
                }
                ast::ExportDecl::Function {
                    name,
                    type_params,
                    params,
                    return_type,
                    body,
                } => {
                    let new_body: Vec<Stmt<'a>> = body
                        .iter()
                        .map(|s| transform_stmt(s, arena, enums))
                        .collect();
                    ast::ExportDecl::Function {
                        name,
                        type_params,
                        params,
                        return_type: return_type.clone(),
                        body: ast::alloc_slice(arena, new_body),
                    }
                }
            };
            Stmt::Export {
                decl: new_decl,
                span: *span,
            }
        }
        Stmt::Import { .. } => return stmt.clone(),
        Stmt::Const {
            name,
            ty,
            value,
            span,
        } => Stmt::Const {
            name,
            ty: ty.clone(),
            value: transform_expr(value, arena, enums).clone(),
            span: *span,
        },
        Stmt::Let {
            name,
            ty,
            value,
            span,
        } => Stmt::Let {
            name,
            ty: ty.clone(),
            value: transform_expr(value, arena, enums).clone(),
            span: *span,
        },
        Stmt::Function {
            name,
            type_params,
            params,
            return_type,
            body,
            span,
        } => {
            let new_body: Vec<Stmt<'a>> = body
                .iter()
                .map(|s| transform_stmt(s, arena, enums))
                .collect();
            Stmt::Function {
                name,
                type_params,
                params,
                return_type: return_type.clone(),
                body: ast::alloc_slice(arena, new_body),
                span: *span,
            }
        }
        Stmt::ReceiverMethod {
            receiver_type,
            name,
            type_params,
            params,
            return_type,
            body,
            span,
        } => {
            let new_body: Vec<Stmt<'a>> = body
                .iter()
                .map(|s| transform_stmt(s, arena, enums))
                .collect();
            Stmt::ReceiverMethod {
                receiver_type,
                name,
                type_params,
                params,
                return_type: return_type.clone(),
                body: ast::alloc_slice(arena, new_body),
                span: *span,
            }
        }
        Stmt::Struct {
            name,
            type_params,
            fields,
            embeds,
            span,
        } => {
            let new_fields: Vec<ast::StructField<'a>> = fields
                .iter()
                .map(|f| ast::StructField {
                    name: f.name,
                    ty: f.ty.clone(),
                    default_value: f
                        .default_value
                        .as_ref()
                        .map(|v| transform_expr(v, arena, enums).clone()),
                    span: f.span,
                })
                .collect();
            Stmt::Struct {
                name,
                type_params,
                fields: ast::alloc_slice(arena, new_fields),
                embeds,
                span: *span,
            }
        }
        Stmt::Enum { .. } | Stmt::TypeAlias { .. } => return stmt.clone(),
        Stmt::Expr { expr, span } => Stmt::Expr {
            expr: transform_expr(expr, arena, enums).clone(),
            span: *span,
        },
        Stmt::Return { value, span } => Stmt::Return {
            value: value.as_ref().map(|v| transform_expr(v, arena, enums).clone()),
            span: *span,
        },
        Stmt::If {
            condition,
            then_body,
            else_body,
            span,
        } => {
            let new_then: Vec<Stmt<'a>> = then_body
                .iter()
                .map(|s| transform_stmt(s, arena, enums))
                .collect();
            let new_else: Vec<Stmt<'a>> = else_body
                .iter()
                .map(|s| transform_stmt(s, arena, enums))
                .collect();
            Stmt::If {
                condition: transform_expr(condition, arena, enums).clone(),
                then_body: ast::alloc_slice(arena, new_then),
                else_body: ast::alloc_slice(arena, new_else),
                span: *span,
            }
        }
        Stmt::For {
            init,
            condition,
            step,
            body,
            span,
        } => {
            let new_init = init.as_ref().map(|i| match i {
                ast::ForInit::Const { name, value } => ast::ForInit::Const {
                    name,
                    value: transform_expr(value, arena, enums).clone(),
                },
                ast::ForInit::Let { name, value } => ast::ForInit::Let {
                    name,
                    value: transform_expr(value, arena, enums).clone(),
                },
                ast::ForInit::Expr(expr) => {
                    ast::ForInit::Expr(transform_expr(expr, arena, enums).clone())
                }
            });
            let new_body: Vec<Stmt<'a>> = body
                .iter()
                .map(|s| transform_stmt(s, arena, enums))
                .collect();
            Stmt::For {
                init: new_init,
                condition: condition
                    .as_ref()
                    .map(|c| transform_expr(c, arena, enums).clone()),
                step: step.as_ref().map(|s| transform_expr(s, arena, enums).clone()),
                body: ast::alloc_slice(arena, new_body),
                span: *span,
            }
        }
    }
}

fn transform_expr<'a>(
    expr: &'a Expr<'a>,
    arena: &'a Bump,
    enums: &HashMap<&'a str, HashSet<&'a str>>,
) -> &'a Expr<'a> {
    if let Some(resolved) = try_resolve_enum_expr(expr, enums) {
        return alloc_expr(arena, resolved);
    }

    let new_expr = match expr {
        Expr::Number { .. }
        | Expr::BigInt { .. }
        | Expr::String { .. }
        | Expr::Boolean { .. }
        | Expr::None { .. }
        | Expr::Identifier { .. } => return expr,

        Expr::Binary { op, left, right, span } => Expr::Binary {
            op: *op,
            left: transform_expr(left, arena, enums),
            right: transform_expr(right, arena, enums),
            span: *span,
        },
        Expr::Unary { op, operand, span } => Expr::Unary {
            op: *op,
            operand: transform_expr(operand, arena, enums),
            span: *span,
        },
        Expr::Call {
            callee,
            type_args,
            args,
            span,
        } => Expr::Call {
            callee: transform_expr(callee, arena, enums),
            type_args,
            args: transform_exprs(args, arena, enums),
            span: *span,
        },
        Expr::FieldAccess { object, field, span } => Expr::FieldAccess {
            object: transform_expr(object, arena, enums),
            field,
            span: *span,
        },
        Expr::IndexAccess { object, index, span } => Expr::IndexAccess {
            object: transform_expr(object, arena, enums),
            index: transform_expr(index, arena, enums),
            span: *span,
        },
        Expr::StructLiteral { name, fields, span } => Expr::StructLiteral {
            name,
            fields: transform_struct_fields(fields, arena, enums),
            span: *span,
        },
        Expr::EnumConstructor {
            enum_name,
            case_name,
            payload,
            span,
        } => Expr::EnumConstructor {
            enum_name,
            case_name,
            payload: payload
                .as_ref()
                .map(|p| transform_expr(p, arena, enums) as &'a Expr<'a>),
            span: *span,
        },
        Expr::Match {
            scrutinee,
            arms,
            span,
        } => Expr::Match {
            scrutinee: transform_expr(scrutinee, arena, enums),
            arms: transform_match_arms(arms, arena, enums),
            span: *span,
        },
        Expr::Unsafe { body, span } => {
            let new_body: Vec<Stmt<'a>> = body
                .iter()
                .map(|s| transform_stmt(s, arena, enums))
                .collect();
            Expr::Unsafe {
                body: ast::alloc_slice(arena, new_body),
                span: *span,
            }
        }
        Expr::Pipe { left, right, span } => Expr::Pipe {
            left: transform_expr(left, arena, enums),
            right: transform_expr(right, arena, enums),
            span: *span,
        },
        Expr::Await { expr, span } => Expr::Await {
            expr: transform_expr(expr, arena, enums),
            span: *span,
        },
        Expr::JsxElement { element, span } => Expr::JsxElement {
            element: transform_jsx_element(element, arena, enums),
            span: *span,
        },
        Expr::JsxFragment { children, span } => Expr::JsxFragment {
            children: transform_exprs(children, arena, enums),
            span: *span,
        },
        Expr::Array { elements, span } => Expr::Array {
            elements: transform_exprs(elements, arena, enums),
            span: *span,
        },
        Expr::Object { fields, span } => Expr::Object {
            fields: transform_object_fields(fields, arena, enums),
            span: *span,
        },
        Expr::Spread { expr, span } => Expr::Spread {
            expr: transform_expr(expr, arena, enums),
            span: *span,
        },
        Expr::Paren { expr, span } => Expr::Paren {
            expr: transform_expr(expr, arena, enums),
            span: *span,
        },
        Expr::ArrowFunction { params, return_type, body, span } => {
            let new_body = match body {
                ast::ArrowBody::Expr(e) => {
                    ast::ArrowBody::Expr(transform_expr(e, arena, enums))
                }
                ast::ArrowBody::Block(stmts) => {
                    let new_stmts: Vec<Stmt<'a>> = stmts
                        .iter()
                        .map(|s| transform_stmt(s, arena, enums))
                        .collect();
                    ast::ArrowBody::Block(ast::alloc_slice(arena, new_stmts))
                }
            };
            Expr::ArrowFunction {
                params,
                return_type: return_type.clone(),
                body: new_body,
                span: *span,
            }
        }
    };

    alloc_expr(arena, new_expr)
}

fn transform_exprs<'a>(
    exprs: &'a [Expr<'a>],
    arena: &'a Bump,
    enums: &HashMap<&'a str, HashSet<&'a str>>,
) -> &'a [Expr<'a>] {
    let transformed: Vec<Expr<'a>> = exprs
        .iter()
        .map(|e| transform_expr(e, arena, enums).clone())
        .collect();
    ast::alloc_slice(arena, transformed)
}

fn transform_struct_fields<'a>(
    fields: &'a [ast::StructLiteralField<'a>],
    arena: &'a Bump,
    enums: &HashMap<&'a str, HashSet<&'a str>>,
) -> &'a [ast::StructLiteralField<'a>] {
    let transformed: Vec<ast::StructLiteralField<'a>> = fields
        .iter()
        .map(|f| ast::StructLiteralField {
            name: f.name,
            value: transform_expr(&f.value, arena, enums).clone(),
            span: f.span,
        })
        .collect();
    ast::alloc_slice(arena, transformed)
}

fn transform_object_fields<'a>(
    fields: &'a [ast::ObjectField<'a>],
    arena: &'a Bump,
    enums: &HashMap<&'a str, HashSet<&'a str>>,
) -> &'a [ast::ObjectField<'a>] {
    let transformed: Vec<ast::ObjectField<'a>> = fields
        .iter()
        .map(|f| ast::ObjectField {
            key: f.key,
            value: transform_expr(&f.value, arena, enums).clone(),
            span: f.span,
        })
        .collect();
    ast::alloc_slice(arena, transformed)
}

fn transform_match_arms<'a>(
    arms: &'a [ast::MatchArm<'a>],
    arena: &'a Bump,
    enums: &HashMap<&'a str, HashSet<&'a str>>,
) -> &'a [ast::MatchArm<'a>] {
    let transformed: Vec<ast::MatchArm<'a>> = arms
        .iter()
        .map(|arm| ast::MatchArm {
            pattern: arm.pattern.clone(),
            guard: arm
                .guard
                .as_ref()
                .map(|g| transform_expr(g, arena, enums).clone()),
            body: transform_expr(&arm.body, arena, enums).clone(),
            span: arm.span,
        })
        .collect();
    ast::alloc_slice(arena, transformed)
}

fn transform_jsx_element<'a>(
    element: &ast::JsxElement<'a>,
    arena: &'a Bump,
    enums: &HashMap<&'a str, HashSet<&'a str>>,
) -> ast::JsxElement<'a> {
    let new_attrs: Vec<ast::JsxAttribute<'a>> = element
        .attributes
        .iter()
        .map(|attr| ast::JsxAttribute {
            name: attr.name,
            value: attr
                .value
                .as_ref()
                .map(|v| transform_expr(v, arena, enums).clone()),
            span: attr.span,
        })
        .collect();
    ast::JsxElement {
        tag: element.tag,
        attributes: ast::alloc_slice(arena, new_attrs),
        children: transform_exprs(element.children, arena, enums),
        span: element.span,
    }
}

/// If `expr` is `EnumName.Case` or `EnumName.Case(payload)`, return the
/// corresponding `EnumConstructor` expression. Otherwise return `None`.
fn try_resolve_enum_expr<'a>(
    expr: &Expr<'a>,
    enums: &HashMap<&'a str, HashSet<&'a str>>,
) -> Option<Expr<'a>> {
    match expr {
        Expr::FieldAccess { object, field, span } => {
            let enum_name = match object {
                Expr::Identifier { name, .. } => *name,
                _ => return None,
            };
            let cases = enums.get(enum_name)?;
            if !cases.contains(field) {
                return None;
            }
            Some(Expr::EnumConstructor {
                enum_name,
                case_name: field,
                payload: None,
                span: *span,
            })
        }
        Expr::Call {
            callee,
            args,
            span,
            ..
        } => {
            let (object, field) = match callee {
                Expr::FieldAccess { object, field, .. } => (object, *field),
                _ => return None,
            };
            let enum_name = match object {
                Expr::Identifier { name, .. } => *name,
                _ => return None,
            };
            let cases = enums.get(enum_name)?;
            if !cases.contains(field) {
                return None;
            }
            // Enum cases in this AST hold at most one payload.
            let payload = if args.is_empty() {
                None
            } else {
                Some(args.first().unwrap() as &'a Expr<'a>)
            };
            Some(Expr::EnumConstructor {
                enum_name,
                case_name: field,
                payload,
                span: *span,
            })
        }
        _ => None,
    }
}

fn alloc_expr<'a>(arena: &'a Bump, expr: Expr<'a>) -> &'a Expr<'a> {
    ast::alloc(arena, expr)
}
