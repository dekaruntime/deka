use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;

use seam_ir::{SeamContract, SeamDefinition, SeamPrimitive, SeamRecord, SeamType};
use swc_common::{FileName, SourceMap, sync::Lrc};
use swc_ecma_ast::{
    Decl, EsVersion, ExportSpecifier, Expr, Lit, Module, ModuleDecl, ModuleExportName, ModuleItem,
    Stmt, TsEntityName, TsInterfaceDecl, TsKeywordTypeKind, TsType, TsTypeAliasDecl, TsTypeElement,
    TsUnionOrIntersectionType,
};
use swc_ecma_parser::{Parser, StringInput, Syntax, TsSyntax, lexer::Lexer};

pub fn extract_contract_from_file(path: impl AsRef<Path>) -> Result<SeamContract, String> {
    let path = path.as_ref();
    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    extract_contract_from_source(&source, &path.to_string_lossy())
}

pub fn extract_contract_from_source(source: &str, file_path: &str) -> Result<SeamContract, String> {
    let module = parse_module(source, file_path)?;
    extract_contract_from_module(&module, contract_name(file_path))
}

fn parse_module(source: &str, file_path: &str) -> Result<Module, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(
        FileName::Custom(file_path.to_string()).into(),
        source.to_string(),
    );
    let lower = file_path.to_ascii_lowercase();
    let syntax = Syntax::Typescript(TsSyntax {
        tsx: lower.ends_with(".tsx"),
        decorators: true,
        dts: lower.ends_with(".d.ts"),
        no_early_errors: false,
        disallow_ambiguous_jsx_like: false,
    });
    let lexer = Lexer::new(syntax, EsVersion::Es2022, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let module = parser
        .parse_module()
        .map_err(|err| format!("failed to parse TypeScript '{file_path}': {:?}", err.kind()))?;
    let errors = parser.take_errors();
    if errors.is_empty() {
        Ok(module)
    } else {
        Err(format!(
            "failed to parse TypeScript '{file_path}': {:?}",
            errors[0].kind()
        ))
    }
}

fn contract_name(file_path: &str) -> String {
    Path::new(file_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("typescript")
        .to_string()
}

#[derive(Clone, Copy)]
enum TypeDecl<'a> {
    Interface(&'a TsInterfaceDecl),
    Alias(&'a TsTypeAliasDecl),
}

fn extract_contract_from_module(module: &Module, name: String) -> Result<SeamContract, String> {
    let mut declarations = BTreeMap::new();
    let mut exported = BTreeSet::new();

    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(decl)) => {
                collect_decl(decl, false, &mut declarations, &mut exported);
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                collect_decl(&export.decl, true, &mut declarations, &mut exported);
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(named)) if named.src.is_none() => {
                for specifier in &named.specifiers {
                    if let ExportSpecifier::Named(named) = specifier
                        && let Some(name) = module_export_name(&named.orig)
                    {
                        exported.insert(name);
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default)) => {
                if let swc_ecma_ast::DefaultDecl::TsInterfaceDecl(interface) = &default.decl {
                    let name = interface.id.sym.to_string();
                    declarations.insert(name.clone(), TypeDecl::Interface(interface));
                    exported.insert(name);
                }
            }
            _ => {}
        }
    }

    if exported.is_empty() {
        return Err("no exported TypeScript interfaces or object type aliases found".to_string());
    }

    let mut definitions = BTreeMap::new();
    let mut queue = VecDeque::from_iter(exported);
    while let Some(type_name) = queue.pop_front() {
        if definitions.contains_key(&type_name) {
            continue;
        }
        let Some(decl) = declarations.get(&type_name).copied() else {
            continue;
        };
        let mut references = BTreeSet::new();
        let fields = fields_for_decl(
            &type_name,
            decl,
            &declarations,
            &mut BTreeSet::new(),
            &mut references,
        )?;
        definitions.insert(
            type_name.clone(),
            SeamDefinition::Record(SeamRecord {
                name: type_name,
                fields,
            }),
        );
        queue.extend(
            references
                .into_iter()
                .filter(|reference| declarations.contains_key(reference)),
        );
    }

    if definitions.is_empty() {
        return Err("no exported TypeScript interfaces or object type aliases found".to_string());
    }

    let mut contract = SeamContract::new(name, 1);
    contract.definitions = definitions.into_values().collect();
    Ok(contract)
}

fn collect_decl<'a>(
    decl: &'a Decl,
    is_exported: bool,
    declarations: &mut BTreeMap<String, TypeDecl<'a>>,
    exported: &mut BTreeSet<String>,
) {
    let entry = match decl {
        Decl::TsInterface(interface) => {
            Some((interface.id.sym.to_string(), TypeDecl::Interface(interface)))
        }
        Decl::TsTypeAlias(alias) => Some((alias.id.sym.to_string(), TypeDecl::Alias(alias))),
        _ => None,
    };
    if let Some((name, decl)) = entry {
        declarations.insert(name.clone(), decl);
        if is_exported {
            exported.insert(name);
        }
    }
}

fn fields_for_decl(
    name: &str,
    decl: TypeDecl<'_>,
    declarations: &BTreeMap<String, TypeDecl<'_>>,
    visiting: &mut BTreeSet<String>,
    references: &mut BTreeSet<String>,
) -> Result<BTreeMap<String, SeamType>, String> {
    if !visiting.insert(name.to_string()) {
        return Err(format!(
            "cyclic TypeScript interface inheritance at '{name}'"
        ));
    }

    let result = match decl {
        TypeDecl::Interface(interface) => {
            let mut fields = BTreeMap::new();
            for parent in &interface.extends {
                let parent_name = expr_name(&parent.expr).ok_or_else(|| {
                    format!("interface '{name}' has an unsupported extends expression")
                })?;
                let parent_decl = declarations.get(&parent_name).copied().ok_or_else(|| {
                    format!("interface '{name}' extends unknown local type '{parent_name}'")
                })?;
                fields.extend(fields_for_decl(
                    &parent_name,
                    parent_decl,
                    declarations,
                    visiting,
                    references,
                )?);
            }
            fields.extend(fields_from_members(name, &interface.body.body, references)?);
            Ok(fields)
        }
        TypeDecl::Alias(alias) => match alias.type_ann.as_ref() {
            TsType::TsTypeLit(literal) => fields_from_members(name, &literal.members, references),
            TsType::TsParenthesizedType(parenthesized) => match parenthesized.type_ann.as_ref() {
                TsType::TsTypeLit(literal) => {
                    fields_from_members(name, &literal.members, references)
                }
                _ => Err(format!(
                    "exported type alias '{name}' must be an object type literal"
                )),
            },
            _ => Err(format!(
                "exported type alias '{name}' must be an object type literal"
            )),
        },
    };
    visiting.remove(name);
    result
}

fn fields_from_members(
    owner: &str,
    members: &[TsTypeElement],
    references: &mut BTreeSet<String>,
) -> Result<BTreeMap<String, SeamType>, String> {
    let mut fields = BTreeMap::new();
    for member in members {
        let TsTypeElement::TsPropertySignature(property) = member else {
            return Err(format!(
                "type '{owner}' contains a non-property member unsupported by seam contracts"
            ));
        };
        let field_name = property_name(&property.key)
            .ok_or_else(|| format!("type '{owner}' contains an unsupported property name"))?;
        let annotation = property.type_ann.as_ref().ok_or_else(|| {
            format!("field '{owner}.{field_name}' needs a TypeScript type annotation")
        })?;
        let mut field_type = seam_type(&annotation.type_ann, references)
            .map_err(|err| format!("field '{owner}.{field_name}': {err}"))?;
        if property.optional && !matches!(field_type, SeamType::Option { .. }) {
            field_type = SeamType::Option {
                item: Box::new(field_type),
            };
        }
        fields.insert(field_name, field_type);
    }
    Ok(fields)
}

fn seam_type(ty: &TsType, references: &mut BTreeSet<String>) -> Result<SeamType, String> {
    match ty {
        TsType::TsKeywordType(keyword) => match keyword.kind {
            TsKeywordTypeKind::TsStringKeyword => Ok(primitive(SeamPrimitive::String)),
            TsKeywordTypeKind::TsNumberKeyword | TsKeywordTypeKind::TsBigIntKeyword => {
                Ok(primitive(SeamPrimitive::Int))
            }
            TsKeywordTypeKind::TsBooleanKeyword => Ok(primitive(SeamPrimitive::Bool)),
            // Seam IR has no dynamic value type. Match the Rust extractor's
            // serde_json::Value convention so Record<string, unknown> agrees
            // across the Zega boundary.
            TsKeywordTypeKind::TsAnyKeyword
            | TsKeywordTypeKind::TsUnknownKeyword
            | TsKeywordTypeKind::TsObjectKeyword => Ok(primitive(SeamPrimitive::String)),
            other => Err(format!("unsupported TypeScript keyword type {other:?}")),
        },
        TsType::TsArrayType(array) => Ok(SeamType::List {
            item: Box::new(seam_type(&array.elem_type, references)?),
        }),
        TsType::TsTypeRef(reference) => {
            let name = entity_name(&reference.type_name)
                .ok_or_else(|| "qualified type names are unsupported".to_string())?;
            let params = reference
                .type_params
                .as_ref()
                .map(|params| params.params.as_slice())
                .unwrap_or_default();
            match (name.as_str(), params) {
                ("Array" | "ReadonlyArray", [item]) => Ok(SeamType::List {
                    item: Box::new(seam_type(item, references)?),
                }),
                ("Record" | "Map", [key, value]) => Ok(SeamType::Map {
                    key: Box::new(seam_type(key, references)?),
                    value: Box::new(seam_type(value, references)?),
                }),
                ("Uint8Array", []) => Ok(primitive(SeamPrimitive::Bytes)),
                (_, []) => {
                    references.insert(name.clone());
                    Ok(SeamType::Named { name })
                }
                _ => Err(format!("unsupported generic type '{name}'")),
            }
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            let concrete = union
                .types
                .iter()
                .filter(|ty| !is_nullish(ty.as_ref()))
                .collect::<Vec<_>>();
            let nullish_count = union.types.len() - concrete.len();
            match (concrete.as_slice(), nullish_count) {
                ([item], count) if count > 0 => Ok(SeamType::Option {
                    item: Box::new(seam_type(item, references)?),
                }),
                _ => Err("only nullable unions such as 'string | null' are supported".to_string()),
            }
        }
        TsType::TsParenthesizedType(parenthesized) => {
            seam_type(&parenthesized.type_ann, references)
        }
        TsType::TsTypeOperator(operator) => seam_type(&operator.type_ann, references),
        TsType::TsOptionalType(optional) => Ok(SeamType::Option {
            item: Box::new(seam_type(&optional.type_ann, references)?),
        }),
        TsType::TsLitType(literal) => match &literal.lit {
            swc_ecma_ast::TsLit::Str(_) => Ok(primitive(SeamPrimitive::String)),
            swc_ecma_ast::TsLit::Number(_) | swc_ecma_ast::TsLit::BigInt(_) => {
                Ok(primitive(SeamPrimitive::Int))
            }
            swc_ecma_ast::TsLit::Bool(_) => Ok(primitive(SeamPrimitive::Bool)),
            _ => Err("unsupported TypeScript literal type".to_string()),
        },
        _ => Err("unsupported TypeScript type shape".to_string()),
    }
}

fn is_nullish(ty: &TsType) -> bool {
    matches!(
        ty,
        TsType::TsKeywordType(keyword)
            if matches!(
                keyword.kind,
                TsKeywordTypeKind::TsNullKeyword | TsKeywordTypeKind::TsUndefinedKeyword
            )
    )
}

fn primitive(name: SeamPrimitive) -> SeamType {
    SeamType::Primitive { name }
}

fn entity_name(name: &TsEntityName) -> Option<String> {
    match name {
        TsEntityName::Ident(ident) => Some(ident.sym.to_string()),
        TsEntityName::TsQualifiedName(_) => None,
    }
}

fn expr_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(ident) => Some(ident.sym.to_string()),
        _ => None,
    }
}

fn property_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(ident) => Some(ident.sym.to_string()),
        Expr::Lit(Lit::Str(value)) => value.value.as_str().map(ToString::to_string),
        _ => None,
    }
}

fn module_export_name(name: &ModuleExportName) -> Option<String> {
    match name {
        ModuleExportName::Ident(ident) => Some(ident.sym.to_string()),
        ModuleExportName::Str(value) => value.value.as_str().map(ToString::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_exported_records_and_referenced_types() {
        let source = r#"
            interface Detail {
                tags: readonly string[];
            }

            export interface KvGetResponse extends Detail {
                ok: boolean;
                value: string | null;
                error?: string;
                attempts: number;
                metadata: Record<string, string>;
            }

            type KvSetResponse = {
                ok: boolean;
            };
            export type { KvSetResponse };
        "#;

        let contract = extract_contract_from_source(source, "clients/zega.ts").unwrap();
        assert_eq!(contract.name, "zega");
        assert!(contract.boundaries.is_empty());
        assert_eq!(contract.definitions.len(), 2);

        let json = serde_json::to_value(&contract).unwrap();
        assert_eq!(json["definitions"][0]["name"], "KvGetResponse");
        assert_eq!(json["definitions"][0]["fields"]["attempts"]["name"], "Int");
        assert_eq!(json["definitions"][0]["fields"]["error"]["kind"], "option");
        assert_eq!(json["definitions"][0]["fields"]["metadata"]["kind"], "map");
        assert_eq!(
            json["definitions"][0]["fields"]["metadata"]["value"]["name"],
            "String"
        );
        assert_eq!(json["definitions"][0]["fields"]["tags"]["kind"], "list");
        assert_eq!(json["definitions"][0]["fields"]["value"]["kind"], "option");
    }

    #[test]
    fn maps_dynamic_map_values_to_the_ir_string_convention() {
        let source = r#"
            export interface CqlQueryResponse {
                ok: boolean;
                rows: Record<string, unknown>[];
                count: number;
                error?: string;
            }
        "#;

        let contract = extract_contract_from_source(source, "zega.ts").unwrap();
        let json = serde_json::to_value(&contract).unwrap();
        assert_eq!(
            json["definitions"][0]["fields"]["rows"]["item"]["value"]["name"],
            "String"
        );
    }

    #[test]
    fn includes_referenced_local_record_definitions() {
        let source = r#"
            interface Detail { message: string }
            export interface Response { detail: Detail }
        "#;
        let contract = extract_contract_from_source(source, "zega.ts").unwrap();
        assert_eq!(contract.definitions.len(), 2);
        assert!(contract.definitions.iter().any(|definition| {
            matches!(definition, SeamDefinition::Record(record) if record.name == "Detail")
        }));
    }

    #[test]
    fn rejects_non_nullable_unions() {
        let error = extract_contract_from_source(
            "export interface Response { value: string | number }",
            "zega.ts",
        )
        .unwrap_err();
        assert!(error.contains("only nullable unions"));
    }

    #[test]
    fn rejects_files_without_exported_contract_types() {
        let error = extract_contract_from_source("interface Response { ok: boolean }", "zega.ts")
            .unwrap_err();
        assert!(error.contains("no exported TypeScript interfaces"));
    }
}
