use super::security::{enforce_read, enforce_wasm};
use super::*;

#[derive(serde::Serialize)]
pub(super) struct WitSchema {
    world: String,
    functions: Vec<WitFunction>,
    interfaces: Vec<WitInterface>,
}

#[derive(serde::Serialize)]
pub(super) struct WitInterface {
    name: String,
    functions: Vec<WitFunction>,
}

#[derive(serde::Serialize)]
pub(super) struct WitFunction {
    name: String,
    params: Vec<WitParam>,
    result: Option<WitType>,
}

#[derive(serde::Serialize)]
pub(super) struct WitParam {
    name: String,
    #[serde(rename = "type")]
    ty: WitType,
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum WitType {
    Bool,
    U8,
    U16,
    U32,
    U64,
    S8,
    S16,
    S32,
    S64,
    F32,
    F64,
    Char,
    String,
    List {
        element: Box<WitType>,
    },
    Record {
        fields: Vec<WitField>,
    },
    Tuple {
        items: Vec<WitType>,
    },
    Option {
        some: Box<WitType>,
    },
    Result {
        ok: Option<Box<WitType>>,
        err: Option<Box<WitType>>,
    },
    Enum {
        cases: Vec<String>,
    },
    Flags {
        flags: Vec<String>,
    },
    Variant {
        cases: Vec<WitVariantCase>,
    },
    Resource,
    Unsupported {
        detail: String,
    },
}

#[derive(serde::Serialize)]
pub(super) struct WitField {
    name: String,
    #[serde(rename = "type")]
    ty: WitType,
}

#[derive(serde::Serialize)]
pub(super) struct WitVariantCase {
    name: String,
    #[serde(rename = "type")]
    ty: Option<WitType>,
}

#[op2]
#[serde]
pub(super) fn op_php_parse_wit(
    #[string] path: String,
    #[string] world: String,
) -> Result<WitSchema, deno_core::error::CoreError> {
    enforce_wasm(Some(&path))?;
    enforce_read(Some(&path))?;
    let mut resolve = Resolve::default();
    let (package_id, _) = resolve.push_path(&path).map_err(|err| {
        deno_core::error::CoreError::from(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to parse WIT '{}': {}", path, err),
        ))
    })?;

    let world_id = if world.trim().is_empty() {
        let package = &resolve.packages[package_id];
        if package.worlds.len() != 1 {
            return Err(deno_core::error::CoreError::from(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!(
                    "WIT package has {} worlds; set deka.json.world",
                    package.worlds.len()
                ),
            )));
        }
        *package.worlds.values().next().expect("worlds len checked")
    } else {
        resolve
            .select_world(package_id, Some(world.trim()))
            .map_err(|err| {
                deno_core::error::CoreError::from(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to select world '{}': {}", world, err),
                ))
            })?
    };

    let world = &resolve.worlds[world_id];
    let mut functions = Vec::new();
    let mut interfaces = Vec::new();

    for (key, item) in world.exports.iter() {
        match item {
            WorldItem::Function(func) => {
                let sig = build_function(&resolve, func);
                let name = world_key_name(key);
                functions.push(WitFunction { name, ..sig });
            }
            WorldItem::Interface { id, .. } => {
                let iface = &resolve.interfaces[*id];
                let iface_name = world_key_name(key);
                let mut iface_functions = Vec::new();
                for func in iface.functions.values() {
                    let sig = build_function(&resolve, func);
                    iface_functions.push(sig);
                }
                interfaces.push(WitInterface {
                    name: iface_name,
                    functions: iface_functions,
                });
            }
            WorldItem::Type(_) => {}
        }
    }

    Ok(WitSchema {
        world: world.name.clone(),
        functions,
        interfaces,
    })
}

fn world_key_name(key: &WorldKey) -> String {
    match key {
        WorldKey::Name(name) => name.clone(),
        WorldKey::Interface(id) => format!("interface_{}", id.index()),
    }
}

fn build_function(resolve: &Resolve, func: &wit_parser::Function) -> WitFunction {
    let params = func
        .params
        .iter()
        .map(|(name, ty)| WitParam {
            name: name.clone(),
            ty: resolve_type(resolve, ty, &mut HashSet::new()),
        })
        .collect::<Vec<_>>();

    let result = match &func.results {
        Results::Anon(ty) => Some(resolve_type(resolve, ty, &mut HashSet::new())),
        Results::Named(named) => {
            if named.is_empty() {
                None
            } else if named.len() == 1 {
                Some(resolve_type(resolve, &named[0].1, &mut HashSet::new()))
            } else {
                let fields = named
                    .iter()
                    .map(|(name, ty)| WitField {
                        name: name.clone(),
                        ty: resolve_type(resolve, ty, &mut HashSet::new()),
                    })
                    .collect();
                Some(WitType::Record { fields })
            }
        }
    };

    WitFunction {
        name: func.name.clone(),
        params,
        result,
    }
}

fn resolve_type(resolve: &Resolve, ty: &Type, visiting: &mut HashSet<TypeId>) -> WitType {
    match ty {
        Type::Bool => WitType::Bool,
        Type::U8 => WitType::U8,
        Type::U16 => WitType::U16,
        Type::U32 => WitType::U32,
        Type::U64 => WitType::U64,
        Type::S8 => WitType::S8,
        Type::S16 => WitType::S16,
        Type::S32 => WitType::S32,
        Type::S64 => WitType::S64,
        Type::F32 => WitType::F32,
        Type::F64 => WitType::F64,
        Type::Char => WitType::Char,
        Type::String => WitType::String,
        Type::Id(id) => resolve_type_id(resolve, *id, visiting),
    }
}

fn resolve_type_id(resolve: &Resolve, id: TypeId, visiting: &mut HashSet<TypeId>) -> WitType {
    if !visiting.insert(id) {
        return WitType::Unsupported {
            detail: "recursive type".to_string(),
        };
    }
    let ty = &resolve.types[id];
    let out = match &ty.kind {
        TypeDefKind::Record(record) => WitType::Record {
            fields: record
                .fields
                .iter()
                .map(|field| WitField {
                    name: field.name.clone(),
                    ty: resolve_type(resolve, &field.ty, visiting),
                })
                .collect(),
        },
        TypeDefKind::Tuple(tuple) => WitType::Tuple {
            items: tuple
                .types
                .iter()
                .map(|ty| resolve_type(resolve, ty, visiting))
                .collect(),
        },
        TypeDefKind::Option(inner) => WitType::Option {
            some: Box::new(resolve_type(resolve, inner, visiting)),
        },
        TypeDefKind::Result(res) => WitType::Result {
            ok: res
                .ok
                .as_ref()
                .map(|ty| Box::new(resolve_type(resolve, ty, visiting))),
            err: res
                .err
                .as_ref()
                .map(|ty| Box::new(resolve_type(resolve, ty, visiting))),
        },
        TypeDefKind::List(inner) => WitType::List {
            element: Box::new(resolve_type(resolve, inner, visiting)),
        },
        TypeDefKind::Enum(enm) => WitType::Enum {
            cases: enm.cases.iter().map(|c| c.name.clone()).collect(),
        },
        TypeDefKind::Flags(flags) => WitType::Flags {
            flags: flags.flags.iter().map(|f| f.name.clone()).collect(),
        },
        TypeDefKind::Variant(variant) => WitType::Variant {
            cases: variant
                .cases
                .iter()
                .map(|case| WitVariantCase {
                    name: case.name.clone(),
                    ty: case
                        .ty
                        .as_ref()
                        .map(|ty| resolve_type(resolve, ty, visiting)),
                })
                .collect(),
        },
        TypeDefKind::Type(inner) => resolve_type(resolve, inner, visiting),
        TypeDefKind::Resource => WitType::Resource,
        TypeDefKind::Handle(_)
        | TypeDefKind::Future(_)
        | TypeDefKind::Stream(_)
        | TypeDefKind::Unknown => WitType::Unsupported {
            detail: ty.kind.as_str().to_string(),
        },
    };
    visiting.remove(&id);
    out
}
