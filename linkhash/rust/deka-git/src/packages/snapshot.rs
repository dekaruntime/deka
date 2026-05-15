use super::git::{git_show_file, list_files_at_ref};
use super::types::{ApiSnapshot, ExportSignature};
use std::collections::BTreeMap;

pub(super) fn build_api_snapshot(
    repo_path: &std::path::Path,
    git_ref: &str,
) -> Result<ApiSnapshot, anyhow::Error> {
    let files = list_files_at_ref(repo_path, git_ref)?;
    let mut exports = BTreeMap::new();

    for file in files {
        if !file.ends_with(".phpx") || file.contains("/.cache/") {
            continue;
        }
        let source = git_show_file(repo_path, git_ref, &file)?;
        parse_file_exports(&file, &source, &mut exports);
    }

    Ok(ApiSnapshot { exports })
}

pub(super) fn parse_file_exports(
    file: &str,
    source: &str,
    out: &mut BTreeMap<String, ExportSignature>,
) {
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if !trimmed.starts_with("export ") {
            i += 1;
            continue;
        }

        if trimmed.starts_with("export struct ") || trimmed.starts_with("export enum ") {
            let (decl, next) = collect_brace_block(&lines, i);
            let docs = parse_doc_comment_above(&lines, i);
            if let Some((kind, name)) = extract_named_kind(trimmed) {
                insert_export(file, &kind, &name, &decl, i + 1, docs.as_ref(), out);
            }
            i = next;
            continue;
        }

        let docs = parse_doc_comment_above(&lines, i);
        if trimmed.starts_with("export {") {
            let (decl, next) = collect_brace_block(&lines, i);
            for name in parse_reexport_names(&decl) {
                insert_export(file, "reexport", &name, &decl, i + 1, docs.as_ref(), out);
            }
            i = next;
            continue;
        }

        let (decl, next) = collect_statement(&lines, i);
        if trimmed.starts_with("export function ") {
            if let Some(name) = extract_name_after(trimmed, "export function") {
                insert_export(file, "function", &name, &decl, i + 1, docs.as_ref(), out);
            }
        } else if trimmed.starts_with("export const ") {
            if let Some(name) = extract_name_after(trimmed, "export const") {
                insert_export(file, "const", &name, &decl, i + 1, docs.as_ref(), out);
            }
        } else if trimmed.starts_with("export type ") {
            if let Some(name) = extract_name_after(trimmed, "export type") {
                insert_export(file, "type", &name, &decl, i + 1, docs.as_ref(), out);
            }
        }

        i = next;
    }
}

fn module_id(file: &str) -> String {
    file.strip_suffix(".phpx")
        .unwrap_or(file)
        .trim_start_matches("./")
        .to_string()
}

fn insert_export(
    file: &str,
    kind: &str,
    name: &str,
    decl: &str,
    line: usize,
    docs: Option<&DocComment>,
    out: &mut BTreeMap<String, ExportSignature>,
) {
    let key = format!("{}::{}", module_id(file), name);
    out.insert(
        key,
        ExportSignature {
            kind: kind.to_string(),
            signature: docs
                .and_then(|d| d.typed_signature.clone())
                .unwrap_or_else(|| normalize_ws(decl)),
            source: format!("{}:{}", file, line),
            summary: docs.and_then(|d| d.summary.clone()),
            description: docs.and_then(|d| d.description.clone()),
            examples: docs.map(|d| d.examples.clone()).unwrap_or_default(),
        },
    );
}

#[derive(Debug, Clone, Default)]
struct DocComment {
    summary: Option<String>,
    description: Option<String>,
    examples: Vec<String>,
    typed_signature: Option<String>,
}

fn parse_doc_comment_above(lines: &[&str], export_line: usize) -> Option<DocComment> {
    if export_line == 0 {
        return None;
    }

    let mut i = export_line;
    while i > 0 {
        i -= 1;
        let line = lines[i].trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("///") {
            let mut block = vec![lines[i].to_string()];
            let mut j = i;
            while j > 0 {
                let prev = lines[j - 1].trim_start();
                if prev.starts_with("///") {
                    j -= 1;
                    block.push(lines[j].to_string());
                    continue;
                }
                break;
            }
            block.reverse();
            return parse_slash_doc_block(&block);
        }

        if !line.ends_with("*/") {
            return None;
        }

        let mut block = vec![lines[i].to_string()];
        let mut j = i;
        let found_start = line.starts_with("/**");
        while !found_start {
            if j == 0 {
                return None;
            }
            j -= 1;
            let current = lines[j];
            block.push(current.to_string());
            if current.trim_start().starts_with("/**") {
                break;
            }
        }
        block.reverse();
        return parse_doc_block(&block);
    }
    None
}

fn parse_doc_block(block: &[String]) -> Option<DocComment> {
    if block.is_empty() {
        return None;
    }

    let mut lines = Vec::new();
    for (idx, raw) in block.iter().enumerate() {
        let mut line = raw.trim().to_string();
        if idx == 0 {
            line = line.trim_start_matches("/**").trim().to_string();
        }
        if idx + 1 == block.len() {
            line = line.trim_end_matches("*/").trim().to_string();
        }
        line = line.trim_start_matches('*').trim().to_string();
        if !line.is_empty() {
            lines.push(line);
        }
    }

    if lines.is_empty() {
        return None;
    }

    let mut summary = None;
    let mut description_lines = Vec::new();
    let mut examples = Vec::new();
    for line in lines {
        if line.starts_with("@example") {
            let example = line.trim_start_matches("@example").trim().to_string();
            if !example.is_empty() {
                examples.push(example);
            }
            continue;
        }
        if line.starts_with('@') {
            continue;
        }
        if summary.is_none() {
            summary = Some(line.clone());
        }
        description_lines.push(line);
    }

    let description = if description_lines.is_empty() {
        None
    } else {
        Some(description_lines.join("\n"))
    };

    Some(DocComment {
        summary,
        description,
        examples,
        typed_signature: None,
    })
}

fn parse_slash_doc_block(block: &[String]) -> Option<DocComment> {
    if block.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    for raw in block {
        let mut line = raw.trim_start().to_string();
        line = line.trim_start_matches("///").trim().to_string();
        if !line.is_empty() {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        return None;
    }

    let mut summary = None;
    let mut description = None;
    let mut examples = Vec::new();
    let mut typed_signature = None;

    if let Some(function_name) = extract_xml_attr(&lines, "Function", "name") {
        let params = extract_parameter_signature_parts(&lines);
        let return_type = extract_xml_attr(&lines, "ReturnType", "type");
        let mut sig = format!("{}(", function_name);
        sig.push_str(&params.join(", "));
        sig.push(')');
        if let Some(rt) = return_type {
            if !rt.trim().is_empty() {
                sig.push_str(": ");
                sig.push_str(rt.trim());
            }
        }
        typed_signature = Some(sig);
    }

    if let Some(desc) = extract_xml_tag(&lines, "Description") {
        let clean = desc.trim().to_string();
        if !clean.is_empty() {
            summary = Some(clean.clone());
            description = Some(clean);
        }
    }

    for line in &lines {
        if line.starts_with("@example") {
            let ex = line.trim_start_matches("@example").trim().to_string();
            if !ex.is_empty() {
                examples.push(ex);
            }
        }
    }

    if summary.is_none() {
        for line in &lines {
            if line.starts_with("docid:") || line.starts_with('<') {
                continue;
            }
            let clean = line.trim().to_string();
            if !clean.is_empty() {
                summary = Some(clean.clone());
                description = Some(clean);
                break;
            }
        }
    }

    if summary.is_none() && description.is_none() && examples.is_empty() {
        return None;
    }

    Some(DocComment {
        summary,
        description,
        examples,
        typed_signature,
    })
}

fn extract_xml_attr(lines: &[String], tag: &str, attr: &str) -> Option<String> {
    let tag_open = format!("<{}", tag);
    let needle = format!("{}=\"", attr);
    for line in lines {
        if !line.contains(&tag_open) {
            continue;
        }
        let start = line.find(&needle)?;
        let rest = &line[start + needle.len()..];
        let end = rest.find('"')?;
        let value = rest[..end].trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

fn extract_parameter_signature_parts(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for line in lines {
        if !line.contains("<Parameter ") {
            continue;
        }
        let name = extract_attr_from_line(line, "name").unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let param_type = extract_attr_from_line(line, "type").unwrap_or_default();
        if param_type.is_empty() {
            out.push(name);
        } else {
            out.push(format!("{} {}", name, param_type));
        }
    }
    out
}

fn extract_attr_from_line(line: &str, attr: &str) -> Option<String> {
    let needle = format!("{}=\"", attr);
    let start = line.find(&needle)?;
    let rest = &line[start + needle.len()..];
    let end = rest.find('"')?;
    let value = rest[..end].trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn extract_xml_tag(lines: &[String], tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let mut collecting = false;
    let mut out = Vec::new();
    for line in lines {
        if !collecting {
            if let Some(start) = line.find(&open) {
                let rest = &line[start + open.len()..];
                if let Some(end) = rest.find(&close) {
                    let inner = rest[..end].trim().to_string();
                    if !inner.is_empty() {
                        return Some(inner);
                    }
                    continue;
                }
                collecting = true;
                let first = rest.trim();
                if !first.is_empty() {
                    out.push(first.to_string());
                }
            }
            continue;
        }

        if let Some(end) = line.find(&close) {
            let part = line[..end].trim();
            if !part.is_empty() {
                out.push(part.to_string());
            }
            break;
        }
        let t = line.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("\n"))
    }
}

fn collect_statement(lines: &[&str], start: usize) -> (String, usize) {
    let mut out = Vec::new();
    let mut i = start;
    while i < lines.len() {
        out.push(lines[i]);
        let t = lines[i].trim_end();
        if t.ends_with(';') || t.ends_with('{') {
            i += 1;
            break;
        }
        i += 1;
    }
    (out.join("\n"), i)
}

fn collect_brace_block(lines: &[&str], start: usize) -> (String, usize) {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut seen_open = false;
    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        out.push(line);
        for ch in line.chars() {
            if ch == '{' {
                depth += 1;
                seen_open = true;
            } else if ch == '}' {
                depth -= 1;
            }
        }
        i += 1;
        if seen_open && depth <= 0 {
            break;
        }
    }
    (out.join("\n"), i)
}

fn extract_named_kind(trimmed: &str) -> Option<(String, String)> {
    if trimmed.starts_with("export struct ") {
        extract_name_after(trimmed, "export struct").map(|name| ("struct".to_string(), name))
    } else if trimmed.starts_with("export enum ") {
        extract_name_after(trimmed, "export enum").map(|name| ("enum".to_string(), name))
    } else {
        None
    }
}

fn extract_name_after(line: &str, prefix: &str) -> Option<String> {
    let tail = line.strip_prefix(prefix)?.trim_start();
    let mut name = String::new();
    for ch in tail.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            name.push(ch);
        } else {
            break;
        }
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

pub(super) fn parse_reexport_names(decl: &str) -> Vec<String> {
    let open = match decl.find('{') {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let close = match decl[open + 1..].find('}') {
        Some(idx) => open + 1 + idx,
        None => return Vec::new(),
    };

    decl[open + 1..close]
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .filter_map(|item| {
            if let Some((_, alias)) = item.split_once(" as ") {
                let v = alias.trim();
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                }
            } else {
                Some(item.to_string())
            }
        })
        .collect()
}
fn normalize_ws(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}
