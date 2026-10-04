//! Open buffers are passed to the native compiler, without shadow files or a
//! second checker. NativeServices is built by the CLI from its real host catalog.
use super::*;

pub(crate) fn open_document_paths(documents: &HashMap<Url, String>) -> SourceTexts {
    documents
        .iter()
        .filter_map(|(uri, text)| {
            let path = uri.to_file_path().ok()?;
            Some((deka_vm::compiler::source_path(&path), text.clone()))
        })
        .collect()
}

/// Preserve the URI the client opened even when compiler identities canonicalize
/// a symlinked parent (for example /var versus /private/var on macOS).
pub(crate) fn document_uri(documents: &HashMap<Url, String>, path: &Path) -> Option<Url> {
    let path = deka_vm::compiler::source_path(path);
    documents
        .keys()
        .find(|uri| {
            uri.to_file_path()
                .is_ok_and(|p| deka_vm::compiler::source_path(&p) == path)
        })
        .cloned()
        .or_else(|| Url::from_file_path(path).ok())
}

pub(crate) fn file_diagnostics(
    entry: &Path,
    open_documents: &SourceTexts,
    services: &NativeServices,
) -> BTreeMap<PathBuf, Vec<Diagnostic>> {
    let mut by_file = BTreeMap::<PathBuf, Vec<Diagnostic>>::new();
    let Err(error) = (services.validate)(entry, open_documents) else {
        return by_file;
    };
    for finding in deka_vm::compiler::source_diagnostics(entry, &error) {
        let source = module_source(&finding.path, open_documents);
        let range = compiler_range(&source, finding.line, finding.column);
        by_file.entry(finding.path).or_default().push(Diagnostic {
            range,
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some(LANGUAGE_ID.to_owned()),
            message: finding.message,
            ..Diagnostic::default()
        });
    }
    by_file
}

fn module_source(path: &Path, open_documents: &SourceTexts) -> String {
    open_documents
        .get(&deka_vm::compiler::source_path(path))
        .cloned()
        .unwrap_or_else(|| fs::read_to_string(path).unwrap_or_default())
}

// Retained dsc analysis mapping: compiler positions count characters; LSP uses
// UTF-16 code units. Clamp edits/errors at valid scalar boundaries.
fn compiler_range(source: &str, line: Option<u32>, column: Option<u32>) -> Range {
    let lines: Vec<&str> = source.split('\n').collect();
    let line = line.unwrap_or(1).saturating_sub(1) as usize;
    let line = line.min(lines.len().saturating_sub(1));
    let text = lines.get(line).copied().unwrap_or_default();
    let character = column.unwrap_or(1).saturating_sub(1) as usize;
    let mut chars = text.chars();
    let start: usize = chars.by_ref().take(character).map(char::len_utf16).sum();
    let end = start + chars.next().map(char::len_utf16).unwrap_or(0);
    Range::new(
        Position::new(line as u32, start as u32),
        Position::new(line as u32, end as u32),
    )
}

pub(crate) fn project_module_location(
    entry: &Path,
    module: &str,
    documents: &SourceTexts,
    services: &NativeServices,
) -> Option<(PathBuf, String)> {
    let info = (services.module)(entry, module, documents).ok()?;
    Some((info.path?, info.source?))
}
pub(crate) fn project_module_exports(
    entry: &Path,
    module: &str,
    documents: &SourceTexts,
    services: &NativeServices,
) -> Option<Vec<ExportInfo>> {
    Some(
        (services.module)(entry, module, documents)
            .ok()?
            .exports
            .into_iter()
            .map(|export| ExportInfo {
                name: export.name,
                kind: Some(export.kind),
            })
            .collect(),
    )
}
