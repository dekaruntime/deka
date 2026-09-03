//! Static descriptor trees for `super` (deka#529, rfd#41).
//!
//! A `super fn validate<T>` call site names a concrete `T`, and the compiler
//! walks that type into a [`DescriptorTree`] — the static counterpart of the
//! runtime `__deka_type_of` descriptor. The tree is computed here in the
//! typechecker (the emitter never sees types; the lowering maps are the only
//! typeck→emitter channel, same pattern as `method_calls`/`type_of_calls`)
//! and printed by the emitter as frozen, content-deduped module-local
//! `__deka_super_desc$N` consts.
//!
//! Shape rule (same one #550 applied to `{kind, name, toString}`): every node
//! keeps `{kind, name, toString()}` so static and runtime descriptors stay
//! interchangeable, and composite nodes expose `fields`/`cases`/`elem`/
//! `inner`/`members`/`repr` so the schema endgame is not precluded.

use std::collections::HashSet;

use crate::ast;

use super::types::Type;
use super::Checker;

/// A fully-resolved static type descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescriptorTree<'a> {
    /// Scalar / marker node: `{kind, name}` with `kind` drawn from the same
    /// vocabulary `__deka_type_of` uses (`number`, `string`, `struct`, …).
    Leaf { kind: &'a str, name: String },
    Struct {
        name: &'a str,
        fields: Vec<DescriptorField<'a>>,
    },
    Newtype {
        name: &'a str,
        repr: Box<DescriptorTree<'a>>,
    },
    Enum {
        name: &'a str,
        cases: Vec<(&'a str, Option<DescriptorTree<'a>>)>,
    },
    Array { elem: Box<DescriptorTree<'a>> },
    Option { inner: Box<DescriptorTree<'a>> },
    Union { members: Vec<DescriptorTree<'a>> },
}

/// One struct field in a descriptor tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptorField<'a> {
    pub name: &'a str,
    pub optional: bool,
    pub ty: DescriptorTree<'a>,
}

/// A recorded `.type()` call site inside a `super` function. `param` is set
/// for `T.type()` (the emitter resolves it against the instantiation's
/// argument); `tree` is set for a concrete receiver (a static descriptor
/// tree, emitted as a `__deka_super_desc$N` const).
#[derive(Clone, Debug)]
pub struct StaticTypeCall<'a> {
    pub tree: Option<DescriptorTree<'a>>,
    pub param: Option<&'a str>,
}

/// A recorded call site of a `super` function. Keyed by call-expression
/// pointer, like every other lowering map. `args` entries name either a type
/// parameter of the enclosing `super fn` (passed through unchanged — the
/// emitter forwards the function's own hidden descriptor parameter) or a
/// concrete type (the emitter references the interned
/// `__deka_super_desc$N` const for its tree).
#[derive(Clone, Debug)]
pub struct SuperCallSite<'a> {
    pub callee: &'a str,
    pub args: Vec<SuperTypeArg<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuperTypeArg<'a> {
    /// A type parameter of the enclosing `super fn`, passed through
    /// unchanged — the emitter forwards the function's hidden descriptor
    /// parameter.
    Param(&'a str),
    /// A concrete type, walked into its descriptor tree at the call site —
    /// the emitter references the interned `__deka_super_desc$N` const for
    /// this tree.
    Concrete(DescriptorTree<'a>),
}

impl<'a> Checker<'a> {
    /// Walk a concrete type into its descriptor tree. Returns the "cannot
    /// describe" message on failure; the caller (a super call site) turns it
    /// into an error at that call site, which is the spec's rule: a type the
    /// compiler cannot describe — one holding a function value or derived
    /// from `unsafe` — is an error at the specific call site, not a viral
    /// constraint.
    pub(super) fn descriptor_tree(
        &mut self,
        ty: &Type<'a>,
        span: ast::Span,
    ) -> Result<DescriptorTree<'a>, String> {
        let mut seen = HashSet::new();
        self.descriptor_tree_rec(ty, span, &mut seen)
    }

    fn descriptor_tree_rec(
        &mut self,
        ty: &Type<'a>,
        span: ast::Span,
        seen: &mut HashSet<&'a str>,
    ) -> Result<DescriptorTree<'a>, String> {
        match ty {
            Type::Named { name } => {
                if self.enums.contains_key(name) {
                    return self.enum_tree(name, &[], span, seen);
                }
                let kind = match *name {
                    "number" | "string" | "boolean" | "bytes" | "void" | "never" => *name,
                    "Type" => "type",
                    other => {
                        // A named type we cannot structurally walk (an
                        // opaque import, an unresolved alias): keep it as an
                        // opaque leaf rather than failing — the name is the
                        // useful information at the call site.
                        return Ok(DescriptorTree::Leaf {
                            kind: "unknown",
                            name: other.to_string(),
                        });
                    }
                };
                Ok(DescriptorTree::Leaf {
                    kind,
                    name: kind.to_string(),
                })
            }
            Type::Struct { name } => self.struct_tree(name, &[], span, seen),
            Type::Generic { base, args } => {
                if self.structs.contains_key(base) {
                    self.struct_tree(base, args, span, seen)
                } else if self.enums.contains_key(base) {
                    self.enum_tree(base, args, span, seen)
                } else if *base == "Result" && args.len() == 2 {
                    // The prelude Result is a real enum at runtime; describe
                    // it structurally so `validate<Result<T, E>>` stays
                    // describable.
                    let ok = self.descriptor_tree_rec(&args[0], span, seen)?;
                    let err = self.descriptor_tree_rec(&args[1], span, seen)?;
                    Ok(DescriptorTree::Enum {
                        name: "Result",
                        cases: vec![
                            ("Ok", Some(ok)),
                            ("Err", Some(err)),
                        ],
                    })
                } else {
                    Err(format!(
                        "cannot describe type `{ty}` at this `super` call site"
                    ))
                }
            }
            Type::Newtype { name, repr } => {
                let repr_tree = DescriptorTree::Leaf {
                    kind: match repr {
                        ast::NewtypeRepr::Number => "number",
                        ast::NewtypeRepr::String => "string",
                        ast::NewtypeRepr::Bool => "boolean",
                    },
                    name: match repr {
                        ast::NewtypeRepr::Number => "number",
                        ast::NewtypeRepr::String => "string",
                        ast::NewtypeRepr::Bool => "boolean",
                    }
                    .to_string(),
                };
                Ok(DescriptorTree::Newtype {
                    name,
                    repr: Box::new(repr_tree),
                })
            }
            Type::Option { inner } => Ok(DescriptorTree::Option {
                inner: Box::new(self.descriptor_tree_rec(inner, span, seen)?),
            }),
            Type::Array { elem } => Ok(DescriptorTree::Array {
                elem: Box::new(self.descriptor_tree_rec(elem, span, seen)?),
            }),
            Type::Union { members } => {
                let mut trees = Vec::with_capacity(members.len());
                for member in members {
                    trees.push(self.descriptor_tree_rec(member, span, seen)?);
                }
                Ok(DescriptorTree::Union { members: trees })
            }
            Type::None => Ok(DescriptorTree::Leaf {
                kind: "none",
                name: "none".to_string(),
            }),
            Type::Param { name } => Err(format!(
                "cannot describe type parameter `{name}` at this `super` call site; \
                 it must be instantiated with a concrete type"
            )),
            Type::Function { .. } => Err(format!(
                "cannot describe type `{ty}` (function-valued) at this `super` call site"
            )),
            Type::Infer | Type::Var => Err(
                "cannot describe this type at this `super` call site: it is derived from \
                 `unsafe` or otherwise unknown to the compiler"
                    .to_string(),
            ),
            Type::Interface { .. } | Type::Object { .. } => Err(format!(
                "cannot describe type `{ty}` at this `super` call site"
            )),
            Type::Error => Err("cannot describe this type at this `super` call site".to_string()),
            Type::Never => Ok(DescriptorTree::Leaf {
                kind: "never",
                name: "never".to_string(),
            }),
        }
    }

    fn struct_tree(
        &mut self,
        name: &'a str,
        args: &[Type<'a>],
        span: ast::Span,
        seen: &mut HashSet<&'a str>,
    ) -> Result<DescriptorTree<'a>, String> {
        if !seen.insert(name) {
            return Err(format!(
                "cannot describe type `{name}` at this `super` call site: it is recursive"
            ));
        }
        let info = match self.structs.get(name) {
            Some(info) => info.clone(),
            None => {
                seen.remove(name);
                return Err(format!(
                    "cannot describe type `{name}` at this `super` call site: its shape is not visible here"
                ));
            }
        };
        let subst = self.generic_subst(&info.type_params, args);
        let mut fields = Vec::new();
        let result: Result<(), String> = (|| {
            for field in info.fields.iter() {
                let resolved = self.resolve_in_declaring_module(&info.type_params, &field.ty);
                let resolved = match subst {
                    Some(ref subst) => super::types::substitute_type(&resolved, subst),
                    None => resolved,
                };
                let tree = self.descriptor_tree_rec(&resolved, span, seen)?;
                fields.push(DescriptorField {
                    name: field.name,
                    optional: field.optional,
                    ty: tree,
                });
            }
            // Embeds are fields for descriptor purposes: named, required, and
            // typed by the embedded struct.
            for embed in info.embeds.iter() {
                let tree = self.struct_tree(embed.name, &[], span, seen)?;
                fields.push(DescriptorField {
                    name: embed.name,
                    optional: false,
                    ty: tree,
                });
            }
            Ok(())
        })();
        seen.remove(name);
        result?;
        Ok(DescriptorTree::Struct { name, fields })
    }

    fn enum_tree(
        &mut self,
        name: &'a str,
        args: &[Type<'a>],
        span: ast::Span,
        seen: &mut HashSet<&'a str>,
    ) -> Result<DescriptorTree<'a>, String> {
        if !seen.insert(name) {
            return Err(format!(
                "cannot describe type `{name}` at this `super` call site: it is recursive"
            ));
        }
        let info = match self.enums.get(name) {
            Some(info) => info.clone(),
            None => {
                seen.remove(name);
                return Err(format!(
                    "cannot describe type `{name}` at this `super` call site: its shape is not visible here"
                ));
            }
        };
        let subst = self.generic_subst(&info.type_params, args);
        let mut cases = Vec::new();
        let mut result: Result<(), String> = Ok(());
        for case in info.cases.iter() {
            let tree = match &case.payload {
                Some(payload) => {
                    let resolved = self.resolve_in_declaring_module(&info.type_params, payload);
                    let resolved = match subst {
                        Some(ref subst) => super::types::substitute_type(&resolved, subst),
                        None => resolved,
                    };
                    match self.descriptor_tree_rec(&resolved, span, seen) {
                        Ok(tree) => Some(tree),
                        Err(message) => {
                            result = Err(message);
                            break;
                        }
                    }
                }
                None => None,
            };
            cases.push((case.name, tree));
        }
        seen.remove(name);
        result?;
        Ok(DescriptorTree::Enum { name, cases })
    }

    /// Zip a generic declaration's type parameters against use-site
    /// arguments. `None` when the declaration is not generic.
    fn generic_subst(
        &mut self,
        type_params: &[ast::TypeParam<'a>],
        args: &[Type<'a>],
    ) -> Option<std::collections::HashMap<&'a str, Type<'a>>> {
        if type_params.is_empty() {
            return None;
        }
        Some(
            type_params
                .iter()
                .map(|p| p.name)
                .zip(args.iter().cloned())
                .collect(),
        )
    }

    /// Resolve a type annotation stored in a *declaration* (struct field,
    /// enum case payload) that may come from another module. The declaring
    /// context's type parameters are pushed so parameter references resolve;
    /// nested diagnostics are suppressed — the caller reports one
    /// "cannot describe" error at the super call site instead.
    fn resolve_in_declaring_module(
        &mut self,
        type_params: &'a [ast::TypeParam<'a>],
        ty: &ast::Type<'a>,
    ) -> Type<'a> {
        let saved_infer_only = self.infer_only;
        self.infer_only = true;
        self.push_type_params(type_params);
        let resolved = self.resolve_ast_type(ty);
        self.pop_type_params();
        self.infer_only = saved_infer_only;
        resolved
    }
}
