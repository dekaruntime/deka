use crate::{ImportDecl, ImportSpec, SourceModuleMeta};

pub fn parse_source_module_meta(source: &str) -> SourceModuleMeta {
    let mut meta = SourceModuleMeta::empty();
    let lines: Vec<&str> = source.lines().collect();
    let bounds = frontmatter_range(&lines);
    let (start, end) = bounds.unwrap_or((0, lines.len()));

    if let Some((s, e)) = bounds {
        meta.frontmatter_start_line = Some(s);
        meta.frontmatter_end_line = Some(e);
        meta.template_start_line = Some(e + 1);
    }

    for line in &lines[start..end] {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        if let Some(decl) = parse_import_line(trimmed) {
            meta.imports.push(decl);
            continue;
        }

        if let Some(name) = parse_export_function_line(trimmed) {
            meta.exported_functions.insert(name);
            continue;
        }

        if let Some(specs) = parse_export_specs_line(trimmed) {
            meta.export_specs.extend(specs);
        }
    }

    meta
}
fn frontmatter_range(lines: &[&str]) -> Option<(usize, usize)> {
    let mut first = None;
    let mut second = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.trim() == "---" {
            if first.is_none() {
                first = Some(idx);
            } else {
                second = Some(idx);
                break;
            }
        }
    }
    match (first, second) {
        (Some(a), Some(b)) if b > a => Some((a + 1, b)),
        _ => None,
    }
}

fn parse_import_line(line: &str) -> Option<ImportDecl> {
    let trimmed = line.trim_end_matches(';').trim();
    if !trimmed.starts_with("import ") {
        return None;
    }
    let open = trimmed.find('{')?;
    let close = trimmed[open..].find('}')? + open;
    let from_pos = trimmed[close + 1..].find("from")? + close + 1;

    let inside = trimmed[open + 1..close].trim();
    let from_part = trimmed[from_pos + 4..].trim();
    let module = unquote(from_part)?;

    let mut specs = Vec::new();
    for chunk in inside.split(',') {
        let part = chunk.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(as_pos) = part.find(" as ") {
            let imported = part[..as_pos].trim();
            let local = part[as_pos + 4..].trim();
            if !imported.is_empty() && !local.is_empty() {
                specs.push(ImportSpec {
                    imported: imported.to_string(),
                    local: local.to_string(),
                });
            }
        } else {
            specs.push(ImportSpec {
                imported: part.to_string(),
                local: part.to_string(),
            });
        }
    }

    if specs.is_empty() {
        return None;
    }

    Some(ImportDecl {
        from: module.to_string(),
        specs,
    })
}

fn parse_export_function_line(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = if let Some(rest) = trimmed.strip_prefix("export function ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("export async function ") {
        rest
    } else {
        return None;
    };
    let name = rest.split('(').next()?.trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

fn parse_export_specs_line(line: &str) -> Option<Vec<ImportSpec>> {
    let trimmed = line.trim_end_matches(';').trim();
    if !trimmed.starts_with("export {") || !trimmed.ends_with('}') {
        return None;
    }
    let inner = &trimmed[8..trimmed.len() - 1];
    let mut specs = Vec::new();
    for chunk in inner.split(',') {
        let part = chunk.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(as_pos) = part.find(" as ") {
            let local = part[..as_pos].trim();
            let exported = part[as_pos + 4..].trim();
            if !local.is_empty() && !exported.is_empty() {
                specs.push(ImportSpec {
                    imported: exported.to_string(),
                    local: local.to_string(),
                });
            }
        } else {
            specs.push(ImportSpec {
                imported: part.to_string(),
                local: part.to_string(),
            });
        }
    }
    Some(specs)
}

fn unquote(input: &str) -> Option<&str> {
    let s = input.trim();
    if s.len() < 2 {
        return None;
    }
    let first = s.as_bytes()[0] as char;
    let last = s.as_bytes()[s.len() - 1] as char;
    if (first == '\'' && last == '\'') || (first == '"' && last == '"') {
        Some(&s[1..s.len() - 1])
    } else {
        None
    }
}
