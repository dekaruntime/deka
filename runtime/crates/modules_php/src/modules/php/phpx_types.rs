use super::*;

#[derive(serde::Serialize, Clone)]
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

#[derive(serde::Serialize, Clone)]
pub(super) struct BridgeField {
    name: String,
    #[serde(rename = "type")]
    ty: BridgeType,
    optional: bool,
}

#[derive(serde::Serialize)]
pub(super) struct BridgeParam {
    #[serde(rename = "type")]
    ty: Option<BridgeType>,
    required: bool,
    variadic: bool,
}

#[derive(serde::Serialize)]
pub(super) struct BridgeFunction {
    params: Vec<BridgeParam>,
    #[serde(rename = "return")]
    return_type: Option<BridgeType>,
    variadic: bool,
}

#[derive(serde::Serialize)]
pub(super) struct BridgeStruct {
    fields: Vec<BridgeField>,
}

#[derive(serde::Serialize)]
pub(super) struct BridgeModuleTypes {
    functions: HashMap<String, BridgeFunction>,
    structs: HashMap<String, BridgeStruct>,
}

#[derive(Clone)]
pub(super) struct TypeAliasInfo<'a> {
    params: Vec<String>,
    ty: &'a AstType<'a>,
}

pub(super) struct TypeResolver<'a> {
    source: &'a [u8],
    aliases: HashMap<String, TypeAliasInfo<'a>>,
    structs: HashMap<String, Vec<BridgeField>>,
}

impl<'a> TypeResolver<'a> {
    fn new(source: &'a [u8], program: &'a Program<'a>) -> Self {
        let aliases = collect_aliases(program, source);
        let structs = collect_structs(program, source, &aliases);
        Self {
            source,
            aliases,
            structs,
        }
    }

    fn type_name(&self, ty: &'a AstType<'a>) -> Option<String> {
        match ty {
            AstType::Simple(token) => Some(token_text(self.source, token)),
            AstType::Name(name) => Some(name_to_string(self.source, name)),
            _ => None,
        }
    }

    fn base_name(name: &str) -> &str {
        name.rsplit('\\').next().unwrap_or(name)
    }

    fn convert_type(&mut self, ty: &'a AstType<'a>) -> BridgeType {
        let mut guard = HashSet::new();
        self.convert_type_internal(ty, &mut guard, None)
    }

    fn convert_type_internal(
        &mut self,
        ty: &'a AstType<'a>,
        alias_guard: &mut HashSet<String>,
        subs: Option<&HashMap<String, BridgeType>>,
    ) -> BridgeType {
        let resolve_param = |name: &str, subs: Option<&HashMap<String, BridgeType>>| {
            subs.and_then(|map| map.get(name).cloned())
        };
        match ty {
            AstType::Simple(token) => {
                let name = token_text(self.source, token);
                if let Some(bound) = resolve_param(&name, subs) {
                    return bound;
                }
                if let Some(resolved) = self.convert_named(&name, alias_guard) {
                    return resolved;
                }
                BridgeType::Unknown
            }
            AstType::Name(name) => {
                let name_str = name_to_string(self.source, name);
                if let Some(bound) = resolve_param(&name_str, subs) {
                    return bound;
                }
                if let Some(resolved) = self.convert_named(&name_str, alias_guard) {
                    return resolved;
                }
                BridgeType::Unknown
            }
            AstType::Option(inner) => {
                let inner = self.convert_type_internal(inner, alias_guard, subs);
                BridgeType::Option {
                    inner: Some(Box::new(inner)),
                }
            }
            AstType::Nullable(inner) => {
                let inner = self.convert_type_internal(inner, alias_guard, subs);
                BridgeType::Option {
                    inner: Some(Box::new(inner)),
                }
            }
            AstType::Union(types) => {
                let mut parts = Vec::new();
                let mut saw_null = false;
                for part in *types {
                    let converted = self.convert_type_internal(part, alias_guard, subs);
                    if is_null_type(&converted) {
                        saw_null = true;
                    } else {
                        parts.push(converted);
                    }
                }
                if saw_null && parts.len() == 1 {
                    BridgeType::Option {
                        inner: Some(Box::new(parts.remove(0))),
                    }
                } else {
                    if saw_null {
                        parts.push(BridgeType::Primitive {
                            name: "null".to_string(),
                        });
                    }
                    BridgeType::Union { types: parts }
                }
            }
            AstType::Intersection(types) => {
                // Intersection types are not supported in the bridge yet; fall back to mixed.
                let _ = types;
                BridgeType::Mixed
            }
            AstType::ObjectShape(fields) => {
                let mut out = Vec::new();
                for field in *fields {
                    let name = token_text(self.source, field.name);
                    let ty = self.convert_type_internal(field.ty, alias_guard, subs);
                    out.push(BridgeField {
                        name,
                        ty,
                        optional: field.optional,
                    });
                }
                BridgeType::ObjectShape { fields: out }
            }
            AstType::Applied { base, args } => {
                let base_name = self
                    .type_name(base)
                    .unwrap_or_else(|| "unknown".to_string());
                if let Some(alias) = self.aliases.get(&base_name).cloned() {
                    if alias.params.len() == args.len() {
                        let mut param_map = HashMap::new();
                        for (idx, param) in alias.params.iter().enumerate() {
                            let arg_ty = self.convert_type_internal(&args[idx], alias_guard, subs);
                            param_map.insert(param.clone(), arg_ty);
                        }
                        if alias_guard.insert(base_name.clone()) {
                            let resolved =
                                self.convert_type_internal(alias.ty, alias_guard, Some(&param_map));
                            alias_guard.remove(&base_name);
                            return resolved;
                        }
                        return BridgeType::Mixed;
                    }
                }
                let base_id = Self::base_name(&base_name).to_ascii_lowercase();
                let mut converted_args = Vec::new();
                for arg in *args {
                    converted_args.push(self.convert_type_internal(arg, alias_guard, subs));
                }
                if base_id == "option" {
                    return BridgeType::Option {
                        inner: converted_args.get(0).cloned().map(Box::new),
                    };
                }
                if base_id == "result" {
                    let ok = converted_args.get(0).cloned().map(Box::new);
                    let err = converted_args.get(1).cloned().map(Box::new);
                    return BridgeType::Result { ok, err };
                }
                if base_id == "array" {
                    let element = converted_args.get(0).cloned().map(Box::new);
                    return BridgeType::Array { element };
                }
                BridgeType::Applied {
                    base: base_name,
                    args: converted_args,
                }
            }
        }
    }

    fn convert_named(
        &mut self,
        name: &str,
        alias_guard: &mut HashSet<String>,
    ) -> Option<BridgeType> {
        let base = Self::base_name(name).to_ascii_lowercase();
        match base.as_str() {
            "mixed" => return Some(BridgeType::Mixed),
            "int" | "float" | "bool" | "string" | "null" => {
                return Some(BridgeType::Primitive {
                    name: base.to_string(),
                });
            }
            "array" => return Some(BridgeType::Array { element: None }),
            "object" => return Some(BridgeType::Object),
            "option" => {
                return Some(BridgeType::Option { inner: None });
            }
            "result" => {
                return Some(BridgeType::Result {
                    ok: None,
                    err: None,
                });
            }
            _ => {}
        }

        if let Some(fields) = self.structs.get(name).cloned() {
            return Some(BridgeType::Struct {
                name: name.to_string(),
                fields,
            });
        }

        if let Some(alias_type) = self.aliases.get(name) {
            if !alias_type.params.is_empty() {
                return Some(BridgeType::Mixed);
            }
            if alias_guard.insert(name.to_string()) {
                let resolved = self.convert_type_internal(alias_type.ty, alias_guard, None);
                alias_guard.remove(name);
                return Some(resolved);
            }
            return Some(BridgeType::Mixed);
        }

        Some(BridgeType::Unknown)
    }
}

fn is_null_type(ty: &BridgeType) -> bool {
    matches!(ty, BridgeType::Primitive { name } if name == "null")
}

fn token_text(source: &[u8], token: &Token) -> String {
    String::from_utf8_lossy(token.text(source)).to_string()
}

fn name_to_string(source: &[u8], name: &php_rs::parser::ast::Name<'_>) -> String {
    let mut out = String::new();
    for (idx, part) in name.parts.iter().enumerate() {
        if idx > 0 {
            out.push('\\');
        }
        out.push_str(&token_text(source, part));
    }
    out
}

fn collect_aliases<'a>(
    program: &'a Program<'a>,
    source: &'a [u8],
) -> HashMap<String, TypeAliasInfo<'a>> {
    let mut out = HashMap::new();
    for stmt in program.statements.iter() {
        if let Stmt::TypeAlias {
            name,
            type_params,
            ty,
            ..
        } = stmt
        {
            let name_str = token_text(source, name);
            let params = type_params
                .iter()
                .map(|param| token_text(source, param.name))
                .collect::<Vec<_>>();
            out.insert(name_str, TypeAliasInfo { params, ty: *ty });
        }
    }
    out
}

fn collect_structs<'a>(
    program: &'a Program<'a>,
    source: &'a [u8],
    aliases: &HashMap<String, TypeAliasInfo<'a>>,
) -> HashMap<String, Vec<BridgeField>> {
    let mut out = HashMap::new();
    for stmt in program.statements.iter() {
        let Stmt::Class {
            kind,
            name,
            members,
            ..
        } = stmt
        else {
            continue;
        };
        if *kind != ClassKind::Struct {
            continue;
        }
        let struct_name = token_text(source, name);
        let mut fields = Vec::new();
        for member in members.iter() {
            match *member {
                ClassMember::Property { ty, entries, .. } => {
                    for entry in entries.iter() {
                        let field_name = token_text(source, entry.name);
                        let optional = entry.default.is_some();
                        let field_ty = ty
                            .map(|ty| {
                                let mut resolver = TypeResolver {
                                    source,
                                    aliases: aliases.clone(),
                                    structs: HashMap::new(),
                                };
                                resolver.convert_type(ty)
                            })
                            .unwrap_or(BridgeType::Mixed);
                        fields.push(BridgeField {
                            name: field_name,
                            ty: field_ty,
                            optional,
                        });
                    }
                }
                _ => {}
            }
        }
        out.insert(struct_name, fields);
    }
    out
}

#[op2]
#[serde]
pub(super) fn op_php_parse_phpx_types(
    #[string] source: String,
    #[string] file_path: String,
) -> Result<BridgeModuleTypes, deno_core::error::CoreError> {
    let trimmed = source.trim_start();
    let source_holder: std::borrow::Cow<[u8]> = if trimmed.starts_with("<?php") {
        let prefix_len = source.len() - trimmed.len();
        let without_tag = source[prefix_len + 5..].to_string();
        std::borrow::Cow::Owned(without_tag.into_bytes())
    } else {
        std::borrow::Cow::Borrowed(source.as_bytes())
    };
    let source_bytes = source_holder.as_ref();
    let arena = Bump::new();
    let lexer = Lexer::new(source_bytes);
    let path = std::path::Path::new(&file_path);
    let mut mode = detect_parser_mode(source_bytes, Some(path));
    if path
        .to_string_lossy()
        .replace('\\', "/")
        .contains("/php_modules/")
    {
        mode = ParserMode::PhpxInternal;
    }
    let mut parser = Parser::new_with_mode(lexer, &arena, mode);
    let program = parser.parse_program();
    if !program.errors.is_empty() {
        return Err(deno_core::error::CoreError::from(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "Failed to parse PHPX types for '{}': {:?}",
                file_path, program.errors
            ),
        )));
    }
    let mut resolver = TypeResolver::new(source_bytes, &program);
    let mut functions = HashMap::new();
    for stmt in program.statements.iter() {
        if let Stmt::Function {
            name,
            params,
            return_type,
            ..
        } = stmt
        {
            let fn_name = token_text(source_bytes, name);
            let mut params_out = Vec::new();
            let mut has_variadic = false;
            for param in params.iter() {
                let ty = param.ty.map(|ty| resolver.convert_type(ty));
                let required = param.default.is_none() && !param.variadic;
                let variadic = param.variadic;
                if variadic {
                    has_variadic = true;
                }
                params_out.push(BridgeParam {
                    ty,
                    required,
                    variadic,
                });
            }
            let return_type = return_type.map(|ty| resolver.convert_type(ty));
            functions.insert(
                fn_name,
                BridgeFunction {
                    params: params_out,
                    return_type,
                    variadic: has_variadic,
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
