use std::collections::{HashMap, HashSet};

use bumpalo::Bump;
use deno_core::op2;
use serde::Serialize;

#[derive(Serialize, Clone)]
#[allow(dead_code)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum BridgeType {
    Unknown,
    Mixed,
    Primitive {
        name: String,
    },
    Array {
        element: Option<Box<BridgeType>>,
    },
    Object,
    ObjectShape {
        fields: Vec<BridgeField>,
    },
    Struct {
        name: String,
        fields: Vec<BridgeField>,
    },
    Union {
        types: Vec<BridgeType>,
    },
    Option {
        inner: Option<Box<BridgeType>>,
    },
    Result {
        ok: Option<Box<BridgeType>>,
        err: Option<Box<BridgeType>>,
    },
    Applied {
        base: String,
        args: Vec<BridgeType>,
    },
}

#[derive(Serialize, Clone)]
pub(super) struct BridgeField {
    name: String,
    #[serde(rename = "type")]
    ty: BridgeType,
    optional: bool,
}

#[derive(Serialize)]
pub(super) struct BridgeParam {
    #[serde(rename = "type")]
    ty: Option<BridgeType>,
    required: bool,
    variadic: bool,
}

#[derive(Serialize)]
pub(super) struct BridgeFunction {
    params: Vec<BridgeParam>,
    #[serde(rename = "return")]
    return_type: Option<BridgeType>,
    variadic: bool,
}

#[derive(Serialize)]
pub(super) struct BridgeStruct {
    fields: Vec<BridgeField>,
}

#[derive(Serialize)]
pub(super) struct BridgeModuleTypes {
    functions: HashMap<String, BridgeFunction>,
    structs: HashMap<String, BridgeStruct>,
}

#[derive(Clone)]
struct TypeAliasInfo<'a> {
    params: Vec<String>,
    ty: &'a deka_syntax::Type<'a>,
}

struct TypeResolver<'a> {
    aliases: HashMap<String, TypeAliasInfo<'a>>,
    structs: HashMap<String, Vec<BridgeField>>,
}

impl<'a> TypeResolver<'a> {
    fn new(program: &'a deka_syntax::Program<'a>) -> Self {
        let aliases = collect_aliases(program);
        let structs = collect_structs(program, &aliases);
        Self { aliases, structs }
    }

    fn convert_type(&mut self, ty: &'a deka_syntax::Type<'a>) -> BridgeType {
        let mut guard = HashSet::new();
        self.convert_type_internal(ty, &mut guard, None)
    }

    fn convert_type_internal(
        &mut self,
        ty: &'a deka_syntax::Type<'a>,
        alias_guard: &mut HashSet<String>,
        subs: Option<&HashMap<String, BridgeType>>,
    ) -> BridgeType {
        let resolve_param = |name: &str, subs: Option<&HashMap<String, BridgeType>>| {
            subs.and_then(|map| map.get(name).cloned())
        };

        match ty {
            deka_syntax::Type::Named { name, .. } => {
                let name = *name;
                if let Some(bound) = resolve_param(name, subs) {
                    return bound;
                }
                if let Some(resolved) = self.convert_named(name, alias_guard) {
                    return resolved;
                }
                BridgeType::Unknown
            }
            deka_syntax::Type::Option { inner, .. } => {
                let inner = self.convert_type_internal(inner, alias_guard, subs);
                BridgeType::Option {
                    inner: Some(Box::new(inner)),
                }
            }
            deka_syntax::Type::Generic { base, args, .. } => {
                if let Some(bound) = resolve_param(base, subs) {
                    return bound;
                }

                if let Some(alias) = self.aliases.get(*base).cloned() {
                    if alias.params.len() == args.len() {
                        let mut param_map = HashMap::new();
                        for (idx, param) in alias.params.iter().enumerate() {
                            let arg_ty = self.convert_type_internal(&args[idx], alias_guard, subs);
                            param_map.insert(param.clone(), arg_ty);
                        }
                        if alias_guard.insert(base.to_string()) {
                            let resolved =
                                self.convert_type_internal(alias.ty, alias_guard, Some(&param_map));
                            alias_guard.remove(*base);
                            return resolved;
                        }
                        return BridgeType::Mixed;
                    }
                }

                let base_id = base.to_ascii_lowercase();
                let mut converted_args = Vec::new();
                for arg in *args {
                    converted_args.push(self.convert_type_internal(arg, alias_guard, subs));
                }
                if base_id == "option" {
                    return BridgeType::Option {
                        inner: converted_args.into_iter().next().map(Box::new),
                    };
                }
                if base_id == "result" {
                    let mut iter = converted_args.into_iter();
                    let ok = iter.next().map(Box::new);
                    let err = iter.next().map(Box::new);
                    return BridgeType::Result { ok, err };
                }
                if base_id == "array" {
                    let element = converted_args.into_iter().next().map(Box::new);
                    return BridgeType::Array { element };
                }
                BridgeType::Applied {
                    base: base.to_string(),
                    args: converted_args,
                }
            }
            deka_syntax::Type::Function { .. } => BridgeType::Mixed,
            deka_syntax::Type::Tuple { .. } => BridgeType::Mixed,
            deka_syntax::Type::Record { fields, .. } => {
                let out = fields
                    .iter()
                    .map(|field| BridgeField {
                        name: field.name.to_string(),
                        ty: self.convert_type_internal(&field.ty, alias_guard, subs),
                        optional: false,
                    })
                    .collect();
                BridgeType::ObjectShape { fields: out }
            }
        }
    }

    fn convert_named(&mut self, name: &str, alias_guard: &mut HashSet<String>) -> Option<BridgeType> {
        let base = name.rsplit('\\').next().unwrap_or(name).to_ascii_lowercase();
        match base.as_str() {
            "mixed" => Some(BridgeType::Mixed),
            "int" | "float" | "bool" | "string" | "null" => Some(BridgeType::Primitive {
                name: base.to_string(),
            }),
            "array" => Some(BridgeType::Array { element: None }),
            "object" => Some(BridgeType::Object),
            "option" => Some(BridgeType::Option { inner: None }),
            "result" => Some(BridgeType::Result {
                ok: None,
                err: None,
            }),
            _ => {
                if let Some(fields) = self.structs.get(name).cloned() {
                    return Some(BridgeType::Struct {
                        name: name.to_string(),
                        fields,
                    });
                }
                if let Some(alias) = self.aliases.get(name).cloned() {
                    if alias.params.is_empty() && alias_guard.insert(name.to_string()) {
                        let resolved = self.convert_type_internal(alias.ty, alias_guard, None);
                        alias_guard.remove(name);
                        return Some(resolved);
                    }
                }
                None
            }
        }
    }
}

fn collect_aliases<'a>(program: &'a deka_syntax::Program<'a>) -> HashMap<String, TypeAliasInfo<'a>> {
    let mut out = HashMap::new();
    for stmt in program.statements.iter() {
        if let deka_syntax::Stmt::TypeAlias {
            name,
            type_params,
            value,
            ..
        } = stmt
        {
            let params = type_params.iter().map(|p| p.name.to_string()).collect();
            out.insert(name.to_string(), TypeAliasInfo { params, ty: value });
        }
    }
    out
}

fn collect_structs<'a>(
    program: &'a deka_syntax::Program<'a>,
    aliases: &HashMap<String, TypeAliasInfo<'a>>,
) -> HashMap<String, Vec<BridgeField>> {
    let mut out = HashMap::new();
    for stmt in program.statements.iter() {
        let deka_syntax::Stmt::Struct { name, fields, .. } = stmt else {
            continue;
        };
        let mut resolver = TypeResolver {
            aliases: aliases.clone(),
            structs: HashMap::new(),
        };
        let bridge_fields = fields
            .iter()
            .map(|field| BridgeField {
                name: field.name.to_string(),
                ty: resolver.convert_type(&field.ty),
                optional: field.optional,
            })
            .collect();
        out.insert(name.to_string(), bridge_fields);
    }
    out
}

#[op2]
#[serde]
pub(super) fn op_php_parse_phpx_types(
    #[string] source: String,
    #[string] file_path: String,
) -> Result<BridgeModuleTypes, deno_core::error::CoreError> {
    // Legacy PHPX stubs may still start with a `<?php` tag; strip it before
    // feeding the source to the DekaScript parser.
    let source_text = if let Some(rest) = source.trim_start().strip_prefix("<?php") {
        rest.to_string()
    } else {
        source
    };

    let arena = Bump::new();
    let result = deka_syntax::parse(&source_text, &arena);
    if !result.errors.is_empty() {
        return Err(deno_core::error::CoreError::from(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "Failed to parse DekaScript types for '{}': {:?}",
                file_path, result.errors
            ),
        )));
    }
    let Some(program) = result.program else {
        return Err(deno_core::error::CoreError::from(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to parse DekaScript types for '{}': no program", file_path),
        )));
    };

    let mut resolver = TypeResolver::new(&program);
    let mut functions = HashMap::new();
    for stmt in program.statements.iter() {
        if let deka_syntax::Stmt::Function {
            name,
            params,
            return_type,
            ..
        } = stmt
        {
            let mut params_out = Vec::new();
            for param in params.iter() {
                let ty = param.ty.as_ref().map(|ty| resolver.convert_type(ty));
                let required = param.default_value.is_none();
                params_out.push(BridgeParam {
                    ty,
                    required,
                    variadic: false,
                });
            }
            let return_type = return_type.as_ref().map(|ty| resolver.convert_type(ty));
            functions.insert(
                name.to_string(),
                BridgeFunction {
                    params: params_out,
                    return_type,
                    variadic: false,
                },
            );
        }
    }

    Ok(BridgeModuleTypes {
        functions,
        structs: resolver
            .structs
            .iter()
            .map(|(k, v)| (k.clone(), BridgeStruct { fields: v.clone() }))
            .collect(),
    })
}
