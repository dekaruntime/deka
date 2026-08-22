use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::Path;

use bumpalo::Bump;
use php_rs::parser::ast::{ClassKind, Program, Stmt};
use php_rs::phpx::typeck::{
    EnumInfo, PrimitiveType, StructInfo, Type, TypeckFunctionInfo, TypeckProgramSummary,
    summarize_program_with_path,
};
use runtime_core::seam::{
    SeamBoundary, SeamContract, SeamDefinition, SeamEnum, SeamEnumVariant, SeamPrimitive,
    SeamRecord, SeamType,
};

use crate::compiler_api::compile_deka;
use crate::validation::format_validation_error;

pub fn extract_contract_from_file(path: impl AsRef<Path>) -> Result<SeamContract, String> {
    let path = path.as_ref();
    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    extract_contract_from_source(&source, &path.to_string_lossy())
}

pub fn extract_contract_from_source(source: &str, file_path: &str) -> Result<SeamContract, String> {
    let arena = Bump::new();
    let result = compile_deka(source, file_path, &arena);
    if !result.errors.is_empty() {
        let mut out = String::new();
        for error in &result.errors {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format_validation_error(source, file_path, error));
        }
        return Err(out);
    }

    let program = result
        .ast
        .ok_or_else(|| "PHPX source did not produce an AST".to_string())?;
    let summary =
        summarize_program_with_path(&program, source.as_bytes(), Some(Path::new(file_path)))
            .map_err(|errors| {
                errors
                    .iter()
                    .map(|err| err.to_human_readable(source.as_bytes()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })?;

    extract_contract_from_program(source, &program, &summary)
}

fn extract_contract_from_program(
    source: &str,
    program: &Program<'_>,
    summary: &TypeckProgramSummary,
) -> Result<SeamContract, String> {
    let function_name = find_boundary_function(source, program, summary)?;
    let function = summary
        .functions
        .get(&function_name)
        .ok_or_else(|| format!("boundary function '{}' was not typechecked", function_name))?;
    let (request, response) = boundary_structs(&function_name, function)?;

    let mut contract = SeamContract::new(function_name.clone(), 1);
    contract.boundaries.push(SeamBoundary {
        function: function_name,
        request: request.clone(),
        response: response.clone(),
    });

    let mut queue = VecDeque::from([Type::Struct(request), Type::Struct(response)]);
    let mut seen_records = BTreeSet::new();
    let mut seen_enums = BTreeSet::new();
    let mut definitions = Vec::new();

    while let Some(ty) = queue.pop_front() {
        match ty {
            Type::Struct(name) => {
                if !seen_records.insert(name.clone()) {
                    continue;
                }
                let info = summary
                    .structs
                    .get(&name)
                    .ok_or_else(|| format!("unknown boundary struct '{}'", name))?;
                let record = record_definition(&name, info, summary, &mut queue)?;
                definitions.push(SeamDefinition::Record(record));
            }
            Type::Enum(name) => {
                if !seen_enums.insert(name.clone()) {
                    continue;
                }
                let info = summary
                    .enums
                    .get(&name)
                    .ok_or_else(|| format!("unknown boundary enum '{}'", name))?;
                let enum_def = enum_definition(&name, info, summary, &mut queue)?;
                definitions.push(SeamDefinition::Enum(enum_def));
            }
            _ => enqueue_references(&ty, &mut queue),
        }
    }

    contract.definitions = definitions;
    Ok(contract)
}

fn find_boundary_function(
    source: &str,
    program: &Program<'_>,
    summary: &TypeckProgramSummary,
) -> Result<String, String> {
    let mut exported = Vec::new();
    let mut function_names = Vec::new();
    for stmt in program.statements.iter() {
        let Stmt::Function { name, .. } = &**stmt else {
            continue;
        };
        let function_name = span_text(source.as_bytes(), name.span).to_string();
        if is_exported_function_line(source, name.span.start) {
            exported.push(function_name.clone());
        }
        function_names.push(function_name);
    }

    if exported.len() == 1 {
        return Ok(exported.remove(0));
    }
    if exported.len() > 1 {
        return Err("found multiple exported functions; expected one handler boundary".to_string());
    }
    if summary.functions.contains_key("handle") {
        return Ok("handle".to_string());
    }

    let candidates = function_names
        .into_iter()
        .filter(|name| {
            summary
                .functions
                .get(name)
                .and_then(|function| boundary_structs(name, function).ok())
                .is_some()
        })
        .collect::<Vec<_>>();

    match candidates.as_slice() {
        [name] => Ok(name.clone()),
        [] => {
            Err("no exported handler function with struct request/response types found".to_string())
        }
        _ => Err("multiple handler-shaped functions found; mark one with export".to_string()),
    }
}

fn is_exported_function_line(source: &str, name_start: usize) -> bool {
    let line_start = source[..name_start]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let prefix = &source[line_start..name_start];
    prefix.contains("export") && prefix.contains("function")
}

fn boundary_structs(
    function_name: &str,
    function: &TypeckFunctionInfo,
) -> Result<(String, String), String> {
    let request = function
        .params
        .first()
        .and_then(|param| param.ty.as_ref())
        .ok_or_else(|| {
            format!(
                "boundary function '{}' needs a typed request parameter",
                function_name
            )
        })?;
    let Type::Struct(request) = request else {
        return Err(format!(
            "boundary function '{}' request must be a struct, got {}",
            function_name,
            request.name()
        ));
    };
    let response = function.return_type.as_ref().ok_or_else(|| {
        format!(
            "boundary function '{}' needs a typed response return",
            function_name
        )
    })?;
    let Type::Struct(response) = response else {
        return Err(format!(
            "boundary function '{}' response must be a struct, got {}",
            function_name,
            response.name()
        ));
    };
    Ok((request.clone(), response.clone()))
}

fn record_definition(
    name: &str,
    info: &StructInfo,
    summary: &TypeckProgramSummary,
    queue: &mut VecDeque<Type>,
) -> Result<SeamRecord, String> {
    let mut fields = std::collections::BTreeMap::new();
    for (field, ty) in &info.fields {
        fields.insert(field.clone(), seam_type(ty, summary, queue)?);
    }
    Ok(SeamRecord {
        name: name.to_string(),
        fields,
    })
}

fn enum_definition(
    name: &str,
    info: &EnumInfo,
    summary: &TypeckProgramSummary,
    queue: &mut VecDeque<Type>,
) -> Result<SeamEnum, String> {
    let mut variants = Vec::new();
    for (variant_name, case) in &info.cases {
        let mut fields = std::collections::BTreeMap::new();
        for param in &case.params {
            let ty = param.ty.as_ref().ok_or_else(|| {
                format!(
                    "enum case '{}::{}' payload field '{}' needs a type",
                    name, variant_name, param.name
                )
            })?;
            fields.insert(param.name.clone(), seam_type(ty, summary, queue)?);
        }
        variants.push(SeamEnumVariant {
            name: variant_name.clone(),
            fields,
        });
    }
    Ok(SeamEnum {
        name: name.to_string(),
        variants,
    })
}

fn seam_type(
    ty: &Type,
    summary: &TypeckProgramSummary,
    queue: &mut VecDeque<Type>,
) -> Result<SeamType, String> {
    match ty {
        Type::Primitive(PrimitiveType::String) => Ok(SeamType::Primitive {
            name: SeamPrimitive::String,
        }),
        Type::Primitive(PrimitiveType::Number) => Ok(SeamType::Primitive {
            name: SeamPrimitive::Int,
        }),
        Type::Primitive(PrimitiveType::Bool) => Ok(SeamType::Primitive {
            name: SeamPrimitive::Bool,
        }),
        Type::Struct(name) => {
            queue.push_back(Type::Struct(name.clone()));
            Ok(SeamType::Named { name: name.clone() })
        }
        Type::Enum(name) => {
            queue.push_back(Type::Enum(name.clone()));
            Ok(SeamType::Named { name: name.clone() })
        }
        Type::Applied { base, args } if base.eq_ignore_ascii_case("Option") && args.len() == 1 => {
            Ok(SeamType::Option {
                item: Box::new(seam_type(&args[0], summary, queue)?),
            })
        }
        Type::Applied { base, args } if base.eq_ignore_ascii_case("array") && args.len() == 1 => {
            Ok(SeamType::List {
                item: Box::new(seam_type(&args[0], summary, queue)?),
            })
        }
        Type::Applied { base, args } if base.eq_ignore_ascii_case("Map") && args.len() == 2 => {
            Ok(SeamType::Map {
                key: Box::new(seam_type(&args[0], summary, queue)?),
                value: Box::new(seam_type(&args[1], summary, queue)?),
            })
        }
        other => {
            enqueue_references(other, queue);
            let known_named = match other {
                Type::Struct(name) => summary.structs.contains_key(name),
                Type::Enum(name) => summary.enums.contains_key(name),
                _ => false,
            };
            if known_named {
                Ok(SeamType::Named { name: other.name() })
            } else {
                Err(format!("unsupported seam boundary type '{}'", other.name()))
            }
        }
    }
}

fn enqueue_references(ty: &Type, queue: &mut VecDeque<Type>) {
    match ty {
        Type::Struct(name) => queue.push_back(Type::Struct(name.clone())),
        Type::Enum(name) => queue.push_back(Type::Enum(name.clone())),
        Type::Applied { args, .. } | Type::Union(args) => {
            for arg in args {
                enqueue_references(arg, queue);
            }
        }
        Type::EnumCase { enum_name, .. } => queue.push_back(Type::Enum(enum_name.clone())),
        _ => {}
    }
}

fn span_text(source: &[u8], span: php_rs::parser::span::Span) -> &str {
    std::str::from_utf8(&source[span.start..span.end]).unwrap_or("")
}
