use super::*;

pub(crate) fn completion_for_annotation(
    source: &str,
    offset: usize,
) -> Option<Vec<CompletionItem>> {
    let line_start = source[..offset.min(source.len())]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let prefix = &source[line_start..offset.min(source.len())];
    let at = prefix.rfind('@')?;
    let typed = &prefix[at + 1..];
    if typed
        .chars()
        .any(|ch| !(ch == '_' || ch.is_ascii_alphanumeric()))
    {
        return None;
    }

    let mut items = Vec::new();
    for (name, detail) in annotation_catalog() {
        if !typed.is_empty() && !name.starts_with(typed) {
            continue;
        }
        let (insert_text, insert_text_format) = match name {
            "index" => (
                Some("index(${1:\"idx_name\"})".to_string()),
                Some(InsertTextFormat::SNIPPET),
            ),
            "map" => (
                Some("map(${1:\"column_name\"})".to_string()),
                Some(InsertTextFormat::SNIPPET),
            ),
            "default" => (
                Some("default(${1:value})".to_string()),
                Some(InsertTextFormat::SNIPPET),
            ),
            "relation" => (
                Some("relation(${1:\"hasMany\"}, ${2:\"Model\"}, ${3:\"foreignKey\"})".to_string()),
                Some(InsertTextFormat::SNIPPET),
            ),
            _ => (Some(name.to_string()), None),
        };
        items.push(CompletionItem {
            label: format!("@{}", name),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some("struct field annotation".to_string()),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: detail.to_string(),
            })),
            insert_text,
            insert_text_format,
            ..CompletionItem::default()
        });
    }
    Some(items)
}

pub(crate) fn annotation_catalog() -> Vec<(&'static str, &'static str)> {
    vec![
        ("id", "Primary key marker. No arguments."),
        ("unique", "Unique constraint marker. No arguments."),
        (
            "autoIncrement",
            "Auto-increment marker. Requires an `int` field.",
        ),
        (
            "index",
            "Secondary index marker. Optional string index name argument.",
        ),
        (
            "map",
            "Column mapping marker. Requires a string column name.",
        ),
        ("default", "Default value marker. Requires one argument."),
        (
            "relation",
            "Relation marker. Requires three string arguments: relation kind (`hasMany|belongsTo|hasOne`), model name, foreign key.",
        ),
    ]
}

pub(crate) fn completion_for_dot(
    index: &SymbolIndex,
    source: &[u8],
    offset: usize,
) -> Option<Vec<CompletionItem>> {
    if offset == 0 || source.get(offset - 1) != Some(&b'.') {
        return None;
    }
    let name = word_before_dot(source, offset)?;
    let ty = index.var_type_at(offset, &name)?;
    let fields = index.fields_for_type(&ty)?;
    if fields.is_empty() {
        return None;
    }
    let mut items = Vec::new();
    for field in fields {
        items.push(CompletionItem {
            label: field,
            kind: Some(CompletionItemKind::FIELD),
            ..CompletionItem::default()
        });
    }
    Some(items)
}

pub(crate) fn completion_for_jsx_props(
    index: &SymbolIndex,
    source: &[u8],
    offset: usize,
) -> Option<Vec<CompletionItem>> {
    if offset == 0 || offset > source.len() {
        return None;
    }

    let prefix = std::str::from_utf8(&source[..offset]).ok()?;
    let lt = prefix.rfind('<')?;
    let open = &prefix[lt + 1..];
    if open.starts_with('/') || open.contains('>') {
        return None;
    }

    let mut component = String::new();
    for ch in open.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '\\' {
            component.push(ch);
        } else {
            break;
        }
    }
    if component.is_empty() {
        return None;
    }
    let simple = component.rsplit('\\').next().unwrap_or(&component);
    if !simple
        .chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
    {
        return None;
    }

    let expected = index.component_prop_fields(simple)?;
    if expected.is_empty() {
        return None;
    }

    let attrs_slice = &open[simple.len()..];
    let typed_prefix = attrs_slice
        .rsplit(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim_start_matches('{')
        .trim_start_matches('/')
        .split('=')
        .next()
        .unwrap_or("")
        .trim();

    let mut used = std::collections::HashSet::new();
    for token in attrs_slice.split_whitespace() {
        let name = token
            .trim_start_matches('{')
            .trim_start_matches('/')
            .split('=')
            .next()
            .unwrap_or("")
            .trim();
        if !name.is_empty() {
            used.insert(name.to_string());
        }
    }

    let mut items = Vec::new();
    for (field, field_ty) in expected {
        if used.contains(&field) {
            continue;
        }
        if !typed_prefix.is_empty() && !field.starts_with(typed_prefix) {
            continue;
        }
        items.push(CompletionItem {
            label: field.clone(),
            kind: Some(CompletionItemKind::FIELD),
            detail: field_ty.clone().map(|ty| format!("{}: {}", field, ty)),
            documentation: field_ty.as_ref().map(|ty| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("`{}` expects type `{}`", field, ty),
                })
            }),
            insert_text: Some(format!(
                "{}={}",
                field,
                jsx_attr_default_value(field_ty.as_deref())
            )),
            ..CompletionItem::default()
        });
    }
    if items.is_empty() {
        return None;
    }
    Some(items)
}

pub(crate) fn jsx_attr_default_value(ty: Option<&str>) -> &'static str {
    let Some(ty) = ty else {
        return "\"\"";
    };
    let ty = ty.trim();
    let core = if let Some(inner) = ty.strip_prefix("Option<").and_then(|v| v.strip_suffix('>')) {
        inner.trim()
    } else if let Some(inner) = ty.strip_prefix("Result<").and_then(|v| v.strip_suffix('>')) {
        inner.split(',').next().unwrap_or(inner).trim()
    } else {
        ty
    };

    match core {
        "int" | "float" => "{0}",
        "bool" => "{false}",
        _ => "\"\"",
    }
}

pub(crate) fn builtin_completion_items() -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for name in [
        "Option", "Result", "Promise", "Object", "array", "int", "string", "bool", "float",
    ] {
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::TYPE_PARAMETER),
            ..CompletionItem::default()
        });
    }
    items
}

pub(crate) fn stdlib_completion_items() -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for name in [
        "panic",
        "is_valid_element",
        "create_root",
        "readFile",
        "readFileSync",
        "writeFile",
        "writeFileSync",
        "connect",
        "connectSync",
        "query",
        "querySync",
        "queryOne",
        "queryOneSync",
        "open",
        "openSync",
        "openHandle",
        "openHandleSync",
        "exec",
        "execSync",
        "begin",
        "beginSync",
        "commit",
        "commitSync",
        "rollback",
        "rollbackSync",
        "close",
        "closeSync",
        "read",
        "readSync",
        "readExact",
        "readExactSync",
        "write",
        "writeSync",
        "setDeadline",
        "setDeadlineSync",
    ] {
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            ..CompletionItem::default()
        });
    }
    items
}

pub(crate) fn snippet_completion_items() -> Vec<CompletionItem> {
    vec![
        CompletionItem {
            label: "snippet:function".to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            insert_text: Some("function ${1:name}(${2:$arg}: ${3:string}): ${4:string} {\n    ${5:return ''}\n}".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            detail: Some("PHPX function template".to_string()),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "snippet:async-function".to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            insert_text: Some(
                "async function ${1:name}(${2:$arg}: Promise<${3:string}>): Promise<${3:string}> {\n    return await ${2:$arg}\n}"
                    .to_string(),
            ),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            detail: Some("PHPX async function template".to_string()),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "snippet:struct".to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            insert_text: Some("struct ${1:Name} {\n    $${2:field}: ${3:string}\n}".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            detail: Some("PHPX struct template".to_string()),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "snippet:import".to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            insert_text: Some("import { ${1:symbol} } from '${2:module}'".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            detail: Some("PHPX import template".to_string()),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "snippet:component".to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            insert_text: Some("function ${1:Component}($props: Object<{ ${2:message}: string }>) {\n    return <div>{$props.${2:message}}</div>\n}".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            detail: Some("PHPX JSX component template".to_string()),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "snippet:frontmatter".to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            insert_text: Some("---\nimport { ${1:Component} } from '${2:module}'\n\n$${3:data} = ${4:null}\n---\n<${1:Component} />\n".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            detail: Some("PHPX frontmatter template".to_string()),
            ..CompletionItem::default()
        },
    ]
}
