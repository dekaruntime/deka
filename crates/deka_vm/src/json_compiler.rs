//! Compiler-private JSON schemas and nominal factory closure wiring.
use super::*;
use deka_syntax::typeck::{DescriptorTree, JsonDescriptor, JsonOperation};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) struct Call {
    pub operation: JsonOperation,
    pub shape: Result<crate::JsonShape>,
    pub body_operation: Option<String>,
}

#[derive(Default)]
pub(super) struct JsonTypes {
    contexts: HashMap<PathBuf, BTreeMap<String, String>>,
    origins: BTreeMap<String, PathBuf>,
    embeds: BTreeMap<String, Vec<String>>,
}
impl JsonTypes {
    pub(super) fn new(
        asts: &HashMap<PathBuf, &deka_syntax::ast::Program<'_>>,
        identities: &HashMap<PathBuf, BTreeMap<String, String>>,
        edges: &HashMap<PathBuf, Vec<(String, String, PathBuf)>>,
    ) -> Self {
        let mut types = Self {
            contexts: identities.clone(),
            ..Default::default()
        };
        for (path, ast) in asts {
            for stmt in ast.statements {
                if let Stmt::Struct { name, embeds, .. } = stmt {
                    let identity = &identities[path][*name];
                    types.origins.insert(identity.clone(), path.clone());
                    types.embeds.insert(
                        identity.clone(),
                        embeds.iter().map(|e| e.name.to_owned()).collect(),
                    );
                }
            }
        }
        for (path, imports) in edges {
            for (local, original, target) in imports {
                if let Some(identity) = identities[target].get(original) {
                    types
                        .contexts
                        .get_mut(path)
                        .expect("module context")
                        .insert(local.clone(), identity.clone());
                }
            }
        }
        types
    }
    pub(super) fn shape(
        &self,
        tree: &JsonDescriptor<'_>,
        module: &Path,
    ) -> Result<crate::JsonShape> {
        use crate::{JsonField, JsonShape};
        Ok(match tree {
            JsonDescriptor::Type(tree) => self.type_shape(tree, module)?,
            JsonDescriptor::Record(fields) => JsonShape::Record(
                fields
                    .iter()
                    .map(|(name, ty)| {
                        Ok(JsonField {
                            name: (*name).into(),
                            shape: self.shape(ty, module)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
            JsonDescriptor::Array(elem) => JsonShape::Array(Box::new(self.shape(elem, module)?)),
            JsonDescriptor::Tuple(elements) => JsonShape::Tuple(
                elements
                    .iter()
                    .map(|ty| self.shape(ty, module))
                    .collect::<Result<Vec<_>>>()?,
            ),
        })
    }
    fn type_shape(&self, tree: &DescriptorTree<'_>, module: &Path) -> Result<crate::JsonShape> {
        use crate::{JsonField, JsonShape};
        Ok(match tree {
            DescriptorTree::Leaf { kind: kind @ ("number" | "string" | "boolean"), .. } => JsonShape::Leaf((*kind).into()),
            DescriptorTree::Struct { name, fields } => {
                let identity = self.contexts.get(module).and_then(|names| names.get(*name))
                    .ok_or_else(|| format!("JSON struct `{name}` has no declaration identity"))?;
                let origin = &self.origins[identity];
                let fields = fields.iter().map(|field| {
                    if field.optional {
                        return Err(format!("JSON encoding for optional field `{}` awaits the APS 43 type-mapping decision", field.name));
                    }
                    Ok(JsonField { name: field.name.into(), shape: self.type_shape(&field.ty, origin)? })
                }).collect::<Result<Vec<_>>>()?;
                JsonShape::Struct { name: (*name).into(), identity: identity.clone(), fields,
                    embeds: self.embeds[identity].clone() }
            }
            DescriptorTree::Array { elem } => JsonShape::Array(Box::new(self.type_shape(elem, module)?)),
            DescriptorTree::Tuple { elements } => JsonShape::Tuple(elements.iter().map(|e| self.type_shape(e, module)).collect::<Result<Vec<_>>>()?),
            _ => return Err("JSON encoding for this type awaits the APS 43 type-mapping decision; supported types are number, string, boolean, arrays, tuples and structs with required fields".into()),
        })
    }
}
fn factory_name(identity: &str) -> String {
    format!("<JSON methods {identity}>")
}
fn factory_ids(shape: &crate::JsonShape, out: &mut std::collections::BTreeSet<String>) {
    match shape {
        crate::JsonShape::Struct {
            identity, fields, ..
        } => {
            out.insert(identity.clone());
            for field in fields {
                factory_ids(&field.shape, out);
            }
        }
        crate::JsonShape::Record(fields) => {
            for field in fields {
                factory_ids(&field.shape, out);
            }
        }
        crate::JsonShape::Array(elem) => factory_ids(elem, out),
        crate::JsonShape::Tuple(elements) => {
            for elem in elements {
                factory_ids(elem, out);
            }
        }
        crate::JsonShape::Leaf(_) => {}
    }
}
impl<'a> Lower<'a> {
    pub(super) fn reserve_json_factories(&mut self, module: &Path, c: &mut Context) {
        for name in self
            .structs
            .keys()
            .filter(|name| !self.imported_structs.contains(*name))
        {
            let identity = &self.struct_identities[name];
            let slot = c.bind(&factory_name(identity));
            self.json_factories
                .insert(identity.clone(), (module.to_path_buf(), slot));
        }
    }
    pub(super) fn finish_json_factories(&self, c: &mut Context) -> Result<()> {
        for name in self
            .structs
            .keys()
            .filter(|name| !self.imported_structs.contains(*name))
        {
            let identity = &self.struct_identities[name];
            c.emit(Op::Record(vec![]));
            self.attach_methods(name, c)?;
            let slot = c.slot(&factory_name(identity))?;
            c.emit(Op::Store(slot));
        }
        Ok(())
    }
    pub(super) fn json_call(&mut self, e: &Expr<'a>, c: &mut Context) -> Result<bool> {
        let Some(Call {
            operation,
            shape,
            body_operation,
        }) = self.json_calls.get(&(e as *const Expr as usize)).cloned()
        else {
            return Ok(false);
        };
        let Expr::Call {
            callee: Expr::FieldAccess { object, .. },
            args,
            ..
        } = e
        else {
            return Err("checked JSON call has an invalid shape".into());
        };
        let shape = shape?;
        if let Some(operation) = body_operation {
            // Start consumption now, before spawning the conversion task. This
            // makes bodyUsed and competing reads identical to text()/bytes().
            let function = self.functions.len();
            self.functions.push(Function {
                name: "<JSON body>".into(),
                parameters: 2,
                captures: 0,
                locals: 2,
                asynchronous: true,
                code: vec![
                    Op::Load(0),
                    Op::Await,
                    Op::Load(1),
                    Op::JsonParseResult(shape.clone()),
                    Op::Return,
                ],
            });
            c.emit(Op::Closure {
                function,
                captures: vec![],
            });
            self.expr(object, c)?;
            c.emit(Op::Host {
                operation,
                arguments: 1,
            });
            self.json_factory_record(&shape, c)?;
            c.emit(Op::Call(2));
            return Ok(true);
        }
        let input = if let [argument] = *args {
            argument
        } else {
            object
        };
        self.expr(input, c)?;
        match operation {
            JsonOperation::ToJson => {
                c.emit(Op::JsonStringify(shape));
            }
            JsonOperation::ParseJson => {
                self.json_factory_record(&shape, c)?;
                c.emit(Op::JsonParse(shape));
            }
        }
        Ok(true)
    }
    fn json_factory_record(&self, shape: &crate::JsonShape, c: &mut Context) -> Result<()> {
        let mut factories = Default::default();
        factory_ids(shape, &mut factories);
        for identity in &factories {
            c.emit_load(&factory_name(identity))?;
        }
        c.emit(Op::Record(factories.into_iter().collect()));
        Ok(())
    }
    pub(super) fn import_json_factories(&self, module: &Path, c: &mut Context) {
        for (identity, (origin, slot)) in &self.json_factories {
            if origin != module {
                c.names.insert(factory_name(identity), *slot);
            }
        }
    }
}
