use super::git::{git_show_file, list_files_at_ref};
use super::types::{ApiSnapshot, ExportSignature};
use docs::{parse_doc_comment_above, DocComment};
use std::collections::BTreeMap;
use syntax::{
    collect_brace_block, collect_statement, extract_name_after, extract_named_kind, normalize_ws,
};

mod docs;
mod syntax;

pub(super) use syntax::parse_reexport_names;

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
