use super::*;

pub(crate) struct ImportInfo {
    pub(crate) imported: String,
    pub(crate) local: String,
    pub(crate) from: String,
    pub(crate) span: Span,
}

pub(crate) fn parse_imports(source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    let mut offset = 0usize;
    for line in source.lines() {
        let line_len = line.len();
        let trimmed = line.trim_start();
        if trimmed.starts_with("import ") {
            let module_info = parse_module_path_with_span(line, offset);
            let mut rest = trimmed
                .strip_prefix("import")
                .unwrap_or(trimmed)
                .trim_start();
            let mut default_name: Option<&str> = None;
            let mut spec_part: Option<&str> = None;

            if rest.starts_with('{') {
                if let (Some(open), Some(close)) = (line.find('{'), line.find('}')) {
                    spec_part = Some(&line[open + 1..close]);
                    rest = &rest[rest.find('}').unwrap_or(0) + 1..];
                }
            } else {
                if let Some((name, after)) = parse_ident_from_str(rest) {
                    default_name = Some(name);
                    rest = after.trim_start();
                    if let Some(rest_after_comma) = rest.strip_prefix(',') {
                        rest = rest_after_comma.trim_start();
                        if let (Some(open), Some(close)) = (line.find('{'), line.find('}')) {
                            spec_part = Some(&line[open + 1..close]);
                            rest = &rest[rest.find('}').unwrap_or(0) + 1..];
                        }
                    }
                }
            }

            if let Some(name) = default_name
                && let Some(col) = line.find(name)
            {
                let span = Span::new(offset + col, offset + col + name.len());
                if let Some((module, _)) = module_info.clone() {
                    imports.push(ImportInfo {
                        imported: "default".to_string(),
                        local: name.to_string(),
                        from: module,
                        span,
                    });
                }
            }

            if let Some(spec_part) = spec_part
                && let (Some(open), Some(_close)) = (line.find('{'), line.find('}'))
            {
                let mut cursor = open + 1;
                for spec in spec_part.split(',') {
                    let spec_trim = spec.trim();
                    if spec_trim.is_empty() {
                        cursor += spec.len() + 1;
                        continue;
                    }
                    let (imported, local) =
                        if let Some((left, right)) = spec_trim.split_once(" as ") {
                            (left.trim(), right.trim())
                        } else {
                            (spec_trim, spec_trim)
                        };
                    let local_pos = line[cursor..].find(local).map(|idx| cursor + idx);
                    if let Some(local_pos) = local_pos {
                        let span = Span::new(offset + local_pos, offset + local_pos + local.len());
                        if let Some((module, _)) = module_info.clone() {
                            imports.push(ImportInfo {
                                imported: imported.to_string(),
                                local: local.to_string(),
                                from: module,
                                span,
                            });
                        } else {
                            imports.push(ImportInfo {
                                imported: imported.to_string(),
                                local: local.to_string(),
                                from: imported.to_string(),
                                span,
                            });
                        }
                    }
                    cursor += spec.len() + 1;
                }
            }
        }
        offset += line_len + 1;
    }
    imports
}

pub(crate) fn parse_ident_from_str(input: &str) -> Option<(&str, &str)> {
    let mut chars = input.char_indices();
    let (start, first) = chars.next()?;
    if start != 0 {
        return None;
    }
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }
    let mut end = first.len_utf8();
    for (idx, ch) in chars {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    Some((&input[..end], &input[end..]))
}

pub(crate) fn parse_module_path(line: &str) -> Option<String> {
    let from_idx = line.find("from")?;
    let rest = &line[from_idx + 4..];
    let quote = rest.find(&['\'', '"'][..])?;
    let quote_char = rest.chars().nth(quote)?;
    let after = &rest[quote + 1..];
    let end = after.find(quote_char)?;
    Some(after[..end].to_string())
}

pub(crate) fn parse_module_path_with_span(
    line: &str,
    line_offset: usize,
) -> Option<(String, Span)> {
    let from_idx = line.find("from")?;
    let rest = &line[from_idx + 4..];
    let quote = rest.find(&['\'', '"'][..])?;
    let quote_char = rest.chars().nth(quote)?;
    let after = &rest[quote + 1..];
    let end = after.find(quote_char)?;
    let start = line_offset + from_idx + 4 + quote + 1;
    let end_pos = start + end;
    Some((after[..end].to_string(), Span::new(start, end_pos)))
}

pub(crate) fn import_module_at_offset(source: &str, offset: usize) -> Option<String> {
    for (line, line_offset) in line_with_offsets(source) {
        let line_end = line_offset + line.len();
        if offset < line_offset || offset > line_end {
            continue;
        }
        if !line.contains("import") || !line.contains("from") {
            return None;
        }
        let (module_spec, span) = parse_module_path_with_span(line, line_offset)?;
        if offset >= span.start && offset <= span.end {
            return Some(module_spec);
        }
        return None;
    }
    None
}

pub(crate) fn import_module_spans(source: &str, module_spec: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    for (line, line_offset) in line_with_offsets(source) {
        if !line.contains("import") || !line.contains("from") {
            continue;
        }
        if let Some((found, span)) = parse_module_path_with_span(line, line_offset)
            && found == module_spec
        {
            spans.push(span);
        }
    }
    spans
}

/// Every quoted module specifier in the file's import lines. Symbol
/// references and renames exclude these: `./Counter.dsx` is a module path,
/// not a use of the `Counter` binding, and rewriting it would corrupt the
/// import.
fn import_module_specifier_spans(source: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    for (line, line_offset) in line_with_offsets(source) {
        if !line.contains("import") || !line.contains("from") {
            continue;
        }
        if let Some((_, span)) = parse_module_path_with_span(line, line_offset) {
            spans.push(span);
        }
    }
    spans
}

/// Whole-word occurrences of `word` outside `excluded` spans.
fn symbol_occurrences(source: &str, word: &str, excluded: &[Span]) -> Vec<Span> {
    find_word_occurrences(source.as_bytes(), word)
        .into_iter()
        .filter(|span| {
            !excluded
                .iter()
                .any(|ex| ex.start <= span.start && span.end <= ex.end)
        })
        .collect()
}

pub(crate) fn line_with_offsets(source: &str) -> Vec<(&str, usize)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for line in source.split('\n') {
        out.push((line, offset));
        offset += line.len() + 1;
    }
    out
}

pub(crate) struct ExportInfo {
    pub(crate) name: String,
    pub(crate) kind: Option<CompletionItemKind>,
}

pub(crate) fn completion_for_import(
    source: &str,
    file_path: &str,
    offset: usize,
    services: &NativeServices,
    open_documents: &SourceTexts,
) -> Option<Vec<CompletionItem>> {
    let line_index = LineIndex::new(source);
    let position = line_index.offset_to_position(offset);
    let line = position.line as usize;
    let line_start = *line_index.line_starts.get(line)?;
    let line_end = source[line_start..]
        .find('\n')
        .map(|idx| line_start + idx)
        .unwrap_or(source.len());
    let line_text = &source[line_start..line_end];
    if !line_text.contains("import") || !line_text.contains("from") {
        return None;
    }
    let rel = offset.saturating_sub(line_start);

    if let Some(open) = line_text.find('{') {
        let close = line_text.find('}').unwrap_or(line_text.len());
        if rel > open && rel <= close {
            let module_spec = parse_module_path(line_text)?;
            let prefix_start = line_text[..rel]
                .rfind(',')
                .map(|idx| idx + 1)
                .unwrap_or(open + 1);
            let raw_prefix = line_text[prefix_start..rel].trim();
            let prefix = raw_prefix
                .split_whitespace()
                .last()
                .unwrap_or(raw_prefix)
                .trim();
            // Project-aware first: resolution and parsing shared with the
            // graph check `dsc check` runs, with unsaved buffers overlaid.
            if let Some(exports) =
                project_module_exports(Path::new(file_path), &module_spec, open_documents, services)
            {
                return Some(exports_to_completion_items(exports, prefix));
            }
            return None;
        }
    }

    let before_cursor = &source[line_start..offset.min(source.len())];
    let quote_pos = before_cursor.rfind(&['\'', '"'][..])?;
    let prefix = &before_cursor[quote_pos + 1..];

    let modules = services.modules.clone();
    let mut items = Vec::new();
    for module in modules {
        if !prefix.is_empty() && !module.starts_with(prefix) {
            continue;
        }
        items.push(CompletionItem {
            label: module.clone(),
            kind: Some(CompletionItemKind::MODULE),
            ..CompletionItem::default()
        });
    }
    Some(items)
}
fn exports_to_completion_items(exports: Vec<ExportInfo>, prefix: &str) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for export in exports {
        if !prefix.is_empty() && !export.name.starts_with(prefix) {
            continue;
        }
        items.push(CompletionItem {
            label: export.name,
            kind: export.kind,
            ..CompletionItem::default()
        });
    }
    items
}
pub(crate) fn collect_module_rename_edits(
    roots: &[PathBuf],
    active_uri: &Url,
    active_text: &str,
    old_module: &str,
    new_module: &str,
) -> HashMap<Url, Vec<TextEdit>> {
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for root in roots {
        for file in collect_dekascript_files(root) {
            let file_uri = match Url::from_file_path(&file) {
                Ok(uri) => uri,
                Err(_) => continue,
            };
            let content = if &file_uri == active_uri {
                active_text.to_string()
            } else {
                fs::read_to_string(&file).unwrap_or_default()
            };
            let line_index = LineIndex::new(&content);
            let mut edits = Vec::new();
            for span in import_module_spans(&content, old_module) {
                edits.push(TextEdit {
                    range: span_to_range(span, &line_index),
                    new_text: new_module.to_string(),
                });
            }
            if !edits.is_empty() {
                changes.insert(file_uri, edits);
            }
        }
    }
    changes
}

pub(crate) fn collect_reference_locations(
    roots: &[PathBuf],
    active_uri: &Url,
    active_text: &str,
    symbol: &str,
) -> Vec<Location> {
    let mut locations = Vec::new();
    let mut saw_active = false;
    for root in roots {
        for file in collect_dekascript_files(root) {
            let file_uri = match Url::from_file_path(&file) {
                Ok(uri) => uri,
                Err(_) => continue,
            };
            let content = if &file_uri == active_uri {
                saw_active = true;
                active_text.to_string()
            } else {
                fs::read_to_string(&file).unwrap_or_default()
            };
            let line_index = LineIndex::new(&content);
            let module_spans = import_module_specifier_spans(&content);
            for span in symbol_occurrences(&content, symbol, &module_spans) {
                locations.push(Location {
                    uri: file_uri.clone(),
                    range: span_to_range(span, &line_index),
                });
            }
        }
    }
    // The file under the cursor may live outside every workspace root (a
    // single-file session, or a root that does not contain it); its unsaved
    // text still holds occurrences.
    if !saw_active {
        let line_index = LineIndex::new(active_text);
        let module_spans = import_module_specifier_spans(active_text);
        for span in symbol_occurrences(active_text, symbol, &module_spans) {
            locations.push(Location {
                uri: active_uri.clone(),
                range: span_to_range(span, &line_index),
            });
        }
    }
    locations
}

pub(crate) fn collect_symbol_rename_edits(
    roots: &[PathBuf],
    active_uri: &Url,
    active_text: &str,
    old_symbol: &str,
    new_symbol: &str,
) -> HashMap<Url, Vec<TextEdit>> {
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    let mut saw_active = false;
    for root in roots {
        for file in collect_dekascript_files(root) {
            let file_uri = match Url::from_file_path(&file) {
                Ok(uri) => uri,
                Err(_) => continue,
            };
            let content = if &file_uri == active_uri {
                saw_active = true;
                active_text.to_string()
            } else {
                fs::read_to_string(&file).unwrap_or_default()
            };
            let line_index = LineIndex::new(&content);
            let module_spans = import_module_specifier_spans(&content);
            let mut edits = Vec::new();
            for span in symbol_occurrences(&content, old_symbol, &module_spans) {
                edits.push(TextEdit {
                    range: span_to_range(span, &line_index),
                    new_text: new_symbol.to_string(),
                });
            }
            if !edits.is_empty() {
                changes.insert(file_uri, edits);
            }
        }
    }
    // Same guarantee as references: the file under the cursor is renamed even
    // when no workspace root contains it.
    if !saw_active {
        let line_index = LineIndex::new(active_text);
        let module_spans = import_module_specifier_spans(active_text);
        let edits: Vec<TextEdit> = symbol_occurrences(active_text, old_symbol, &module_spans)
            .into_iter()
            .map(|span| TextEdit {
                range: span_to_range(span, &line_index),
                new_text: new_symbol.to_string(),
            })
            .collect();
        if !edits.is_empty() {
            changes.insert(active_uri.clone(), edits);
        }
    }
    changes
}

pub(crate) fn collect_dekascript_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if should_skip_dir(&path) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if is_dekascript_path(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

pub(crate) fn should_skip_dir(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    name.starts_with('.')
        || name == "node_modules"
        || name == "target"
        || name == "dist"
        || name == "build"
        || name == "vendor"
}
