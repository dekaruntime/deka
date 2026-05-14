use bumpalo::Bump;
use modules_php::compiler_api::compile_phpx;
use modules_php::validation::{Severity, ValidationError, ValidationWarning};
use php_rs::parser::ast::{
    BinaryOp, ClassKind, ClassMember, Expr, ExprId, Name, ObjectKey, Param, Program, Stmt, StmtId,
    Type,
};
use php_rs::parser::lexer::token::Token;
use php_rs::parser::span::Span;
use php_rs::phpx::typeck::{ExternalFunctionSig, Type as PhpType};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticOptions, DiagnosticServerCapabilities, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, DocumentDiagnosticParams,
    DocumentDiagnosticReport, DocumentDiagnosticReportResult, DocumentSymbol, DocumentSymbolParams,
    Documentation, FullDocumentDiagnosticReport, Hover, HoverContents, InitializeParams,
    InitializeResult, InitializedParams, InsertTextFormat, Location, MarkupContent, MarkupKind,
    MessageType, OneOf, Position, Range, ReferenceParams, RelatedFullDocumentDiagnosticReport,
    RenameParams, ServerCapabilities, SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextEdit, Url, WorkspaceEdit,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

struct Backend {
    _client: Client,
    documents: Arc<RwLock<HashMap<Url, String>>>,
    workspace_roots: Arc<RwLock<Vec<PathBuf>>>,
    target_mode: Arc<RwLock<TargetMode>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
enum TargetMode {
    #[default]
    Server,
    Adwa,
}

impl TargetMode {
    fn from_initialize_params(params: &InitializeParams) -> Self {
        let Some(options) = params.initialization_options.as_ref() else {
            return Self::Server;
        };
        let Some(root) = options.as_object() else {
            return Self::Server;
        };
        let Some(phpx) = root.get("phpx").and_then(|value| value.as_object()) else {
            return Self::Server;
        };
        let Some(target) = phpx.get("target").and_then(|value| value.as_str()) else {
            return Self::Server;
        };
        if target.eq_ignore_ascii_case("adwa") {
            Self::Adwa
        } else {
            Self::Server
        }
    }
}

impl Backend {
    async fn diagnostics_for_text(&self, text: &str, file_path: &str) -> Vec<Diagnostic> {
        let arena = Bump::new();
        let result = compile_phpx(text, file_path, &arena);
        let mut diagnostics = Vec::new();
        let workspace_roots = self.workspace_roots.read().await.clone();
        let target_mode = *self.target_mode.read().await;
        let unresolved_imports = unresolved_import_diagnostics(text, file_path, &workspace_roots);
        let unresolved_ranges: std::collections::HashSet<(u32, u32, u32, u32)> = unresolved_imports
            .iter()
            .map(|diag| {
                (
                    diag.range.start.line,
                    diag.range.start.character,
                    diag.range.end.line,
                    diag.range.end.character,
                )
            })
            .collect();

        for error in result.errors {
            if should_skip_template_html_diagnostic(&error) {
                continue;
            }
            diagnostics.push(diagnostic_from_error(file_path, text, &error));
        }

        for warning in result.warnings {
            if should_skip_unused_import_warning(&warning, &unresolved_ranges) {
                continue;
            }
            diagnostics.push(diagnostic_from_warning(file_path, text, &warning));
        }
        diagnostics.extend(unresolved_imports);
        diagnostics.extend(target_capability_diagnostics(text, target_mode));
        diagnostics
    }

    async fn validate_document(&self, uri: Url, text: &str) {
        let file_path = uri
            .to_file_path()
            .ok()
            .and_then(|path| path.to_str().map(|path| path.to_string()))
            .unwrap_or_else(|| uri.to_string());
        let diagnostics = self.diagnostics_for_text(text, &file_path).await;

        self._client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    async fn get_document(&self, uri: &Url) -> Option<String> {
        let docs = self.documents.read().await;
        docs.get(uri).cloned()
    }
}

fn should_skip_template_html_diagnostic(error: &ValidationError) -> bool {
    // Frontmatter template sections intentionally allow HTML-style markup.
    // Runtime handles this as template HTML, but compile-time JSX-style validation
    // can emit false positives for opening/closing tag rules in editor feedback.
    error
        .help_text
        .contains("Fix JSX/template syntax in the template section.")
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(
        &self,
        params: InitializeParams,
    ) -> tower_lsp::jsonrpc::Result<InitializeResult> {
        let target_mode = TargetMode::from_initialize_params(&params);
        *self.target_mode.write().await = target_mode;
        let mut roots = Vec::new();
        if let Some(folders) = params.workspace_folders {
            for folder in folders {
                if let Ok(path) = folder.uri.to_file_path() {
                    roots.push(path);
                }
            }
        } else if let Some(root_uri) = params.root_uri {
            if let Ok(path) = root_uri.to_file_path() {
                roots.push(path);
            }
        }
        if roots.is_empty() {
            if let Ok(current) = std::env::current_dir() {
                roots.push(current);
            }
        }
        *self.workspace_roots.write().await = roots;

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(true.into()),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "'".to_string(),
                        "\"".to_string(),
                        ".".to_string(),
                        "\\".to_string(),
                        "<".to_string(),
                        " ".to_string(),
                    ]),
                    ..CompletionOptions::default()
                }),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        identifier: Some("phpx".to_string()),
                        inter_file_dependencies: true,
                        workspace_diagnostics: false,
                        work_done_progress_options: Default::default(),
                    },
                )),
                definition_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
                ..ServerCapabilities::default()
            },
            server_info: None,
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self._client
            .log_message(MessageType::INFO, "PHPX LSP initialized")
            .await;
    }

    async fn shutdown(&self) -> tower_lsp::jsonrpc::Result<()> {
        Ok(())
    }

    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> tower_lsp::jsonrpc::Result<DocumentDiagnosticReportResult> {
        let uri = params.text_document.uri;
        let text = if let Some(in_memory) = self.get_document(&uri).await {
            in_memory
        } else if let Ok(path) = uri.to_file_path() {
            fs::read_to_string(path).unwrap_or_default()
        } else {
            String::new()
        };
        let file_path = uri
            .to_file_path()
            .ok()
            .and_then(|path| path.to_str().map(|path| path.to_string()))
            .unwrap_or_else(|| uri.to_string());
        let diagnostics = self.diagnostics_for_text(&text, &file_path).await;
        Ok(DocumentDiagnosticReportResult::Report(
            DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: diagnostics,
                },
            }),
        ))
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self._client
            .log_message(
                MessageType::INFO,
                format!("Opened {}", params.text_document.uri),
            )
            .await;

        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.documents
            .write()
            .await
            .insert(uri.clone(), text.clone());
        self.validate_document(uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self._client
            .log_message(
                MessageType::INFO,
                format!("Changed {}", params.text_document.uri),
            )
            .await;

        let uri = params.text_document.uri;
        let text = params
            .content_changes
            .last()
            .map(|change| change.text.as_str())
            .unwrap_or("")
            .to_string();
        self.documents
            .write()
            .await
            .insert(uri.clone(), text.clone());
        self.validate_document(uri, &text).await;
    }

    async fn hover(
        &self,
        params: tower_lsp::lsp_types::HoverParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.get_document(&uri).await else {
            return Ok(None);
        };

        let file_path = uri
            .to_file_path()
            .ok()
            .and_then(|path| path.to_str().map(|path| path.to_string()))
            .unwrap_or_else(|| uri.to_string());

        let line_index = LineIndex::new(&text);
        let offset = match line_index.position_to_offset(position) {
            Some(offset) => offset,
            None => return Ok(None),
        };

        let arena = Bump::new();
        let result = compile_phpx(&text, &file_path, &arena);
        let mut hover_text = None;
        if let Some(program) = result.ast.as_ref() {
            let index = build_index(program, text.as_bytes());
            hover_text = index.hover_at(offset);
        }

        if hover_text.is_none() {
            if let Some(word) = word_at_offset(text.as_bytes(), offset) {
                if let Some(sig) = result.wasm_functions.get(&word) {
                    let signature = format_external_signature(&word, sig);
                    hover_text = Some(format!("```php\n{}\n```", signature));
                }
            }
        }
        if hover_text.is_none() {
            hover_text = hover_for_annotation(&text, offset);
        }
        if hover_text.is_none() {
            hover_text = hover_from_import(&text, offset);
        }

        let Some(value) = hover_text else {
            return Ok(None);
        };

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        }))
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let Some(text) = self.get_document(&uri).await else {
            return Ok(None);
        };
        let file_path = uri
            .to_file_path()
            .ok()
            .and_then(|path| path.to_str().map(|path| path.to_string()))
            .unwrap_or_else(|| uri.to_string());

        let line_index = LineIndex::new(&text);
        let offset = match line_index.position_to_offset(position) {
            Some(offset) => offset,
            None => return Ok(None),
        };

        let mut dot_items = None;
        with_program(&text, &file_path, |program, source| {
            let index = build_index(program, source);
            dot_items = completion_for_dot(&index, source, offset);
        });
        if let Some(items) = dot_items {
            return Ok(Some(CompletionResponse::Array(items)));
        }

        let mut jsx_prop_items = None;
        with_program(&text, &file_path, |program, source| {
            let index = build_index(program, source);
            jsx_prop_items = completion_for_jsx_props(&index, source, offset);
        });
        if let Some(items) = jsx_prop_items {
            return Ok(Some(CompletionResponse::Array(items)));
        }

        let workspace_roots = self.workspace_roots.read().await.clone();
        if let Some(items) = completion_for_import(&text, &file_path, offset, &workspace_roots) {
            return Ok(Some(CompletionResponse::Array(items)));
        }
        if let Some(items) = completion_for_annotation(&text, offset) {
            return Ok(Some(CompletionResponse::Array(items)));
        }

        let mut items = builtin_completion_items();
        items.extend(stdlib_completion_items());
        items.extend(snippet_completion_items());
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn goto_definition(
        &self,
        params: tower_lsp::lsp_types::GotoDefinitionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<tower_lsp::lsp_types::GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(text) = self.get_document(&uri).await else {
            return Ok(None);
        };
        let file_path = uri
            .to_file_path()
            .ok()
            .and_then(|path| path.to_str().map(|path| path.to_string()))
            .unwrap_or_else(|| uri.to_string());

        let line_index = LineIndex::new(&text);
        let offset = match line_index.position_to_offset(position) {
            Some(offset) => offset,
            None => return Ok(None),
        };

        let mut location = None;
        with_program(&text, &file_path, |program, source| {
            let index = build_index(program, source);
            location = index.definition_at(offset, &uri, &line_index, source);
        });

        if let Some(loc) = location {
            return Ok(Some(tower_lsp::lsp_types::GotoDefinitionResponse::Scalar(
                loc,
            )));
        }

        let workspace_roots = self.workspace_roots.read().await.clone();
        if let Some(word) = word_at_offset(text.as_bytes(), offset) {
            if let Some(loc) =
                definition_for_imported_symbol(&text, &file_path, &word, &workspace_roots)
            {
                return Ok(Some(tower_lsp::lsp_types::GotoDefinitionResponse::Scalar(
                    loc,
                )));
            }
        }

        if let Some(loc) = definition_for_import_module(&text, &file_path, offset, &workspace_roots)
        {
            return Ok(Some(tower_lsp::lsp_types::GotoDefinitionResponse::Scalar(
                loc,
            )));
        }

        Ok(None)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> tower_lsp::jsonrpc::Result<Option<tower_lsp::lsp_types::DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let Some(text) = self.get_document(&uri).await else {
            return Ok(None);
        };
        let file_path = uri
            .to_file_path()
            .ok()
            .and_then(|path| path.to_str().map(|path| path.to_string()))
            .unwrap_or_else(|| uri.to_string());

        let line_index = LineIndex::new(&text);
        let mut symbols = Vec::new();
        with_program(&text, &file_path, |program, source| {
            let index = build_index(program, source);
            symbols = index.document_symbols(&line_index);
        });

        Ok(Some(tower_lsp::lsp_types::DocumentSymbolResponse::Nested(
            symbols,
        )))
    }

    async fn references(
        &self,
        params: ReferenceParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let mut text = self.get_document(&uri).await;
        if text.is_none() {
            if let Ok(path) = uri.to_file_path() {
                text = fs::read_to_string(path).ok();
            }
        }
        let Some(text) = text else {
            return Ok(None);
        };
        let line_index = LineIndex::new(&text);
        let offset = match line_index.position_to_offset(position) {
            Some(offset) => offset,
            None => return Ok(None),
        };

        let mut roots = self.workspace_roots.read().await.clone();
        if roots.is_empty() {
            if let Ok(path) = uri.to_file_path() {
                if let Some(parent) = path.parent() {
                    roots.push(parent.to_path_buf());
                }
            }
        }

        let Some(word) = word_at_offset(text.as_bytes(), offset) else {
            return Ok(None);
        };

        let locations = collect_reference_locations(&roots, &uri, &text, &word);

        Ok(Some(locations))
    }

    async fn rename(
        &self,
        params: RenameParams,
    ) -> tower_lsp::jsonrpc::Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;
        let mut text = self.get_document(&uri).await;
        if text.is_none() {
            if let Ok(path) = uri.to_file_path() {
                text = fs::read_to_string(path).ok();
            }
        }
        let Some(text) = text else {
            return Ok(None);
        };
        let line_index = LineIndex::new(&text);
        let offset = match line_index.position_to_offset(position) {
            Some(offset) => offset,
            None => return Ok(None),
        };

        let mut roots = self.workspace_roots.read().await.clone();
        if roots.is_empty() {
            if let Ok(path) = uri.to_file_path() {
                if let Some(parent) = path.parent() {
                    roots.push(parent.to_path_buf());
                }
            }
        }

        if let Some(module_spec) = import_module_at_offset(&text, offset) {
            let changes = collect_module_rename_edits(&roots, &uri, &text, &module_spec, &new_name);
            if !changes.is_empty() {
                return Ok(Some(WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                }));
            }
        }

        let Some(word) = word_at_offset(text.as_bytes(), offset) else {
            return Ok(None);
        };

        let changes = collect_symbol_rename_edits(&roots, &uri, &text, &word, &new_name);

        if changes.is_empty() {
            return Ok(None);
        }

        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }))
    }
}

pub async fn run_stdio() -> anyhow::Result<()> {
    let (service, socket) = LspService::new(|client| Backend {
        _client: client,
        documents: Arc::new(RwLock::new(HashMap::new())),
        workspace_roots: Arc::new(RwLock::new(Vec::new())),
        target_mode: Arc::new(RwLock::new(TargetMode::default())),
    });
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
    Ok(())
}

fn diagnostic_from_error(file_path: &str, source: &str, error: &ValidationError) -> Diagnostic {
    let rendered = deka_validation::format_validation_error_with_suggestion(
        source,
        file_path,
        error.kind.as_str(),
        error.line,
        error.column,
        &error.message,
        &error.help_text,
        error.underline_length,
        severity_label(error.severity),
        None,
        error.suggestion.clone(),
    );
    Diagnostic {
        range: diagnostic_range(error.line, error.column, error.underline_length),
        severity: Some(severity_to_lsp(error.severity)),
        code: Some(tower_lsp::lsp_types::NumberOrString::String(
            error.kind.as_str().to_string(),
        )),
        source: Some("phpx".to_string()),
        message: strip_ansi_codes(&rendered),
        ..Diagnostic::default()
    }
}

fn diagnostic_from_warning(
    file_path: &str,
    source: &str,
    warning: &ValidationWarning,
) -> Diagnostic {
    let rendered = deka_validation::format_validation_error_with_suggestion(
        source,
        file_path,
        warning.kind.as_str(),
        warning.line,
        warning.column,
        &warning.message,
        &warning.help_text,
        warning.underline_length,
        severity_label(warning.severity),
        None,
        warning.suggestion.clone(),
    );
    Diagnostic {
        range: diagnostic_range(warning.line, warning.column, warning.underline_length),
        severity: Some(severity_to_lsp(warning.severity)),
        code: Some(tower_lsp::lsp_types::NumberOrString::String(
            warning.kind.as_str().to_string(),
        )),
        source: Some("phpx".to_string()),
        message: strip_ansi_codes(&rendered),
        ..Diagnostic::default()
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn strip_ansi_codes(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i += 2;
            while i < bytes.len() {
                let b = bytes[i];
                if (b as char).is_ascii_alphabetic() {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }

    let bytes = out.as_bytes();
    let mut cleaned = String::with_capacity(out.len());
    let mut j = 0usize;
    while j < bytes.len() {
        if bytes[j] == b'[' {
            let mut k = j + 1;
            let mut saw_digit = false;
            while k < bytes.len() && (bytes[k].is_ascii_digit() || bytes[k] == b';') {
                saw_digit = true;
                k += 1;
            }
            if saw_digit && k < bytes.len() && bytes[k] == b'm' {
                j = k + 1;
                continue;
            }
        }
        cleaned.push(bytes[j] as char);
        j += 1;
    }
    cleaned
}

fn severity_to_lsp(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
    }
}

fn diagnostic_range(line: usize, column: usize, underline_length: usize) -> Range {
    let line = line.saturating_sub(1) as u32;
    let start_char = column.saturating_sub(1) as u32;
    let end_char = start_char + underline_length.max(1) as u32;
    Range {
        start: Position {
            line,
            character: start_char,
        },
        end: Position {
            line,
            character: end_char,
        },
    }
}

fn should_skip_unused_import_warning(
    warning: &ValidationWarning,
    unresolved_ranges: &std::collections::HashSet<(u32, u32, u32, u32)>,
) -> bool {
    if !warning.message.contains("Unused import") {
        return false;
    }
    let range = diagnostic_range(warning.line, warning.column, warning.underline_length);
    let key = (
        range.start.line,
        range.start.character,
        range.end.line,
        range.end.character,
    );
    unresolved_ranges.contains(&key)
}

struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (idx, byte) in source.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        Self { line_starts }
    }

    fn offset_to_position(&self, offset: usize) -> Position {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line_start = self.line_starts.get(line).copied().unwrap_or(0);
        Position {
            line: line as u32,
            character: offset.saturating_sub(line_start) as u32,
        }
    }

    fn position_to_offset(&self, position: Position) -> Option<usize> {
        let line = position.line as usize;
        let line_start = *self.line_starts.get(line)?;
        Some(line_start + position.character as usize)
    }
}

fn with_program<F, R>(source: &str, file_path: &str, f: F) -> Option<R>
where
    F: FnOnce(&Program, &[u8]) -> R,
{
    let arena = Bump::new();
    let result = compile_phpx(source, file_path, &arena);
    let program = result.ast?;
    Some(f(&program, source.as_bytes()))
}

#[derive(Default)]
struct SymbolIndex {
    functions: Vec<FunctionInfo>,
    structs: Vec<StructInfo>,
    interfaces: Vec<InterfaceInfo>,
    enums: Vec<EnumInfo>,
    type_aliases: Vec<TypeAliasInfo>,
    globals: Vec<VarInfo>,
    consts: Vec<ConstInfo>,
}

#[derive(Clone)]
struct FunctionInfo {
    name: String,
    span: Span,
    signature: String,
    props_type: Option<String>,
    vars: Vec<VarInfo>,
    scope_span: Span,
}

#[derive(Clone)]
struct VarInfo {
    name: String,
    span: Span,
    ty: Option<String>,
}

#[derive(Clone)]
struct StructInfo {
    name: String,
    span: Span,
    fields: Vec<FieldInfo>,
}

#[derive(Clone)]
struct InterfaceInfo {
    name: String,
    span: Span,
    fields: Vec<FieldInfo>,
}

#[derive(Clone)]
struct FieldInfo {
    name: String,
    span: Span,
    ty: Option<String>,
}

#[derive(Clone)]
struct EnumInfo {
    name: String,
    span: Span,
    cases: Vec<EnumCaseInfo>,
}

#[derive(Clone)]
struct EnumCaseInfo {
    name: String,
    span: Span,
}

#[derive(Clone)]
struct TypeAliasInfo {
    name: String,
    span: Span,
    ty: String,
}

#[derive(Clone)]
struct ConstInfo {
    name: String,
    span: Span,
}

impl SymbolIndex {
    fn hover_at(&self, offset: usize) -> Option<String> {
        for func in &self.functions {
            if span_contains(func.span, offset) {
                return Some(format!("```php\n{}\n```", func.signature));
            }
            for var in &func.vars {
                if span_contains(var.span, offset) {
                    let ty = var.ty.clone().unwrap_or_else(|| "unknown".to_string());
                    return Some(format!("```php\n{}: {}\n```", var.name, ty));
                }
            }
        }

        for strukt in &self.structs {
            if span_contains(strukt.span, offset) {
                let mut out = format!("```php\nstruct {}\n", strukt.name);
                for field in &strukt.fields {
                    let ty = field.ty.clone().unwrap_or_else(|| "mixed".to_string());
                    out.push_str(&format!(
                        "  ${}: {}\n",
                        field.name.trim_start_matches('$'),
                        ty
                    ));
                }
                out.push_str("```");
                return Some(out);
            }
            for field in &strukt.fields {
                if span_contains(field.span, offset) {
                    let ty = field.ty.clone().unwrap_or_else(|| "mixed".to_string());
                    return Some(format!(
                        "```php\n${}: {}\n```",
                        field.name.trim_start_matches('$'),
                        ty
                    ));
                }
            }
        }

        for iface in &self.interfaces {
            if span_contains(iface.span, offset) {
                let mut out = format!("```php\ninterface {} {{\n", iface.name);
                for field in &iface.fields {
                    let ty = field.ty.clone().unwrap_or_else(|| "mixed".to_string());
                    out.push_str(&format!(
                        "  ${}: {}\n",
                        field.name.trim_start_matches('$'),
                        ty
                    ));
                }
                out.push_str("}\n```");
                return Some(out);
            }
            for field in &iface.fields {
                if span_contains(field.span, offset) {
                    let ty = field.ty.clone().unwrap_or_else(|| "mixed".to_string());
                    return Some(format!(
                        "```php\n${}: {}\n```",
                        field.name.trim_start_matches('$'),
                        ty
                    ));
                }
            }
        }

        for en in &self.enums {
            if span_contains(en.span, offset) {
                let mut out = format!("```php\nenum {}\n", en.name);
                for case_info in &en.cases {
                    out.push_str(&format!("  case {}\n", case_info.name));
                }
                out.push_str("```");
                return Some(out);
            }
            for case_info in &en.cases {
                if span_contains(case_info.span, offset) {
                    return Some(format!("```php\n{}::{}\n```", en.name, case_info.name));
                }
            }
        }

        for alias in &self.type_aliases {
            if span_contains(alias.span, offset) {
                return Some(format!("```php\ntype {} = {}\n```", alias.name, alias.ty));
            }
        }

        for konst in &self.consts {
            if span_contains(konst.span, offset) {
                return Some(format!("```php\nconst {}\n```", konst.name));
            }
        }

        None
    }

    fn definition_at(
        &self,
        offset: usize,
        uri: &Url,
        line_index: &LineIndex,
        source: &[u8],
    ) -> Option<Location> {
        let word = word_at_offset(source, offset);
        let Some(word) = word else {
            return None;
        };

        if word.starts_with('$') {
            for func in &self.functions {
                if span_contains(func.scope_span, offset) {
                    let mut best: Option<&VarInfo> = None;
                    for var in &func.vars {
                        if var.name == word && var.span.start <= offset {
                            if best.is_none() || var.span.start > best.unwrap().span.start {
                                best = Some(var);
                            }
                        }
                    }
                    if let Some(var) = best {
                        return Some(Location {
                            uri: uri.clone(),
                            range: span_to_range(var.span, line_index),
                        });
                    }
                    return None;
                }
            }
            let mut best: Option<&VarInfo> = None;
            for var in &self.globals {
                if var.name == word && var.span.start <= offset {
                    if best.is_none() || var.span.start > best.unwrap().span.start {
                        best = Some(var);
                    }
                }
            }
            if let Some(var) = best {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(var.span, line_index),
                });
            }
        }

        for func in &self.functions {
            if func.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(func.span, line_index),
                });
            }
        }
        for strukt in &self.structs {
            if strukt.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(strukt.span, line_index),
                });
            }
        }
        for iface in &self.interfaces {
            if iface.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(iface.span, line_index),
                });
            }
        }
        for en in &self.enums {
            if en.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(en.span, line_index),
                });
            }
        }
        for alias in &self.type_aliases {
            if alias.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(alias.span, line_index),
                });
            }
        }
        for konst in &self.consts {
            if konst.name == word {
                return Some(Location {
                    uri: uri.clone(),
                    range: span_to_range(konst.span, line_index),
                });
            }
        }
        None
    }

    #[allow(deprecated)]
    fn document_symbols(&self, line_index: &LineIndex) -> Vec<DocumentSymbol> {
        let mut symbols = Vec::new();
        for func in &self.functions {
            symbols.push(DocumentSymbol {
                name: func.name.clone(),
                detail: Some(func.signature.clone()),
                kind: SymbolKind::FUNCTION,
                range: span_to_range(func.scope_span, line_index),
                selection_range: span_to_range(func.span, line_index),
                children: None,
                deprecated: None,
                tags: None,
            });
        }
        for strukt in &self.structs {
            let fields = strukt
                .fields
                .iter()
                .map(|field| DocumentSymbol {
                    name: field.name.trim_start_matches('$').to_string(),
                    detail: field.ty.clone(),
                    kind: SymbolKind::FIELD,
                    range: span_to_range(field.span, line_index),
                    selection_range: span_to_range(field.span, line_index),
                    children: None,
                    deprecated: None,
                    tags: None,
                })
                .collect();
            symbols.push(DocumentSymbol {
                name: strukt.name.clone(),
                detail: Some("struct".to_string()),
                kind: SymbolKind::STRUCT,
                range: span_to_range(strukt.span, line_index),
                selection_range: span_to_range(strukt.span, line_index),
                children: Some(fields),
                deprecated: None,
                tags: None,
            });
        }
        for iface in &self.interfaces {
            let fields = iface
                .fields
                .iter()
                .map(|field| DocumentSymbol {
                    name: field.name.trim_start_matches('$').to_string(),
                    detail: field.ty.clone(),
                    kind: SymbolKind::FIELD,
                    range: span_to_range(field.span, line_index),
                    selection_range: span_to_range(field.span, line_index),
                    children: None,
                    deprecated: None,
                    tags: None,
                })
                .collect();
            symbols.push(DocumentSymbol {
                name: iface.name.clone(),
                detail: Some("interface".to_string()),
                kind: SymbolKind::INTERFACE,
                range: span_to_range(iface.span, line_index),
                selection_range: span_to_range(iface.span, line_index),
                children: Some(fields),
                deprecated: None,
                tags: None,
            });
        }
        for en in &self.enums {
            let cases = en
                .cases
                .iter()
                .map(|case_info| DocumentSymbol {
                    name: case_info.name.clone(),
                    detail: Some("case".to_string()),
                    kind: SymbolKind::ENUM_MEMBER,
                    range: span_to_range(case_info.span, line_index),
                    selection_range: span_to_range(case_info.span, line_index),
                    children: None,
                    deprecated: None,
                    tags: None,
                })
                .collect();
            symbols.push(DocumentSymbol {
                name: en.name.clone(),
                detail: Some("enum".to_string()),
                kind: SymbolKind::ENUM,
                range: span_to_range(en.span, line_index),
                selection_range: span_to_range(en.span, line_index),
                children: Some(cases),
                deprecated: None,
                tags: None,
            });
        }
        for alias in &self.type_aliases {
            symbols.push(DocumentSymbol {
                name: alias.name.clone(),
                detail: Some(alias.ty.clone()),
                kind: SymbolKind::TYPE_PARAMETER,
                range: span_to_range(alias.span, line_index),
                selection_range: span_to_range(alias.span, line_index),
                children: None,
                deprecated: None,
                tags: None,
            });
        }
        for konst in &self.consts {
            symbols.push(DocumentSymbol {
                name: konst.name.clone(),
                detail: Some("const".to_string()),
                kind: SymbolKind::CONSTANT,
                range: span_to_range(konst.span, line_index),
                selection_range: span_to_range(konst.span, line_index),
                children: None,
                deprecated: None,
                tags: None,
            });
        }
        symbols
    }

    fn var_type_at(&self, offset: usize, name: &str) -> Option<String> {
        for func in &self.functions {
            if span_contains(func.scope_span, offset) {
                let mut best: Option<&VarInfo> = None;
                for var in &func.vars {
                    if var.name == name && var.span.start <= offset {
                        if best.is_none() || var.span.start > best.unwrap().span.start {
                            best = Some(var);
                        }
                    }
                }
                return best.and_then(|var| var.ty.clone());
            }
        }
        let mut best: Option<&VarInfo> = None;
        for var in &self.globals {
            if var.name == name && var.span.start <= offset {
                if best.is_none() || var.span.start > best.unwrap().span.start {
                    best = Some(var);
                }
            }
        }
        best.and_then(|var| var.ty.clone())
    }

    fn fields_for_type(&self, ty: &str) -> Option<Vec<String>> {
        if let Some(strukt) = self.structs.iter().find(|s| s.name == ty) {
            return Some(
                strukt
                    .fields
                    .iter()
                    .map(|field| field.name.trim_start_matches('$').to_string())
                    .collect(),
            );
        }
        if let Some(iface) = self.interfaces.iter().find(|i| i.name == ty) {
            return Some(
                iface
                    .fields
                    .iter()
                    .map(|field| field.name.trim_start_matches('$').to_string())
                    .collect(),
            );
        }
        if let Some(alias) = self.type_aliases.iter().find(|alias| alias.name == ty) {
            return self.fields_for_type(alias.ty.trim());
        }
        if let Some(inner) = ty
            .strip_prefix("Option<")
            .and_then(|value| value.strip_suffix('>'))
        {
            return self.fields_for_type(inner.trim());
        }
        if let Some(inner) = ty
            .strip_prefix("Result<")
            .and_then(|value| value.strip_suffix('>'))
        {
            let first = inner.split(',').next().unwrap_or(inner).trim();
            return self.fields_for_type(first);
        }
        if let Some(inner) = ty
            .strip_prefix("Object<{")
            .and_then(|value| value.strip_suffix("}>"))
        {
            let mut fields = Vec::new();
            for part in inner.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let name = part
                    .split(':')
                    .next()
                    .unwrap_or(part)
                    .trim()
                    .trim_end_matches('?');
                if !name.is_empty() {
                    fields.push(name.to_string());
                }
            }
            return Some(fields);
        }
        None
    }

    fn component_prop_fields(&self, component: &str) -> Option<Vec<(String, Option<String>)>> {
        let func = self.functions.iter().find(|func| func.name == component)?;
        let ty = func.props_type.as_deref()?.trim();
        self.fields_for_type_with_types(ty)
    }

    fn fields_for_type_with_types(&self, ty: &str) -> Option<Vec<(String, Option<String>)>> {
        if let Some(strukt) = self.structs.iter().find(|s| s.name == ty) {
            return Some(
                strukt
                    .fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.trim_start_matches('$').to_string(),
                            field.ty.clone(),
                        )
                    })
                    .collect(),
            );
        }
        if let Some(iface) = self.interfaces.iter().find(|i| i.name == ty) {
            return Some(
                iface
                    .fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.trim_start_matches('$').to_string(),
                            field.ty.clone(),
                        )
                    })
                    .collect(),
            );
        }
        if let Some(alias) = self.type_aliases.iter().find(|alias| alias.name == ty) {
            return self.fields_for_type_with_types(alias.ty.trim());
        }
        if let Some(inner) = ty
            .strip_prefix("Option<")
            .and_then(|value| value.strip_suffix('>'))
        {
            return self.fields_for_type_with_types(inner.trim());
        }
        if let Some(inner) = ty
            .strip_prefix("Result<")
            .and_then(|value| value.strip_suffix('>'))
        {
            let first = inner.split(',').next().unwrap_or(inner).trim();
            return self.fields_for_type_with_types(first);
        }
        if let Some(inner) = ty
            .strip_prefix("Object<{")
            .and_then(|value| value.strip_suffix("}>"))
        {
            let mut fields = Vec::new();
            for part in inner.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let mut segments = part.split(':');
                let name = segments.next().unwrap_or(part).trim().trim_end_matches('?');
                let ty = segments.next().map(|t| t.trim().to_string());
                if !name.is_empty() {
                    fields.push((name.to_string(), ty));
                }
            }
            return Some(fields);
        }
        None
    }
}

fn build_index(program: &Program, source: &[u8]) -> SymbolIndex {
    let mut index = SymbolIndex::default();
    for stmt in program.statements.iter() {
        collect_stmt(stmt, source, &mut index);
    }
    index
}

fn collect_stmt(stmt: StmtId, source: &[u8], index: &mut SymbolIndex) {
    match stmt {
        Stmt::Expression { expr, .. } => {
            collect_vars_in_expr(expr, source, &mut index.globals);
        }
        Stmt::Return {
            expr: Some(expr), ..
        } => {
            collect_vars_in_expr(expr, source, &mut index.globals);
        }
        Stmt::Function {
            name,
            is_async,
            params,
            return_type,
            body,
            span,
            ..
        } => {
            let fn_name = token_text(source, name);
            let signature = format_function_signature(
                fn_name.as_str(),
                params,
                *return_type,
                source,
                *is_async,
            );
            let mut vars = Vec::new();
            for param in *params {
                let param_name = token_text(source, param.name);
                let ty = param.ty.map(|ty| format_type(ty, source));
                vars.push(VarInfo {
                    name: param_name,
                    span: param.name.span,
                    ty,
                });
            }
            collect_vars_in_block(body, source, &mut vars);
            index.functions.push(FunctionInfo {
                name: fn_name,
                span: name.span,
                signature,
                props_type: params
                    .first()
                    .and_then(|param| param.ty.map(|ty| format_type(ty, source))),
                vars,
                scope_span: *span,
            });
        }
        Stmt::Class {
            kind,
            name,
            members,
            span: _,
            ..
        } => {
            if *kind == ClassKind::Struct {
                let struct_name = token_text(source, name);
                let mut fields = Vec::new();
                for member in *members {
                    if let ClassMember::Property { ty, entries, .. } = member {
                        for entry in *entries {
                            let field_name = token_text(source, entry.name);
                            let field_ty = ty.map(|ty| format_type(ty, source));
                            fields.push(FieldInfo {
                                name: field_name,
                                span: entry.name.span,
                                ty: field_ty,
                            });
                        }
                    }
                }
                index.structs.push(StructInfo {
                    name: struct_name,
                    span: name.span,
                    fields,
                });
            }
        }
        Stmt::Interface {
            name,
            members,
            span: _,
            ..
        } => {
            let iface_name = token_text(source, name);
            let mut fields = Vec::new();
            for member in *members {
                if let ClassMember::Property { ty, entries, .. } = member {
                    for entry in *entries {
                        let field_name = token_text(source, entry.name);
                        let field_ty = ty.map(|ty| format_type(ty, source));
                        fields.push(FieldInfo {
                            name: field_name,
                            span: entry.name.span,
                            ty: field_ty,
                        });
                    }
                }
            }
            index.interfaces.push(InterfaceInfo {
                name: iface_name,
                span: name.span,
                fields,
            });
        }
        Stmt::Enum {
            name,
            members,
            span: _,
            ..
        } => {
            let enum_name = token_text(source, name);
            let mut cases = Vec::new();
            for member in *members {
                if let ClassMember::Case { name, .. } = member {
                    cases.push(EnumCaseInfo {
                        name: token_text(source, name),
                        span: name.span,
                    });
                }
            }
            index.enums.push(EnumInfo {
                name: enum_name,
                span: name.span,
                cases,
            });
        }
        Stmt::TypeAlias { name, ty, .. } => {
            let alias_name = token_text(source, name);
            let alias_ty = format_type(ty, source);
            index.type_aliases.push(TypeAliasInfo {
                name: alias_name,
                span: name.span,
                ty: alias_ty,
            });
        }
        Stmt::Const { consts, .. } => {
            for konst in *consts {
                let const_name = token_text(source, konst.name);
                index.consts.push(ConstInfo {
                    name: const_name,
                    span: konst.name.span,
                });
            }
        }
        Stmt::Block { statements, .. } => {
            for stmt in *statements {
                collect_stmt(stmt, source, index);
            }
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            for stmt in *then_block {
                collect_stmt(stmt, source, index);
            }
            if let Some(block) = else_block {
                for stmt in *block {
                    collect_stmt(stmt, source, index);
                }
            }
        }
        Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::For { body, .. }
        | Stmt::Foreach { body, .. } => {
            for stmt in *body {
                collect_stmt(stmt, source, index);
            }
        }
        _ => {}
    }
}

fn collect_vars_in_block(body: &[StmtId], source: &[u8], vars: &mut Vec<VarInfo>) {
    for stmt in body {
        match stmt {
            Stmt::Expression { expr, .. } => collect_vars_in_expr(expr, source, vars),
            Stmt::Return {
                expr: Some(expr), ..
            } => collect_vars_in_expr(expr, source, vars),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_vars_in_block(then_block, source, vars);
                if let Some(block) = else_block {
                    collect_vars_in_block(block, source, vars);
                }
            }
            Stmt::While { body, .. }
            | Stmt::DoWhile { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Foreach { body, .. }
            | Stmt::Block {
                statements: body, ..
            } => {
                collect_vars_in_block(body, source, vars);
            }
            _ => {}
        }
    }
}

fn collect_vars_in_expr(expr: ExprId, source: &[u8], vars: &mut Vec<VarInfo>) {
    match expr {
        Expr::Assign { var, expr, .. }
        | Expr::AssignOp { var, expr, .. }
        | Expr::AssignRef { var, expr, .. } => {
            if let Some((name, span)) = variable_name(var, source) {
                let ty = infer_expr_type(expr, source);
                vars.push(VarInfo { name, span, ty });
            }
            collect_vars_in_expr(expr, source, vars);
        }
        Expr::Binary { left, right, .. } => {
            collect_vars_in_expr(left, source, vars);
            collect_vars_in_expr(right, source, vars);
        }
        Expr::Unary { expr, .. } => collect_vars_in_expr(expr, source, vars),
        Expr::Call { func, args, .. } => {
            collect_vars_in_expr(func, source, vars);
            for arg in *args {
                collect_vars_in_expr(arg.value, source, vars);
            }
        }
        Expr::Array { items, .. } => {
            for item in *items {
                collect_vars_in_expr(item.value, source, vars);
            }
        }
        Expr::ObjectLiteral { items, .. } => {
            for item in *items {
                collect_vars_in_expr(item.value, source, vars);
            }
        }
        Expr::StructLiteral { fields, .. } => {
            for field in *fields {
                collect_vars_in_expr(field.value, source, vars);
            }
        }
        Expr::JsxElement { children, .. } | Expr::JsxFragment { children, .. } => {
            for child in *children {
                if let php_rs::parser::ast::JsxChild::Expr(expr) = child {
                    collect_vars_in_expr(expr, source, vars);
                }
            }
        }
        Expr::ArrayDimFetch { array, dim, .. } => {
            collect_vars_in_expr(array, source, vars);
            if let Some(dim) = dim {
                collect_vars_in_expr(dim, source, vars);
            }
        }
        Expr::PropertyFetch {
            target, property, ..
        } => {
            collect_vars_in_expr(target, source, vars);
            collect_vars_in_expr(property, source, vars);
        }
        Expr::MethodCall {
            target,
            method,
            args,
            ..
        } => {
            collect_vars_in_expr(target, source, vars);
            collect_vars_in_expr(method, source, vars);
            for arg in *args {
                collect_vars_in_expr(arg.value, source, vars);
            }
        }
        Expr::StaticCall {
            class,
            method,
            args,
            ..
        } => {
            collect_vars_in_expr(class, source, vars);
            collect_vars_in_expr(method, source, vars);
            for arg in *args {
                collect_vars_in_expr(arg.value, source, vars);
            }
        }
        Expr::ClassConstFetch {
            class, constant, ..
        } => {
            collect_vars_in_expr(class, source, vars);
            collect_vars_in_expr(constant, source, vars);
        }
        Expr::New { class, args, .. } => {
            collect_vars_in_expr(class, source, vars);
            for arg in *args {
                collect_vars_in_expr(arg.value, source, vars);
            }
        }
        Expr::Ternary {
            condition,
            if_true,
            if_false,
            ..
        } => {
            collect_vars_in_expr(condition, source, vars);
            if let Some(expr) = if_true {
                collect_vars_in_expr(expr, source, vars);
            }
            collect_vars_in_expr(if_false, source, vars);
        }
        Expr::Include { expr, .. } => collect_vars_in_expr(expr, source, vars),
        Expr::DotAccess { target, .. } => collect_vars_in_expr(target, source, vars),
        Expr::PostInc { var, .. } | Expr::PostDec { var, .. } => {
            collect_vars_in_expr(var, source, vars)
        }
        Expr::InterpolatedString { parts, .. } | Expr::ShellExec { parts, .. } => {
            for part in *parts {
                collect_vars_in_expr(part, source, vars);
            }
        }
        _ => {}
    }
}

fn variable_name(expr: ExprId, source: &[u8]) -> Option<(String, Span)> {
    match expr {
        Expr::Variable { name, .. } => {
            let text = span_text(source, name);
            Some((text, *name))
        }
        _ => None,
    }
}

fn infer_expr_type(expr: ExprId, source: &[u8]) -> Option<String> {
    match expr {
        Expr::Integer { .. } => Some("int".to_string()),
        Expr::Float { .. } => Some("float".to_string()),
        Expr::Boolean { .. } => Some("bool".to_string()),
        Expr::String { .. } | Expr::InterpolatedString { .. } => Some("string".to_string()),
        Expr::Array { .. } => Some("array".to_string()),
        Expr::ObjectLiteral { items, .. } => {
            let mut fields = Vec::new();
            for item in *items {
                let key = match item.key {
                    ObjectKey::Ident(token) => token_text(source, token),
                    ObjectKey::String(token) => decode_string_key(&token_text(source, token)),
                };
                if !key.is_empty() {
                    fields.push(format!("{}: mixed", key));
                }
            }
            if fields.is_empty() {
                Some("Object".to_string())
            } else {
                Some(format!("Object<{{{}}}>", fields.join(", ")))
            }
        }
        Expr::StructLiteral { name, .. } => Some(name_text(source, name)),
        Expr::JsxElement { .. } | Expr::JsxFragment { .. } => Some("VNode".to_string()),
        Expr::Null { .. } => Some("null".to_string()),
        Expr::Binary {
            op: BinaryOp::Coalesce,
            left,
            right,
            ..
        } => {
            let left_ty = infer_expr_type(left, source);
            let right_ty = infer_expr_type(right, source);
            match (left_ty, right_ty) {
                (Some(left), Some(right)) if left == right => Some(left),
                (Some(left), Some(right)) => Some(format!("{} | {}", left, right)),
                (Some(left), None) => Some(left),
                (None, Some(right)) => Some(right),
                (None, None) => None,
            }
        }
        _ => None,
    }
}

fn format_function_signature(
    name: &str,
    params: &[Param],
    return_type: Option<&Type>,
    source: &[u8],
    is_async: bool,
) -> String {
    let mut sig = if is_async {
        format!("async function {}(", name)
    } else {
        format!("function {}(", name)
    };
    let mut first = true;
    for param in params {
        if !first {
            sig.push_str(", ");
        }
        first = false;
        if let Some(ty) = param.ty {
            sig.push_str(&format_type(ty, source));
            sig.push(' ');
        }
        sig.push_str(&token_text(source, param.name));
    }
    sig.push(')');
    if let Some(ty) = return_type {
        sig.push_str(": ");
        sig.push_str(&format_type(ty, source));
    }
    sig
}

fn format_type(ty: &Type, source: &[u8]) -> String {
    match ty {
        Type::Simple(token) => token_text(source, token),
        Type::Name(name) => name_text(source, name),
        Type::Union(types) => types
            .iter()
            .map(|ty| format_type(ty, source))
            .collect::<Vec<_>>()
            .join(" | "),
        Type::Intersection(types) => types
            .iter()
            .map(|ty| format_type(ty, source))
            .collect::<Vec<_>>()
            .join(" & "),
        Type::Nullable(inner) => format!("?{}", format_type(inner, source)),
        Type::ObjectShape(fields) => {
            let mut out = String::from("Object<{");
            let mut first = true;
            for field in *fields {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                out.push_str(&token_text(source, field.name));
                if field.optional {
                    out.push('?');
                }
                out.push_str(": ");
                out.push_str(&format_type(field.ty, source));
            }
            out.push_str("}>");
            out
        }
        Type::Applied { base, args } => {
            let mut out = format_type(base, source);
            let rendered = args
                .iter()
                .map(|ty| format_type(ty, source))
                .collect::<Vec<_>>();
            out.push('<');
            out.push_str(&rendered.join(", "));
            out.push('>');
            out
        }
    }
}

fn name_text(source: &[u8], name: &Name) -> String {
    String::from_utf8_lossy(name.span.as_str(source)).to_string()
}

fn token_text(source: &[u8], token: &Token) -> String {
    String::from_utf8_lossy(token.text(source)).to_string()
}

fn span_text(source: &[u8], span: &Span) -> String {
    String::from_utf8_lossy(span.as_str(source)).to_string()
}

// Keep in sync with `decode_string_key` in
// runtime/crates/phpx_js/src/lib.rs and `parse_string_key` in
// runtime/crates/php-rs/src/phpx/typeck/check.rs. ObjectKey::String tokens
// retain their surrounding quotes and embedded escapes — we strip the matched
// quote pair and decode the few escapes the lexer accepts inside string keys.
fn decode_string_key(raw: &str) -> String {
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            let inner = &raw[1..raw.len() - 1];
            return unescape_string_key(inner, first == b'"');
        }
    }
    raw.to_string()
}

fn unescape_string_key(value: &str, double_quoted: bool) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        match next {
            '\'' if !double_quoted => out.push('\''),
            '"' if double_quoted => out.push('"'),
            '\\' => out.push('\\'),
            'n' if double_quoted => out.push('\n'),
            'r' if double_quoted => out.push('\r'),
            't' if double_quoted => out.push('\t'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

fn span_contains(span: Span, offset: usize) -> bool {
    span.start <= offset && offset < span.end
}

fn span_to_range(span: Span, line_index: &LineIndex) -> Range {
    Range {
        start: line_index.offset_to_position(span.start),
        end: line_index.offset_to_position(span.end),
    }
}

fn word_at_offset(source: &[u8], offset: usize) -> Option<String> {
    let bytes = source;
    if offset >= bytes.len() {
        return None;
    }
    let mut start = offset;
    let mut end = offset;
    while start > 0 && is_ident_char(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && is_ident_char(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    let word = &bytes[start..end];
    if word.iter().all(|b| b.is_ascii_whitespace()) {
        return None;
    }
    Some(String::from_utf8_lossy(word).to_string())
}

fn word_before_dot(source: &[u8], offset: usize) -> Option<String> {
    if offset == 0 || source.get(offset - 1) != Some(&b'.') {
        return None;
    }
    if offset < 2 || !is_ident_char(source[offset - 2]) {
        return None;
    }
    let mut start = offset - 2;
    while start > 0 && is_ident_char(source[start - 1]) {
        start -= 1;
    }
    let end = offset - 1;
    if start >= end {
        return None;
    }
    Some(String::from_utf8_lossy(&source[start..end]).to_string())
}

fn is_ident_char(byte: u8) -> bool {
    byte == b'$'
        || byte == b'_'
        || (byte >= b'0' && byte <= b'9')
        || (byte >= b'a' && byte <= b'z')
        || (byte >= b'A' && byte <= b'Z')
        || byte == b'\\'
}

fn hover_from_import(source: &str, offset: usize) -> Option<String> {
    let imports = parse_imports(source);
    for import in imports {
        if import.span.start <= offset && offset < import.span.end {
            let line = if import.imported == "default" {
                format!("import {} from '{}'", import.local, import.from)
            } else {
                format!("import {{ {} }} from '{}'", import.local, import.from)
            };
            return Some(format!("```php\n{}\n```", line));
        }
    }
    None
}

fn hover_for_annotation(source: &str, offset: usize) -> Option<String> {
    let name = annotation_name_at_offset(source, offset)?;
    let (_, detail) = annotation_catalog()
        .into_iter()
        .find(|(label, _)| *label == name.as_str())?;
    Some(format!("```php\n@{}\n```\n{}", name, detail))
}

fn annotation_name_at_offset(source: &str, offset: usize) -> Option<String> {
    let bytes = source.as_bytes();
    if offset > bytes.len() {
        return None;
    }
    let mut start = offset.min(bytes.len());
    while start > 0 && is_ident_char(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = offset.min(bytes.len());
    while end < bytes.len() && is_ident_char(bytes[end]) {
        end += 1;
    }
    if start >= end {
        return None;
    }
    if start == 0 || bytes[start - 1] != b'@' {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[start..end]).to_string())
}

fn definition_for_import_module(
    source: &str,
    file_path: &str,
    offset: usize,
    workspace_roots: &[PathBuf],
) -> Option<Location> {
    let imports = parse_imports(source);
    for import in imports {
        if let Some(module_span) = import.module_span {
            if module_span.start <= offset && offset < module_span.end {
                let root = find_php_modules_root(Path::new(file_path), workspace_roots)?;
                let path = resolve_module_file(&root, &import.from, import.is_wasm)?;
                let uri = Url::from_file_path(path).ok()?;
                return Some(Location {
                    uri,
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                });
            }
        }
    }
    None
}

fn definition_for_imported_symbol(
    source: &str,
    file_path: &str,
    symbol: &str,
    workspace_roots: &[PathBuf],
) -> Option<Location> {
    let imports = parse_imports(source);
    for import in imports {
        if import.local != symbol {
            continue;
        }
        let root = find_php_modules_root(Path::new(file_path), workspace_roots)?;
        let path = resolve_module_file(&root, &import.from, import.is_wasm)?;
        let module_source = fs::read_to_string(&path).ok()?;
        let range = export_range_for_symbol(&module_source, &import.imported).unwrap_or(Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        });
        let uri = Url::from_file_path(path).ok()?;
        return Some(Location { uri, range });
    }
    None
}

fn export_range_for_symbol(source: &str, symbol: &str) -> Option<Range> {
    let line_index = LineIndex::new(source);
    let mut offset = 0usize;
    for line in source.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("export ") {
            offset += line.len() + 1;
            continue;
        }
        let rest = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        if let Some(open) = rest.find('{') {
            let close = rest.rfind('}').unwrap_or(rest.len());
            let inner = &rest[open + 1..close];
            for spec in inner.split(',') {
                let spec = spec.trim();
                if spec.is_empty() {
                    continue;
                }
                let (imported, local) = if let Some((left, right)) = spec.split_once(" as ") {
                    (left.trim(), right.trim())
                } else {
                    (spec, spec)
                };
                let name = if symbol == local {
                    local
                } else if symbol == imported {
                    imported
                } else {
                    continue;
                };
                if let Some(col) = line.find(name) {
                    let span = Span::new(offset + col, offset + col + name.len());
                    return Some(span_to_range(span, &line_index));
                }
            }
            offset += line.len() + 1;
            continue;
        }
        let keywords = ["function ", "const ", "type ", "struct ", "enum "];
        for keyword in keywords {
            if let Some(name) = rest.strip_prefix(keyword) {
                if let Some(token) = name.split_whitespace().next() {
                    if token == symbol {
                        if let Some(col) = line.find(token) {
                            let span = Span::new(offset + col, offset + col + token.len());
                            return Some(span_to_range(span, &line_index));
                        }
                    }
                }
            }
        }
        offset += line.len() + 1;
    }
    None
}

fn format_external_signature(name: &str, sig: &ExternalFunctionSig) -> String {
    let mut out = String::from("function ");
    out.push_str(name);
    out.push('(');
    for (idx, param) in sig.params.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        if sig.variadic && idx == sig.params.len().saturating_sub(1) {
            out.push_str("...");
        }
        let ty = param
            .ty
            .as_ref()
            .map(format_php_type)
            .unwrap_or_else(|| "mixed".to_string());
        let name = format!("$arg{}", idx + 1);
        out.push_str(&format!("{ty} {name}"));
        if !param.required {
            out.push_str(" = ?");
        }
    }
    out.push(')');
    if let Some(ret) = &sig.return_type {
        out.push_str(": ");
        out.push_str(&format_php_type(ret));
    }
    out
}

fn format_php_type(ty: &PhpType) -> String {
    ty.name()
}

struct ImportInfo {
    imported: String,
    local: String,
    from: String,
    span: Span,
    module_span: Option<Span>,
    is_wasm: bool,
}

fn parse_imports(source: &str) -> Vec<ImportInfo> {
    let mut imports = Vec::new();
    let mut offset = 0usize;
    for line in source.lines() {
        let line_len = line.len();
        let trimmed = line.trim_start();
        if trimmed.starts_with("import ") {
            let is_wasm = line.contains(" as wasm");
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

            if let Some(name) = default_name {
                if let Some(col) = line.find(name) {
                    let span = Span::new(offset + col, offset + col + name.len());
                    if let Some((module, module_span)) = module_info.clone() {
                        imports.push(ImportInfo {
                            imported: "default".to_string(),
                            local: name.to_string(),
                            from: module,
                            span,
                            module_span: Some(module_span),
                            is_wasm,
                        });
                    }
                }
            }

            if let Some(spec_part) = spec_part {
                if let (Some(open), Some(_close)) = (line.find('{'), line.find('}')) {
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
                            let span =
                                Span::new(offset + local_pos, offset + local_pos + local.len());
                            if let Some((module, module_span)) = module_info.clone() {
                                imports.push(ImportInfo {
                                    imported: imported.to_string(),
                                    local: local.to_string(),
                                    from: module,
                                    span,
                                    module_span: Some(module_span),
                                    is_wasm,
                                });
                            } else {
                                imports.push(ImportInfo {
                                    imported: imported.to_string(),
                                    local: local.to_string(),
                                    from: imported.to_string(),
                                    span,
                                    module_span: None,
                                    is_wasm,
                                });
                            }
                        }
                        cursor += spec.len() + 1;
                    }
                }
            }
        }
        offset += line_len + 1;
    }
    imports
}

fn parse_ident_from_str(input: &str) -> Option<(&str, &str)> {
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

fn parse_module_path(line: &str) -> Option<String> {
    let from_idx = line.find("from")?;
    let rest = &line[from_idx + 4..];
    let quote = rest.find(&['\'', '"'][..])?;
    let quote_char = rest.chars().nth(quote)?;
    let after = &rest[quote + 1..];
    let end = after.find(quote_char)?;
    Some(after[..end].to_string())
}

fn parse_module_path_with_span(line: &str, line_offset: usize) -> Option<(String, Span)> {
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

fn import_module_at_offset(source: &str, offset: usize) -> Option<String> {
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

fn import_module_spans(source: &str, module_spec: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    for (line, line_offset) in line_with_offsets(source) {
        if !line.contains("import") || !line.contains("from") {
            continue;
        }
        if let Some((found, span)) = parse_module_path_with_span(line, line_offset) {
            if found == module_spec {
                spans.push(span);
            }
        }
    }
    spans
}

fn line_with_offsets(source: &str) -> Vec<(&str, usize)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for line in source.split('\n') {
        out.push((line, offset));
        offset += line.len() + 1;
    }
    out
}

struct ExportInfo {
    name: String,
    kind: Option<CompletionItemKind>,
}

fn module_exports(root: &Path, module_spec: &str, is_wasm: bool) -> Option<Vec<ExportInfo>> {
    let path = resolve_module_file(root, module_spec, is_wasm)?;
    let source = fs::read_to_string(path).ok()?;
    Some(parse_exported_names(&source))
}

fn parse_exported_names(source: &str) -> Vec<ExportInfo> {
    let mut exports = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("export ") {
            continue;
        }
        let rest = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        if let Some(name) = rest.strip_prefix("function ") {
            if let Some(token) = export_token(name) {
                exports.push(ExportInfo {
                    name: token,
                    kind: Some(CompletionItemKind::FUNCTION),
                });
            }
            continue;
        }
        if let Some(open) = rest.find('{') {
            let close = rest.rfind('}').unwrap_or(rest.len());
            let inner = &rest[open + 1..close];
            for spec in inner.split(',') {
                let spec = spec.trim();
                if spec.is_empty() {
                    continue;
                }
                let name = if let Some((_, alias)) = spec.split_once(" as ") {
                    alias.trim()
                } else {
                    spec
                };
                if !name.is_empty() {
                    exports.push(ExportInfo {
                        name: name.to_string(),
                        kind: Some(CompletionItemKind::VARIABLE),
                    });
                }
            }
            continue;
        }
        if let Some(name) = rest.strip_prefix("const ") {
            if let Some(token) = export_token(name) {
                exports.push(ExportInfo {
                    name: token,
                    kind: Some(CompletionItemKind::CONSTANT),
                });
            }
            continue;
        }
        if let Some(name) = rest.strip_prefix("type ") {
            if let Some(token) = export_token(name) {
                exports.push(ExportInfo {
                    name: token,
                    kind: Some(CompletionItemKind::TYPE_PARAMETER),
                });
            }
            continue;
        }
        if let Some(name) = rest.strip_prefix("struct ") {
            if let Some(token) = export_token(name) {
                exports.push(ExportInfo {
                    name: token,
                    kind: Some(CompletionItemKind::STRUCT),
                });
            }
            continue;
        }
        if let Some(name) = rest.strip_prefix("enum ") {
            if let Some(token) = export_token(name) {
                exports.push(ExportInfo {
                    name: token,
                    kind: Some(CompletionItemKind::ENUM),
                });
            }
            continue;
        }
    }
    exports
}

fn export_token(input: &str) -> Option<String> {
    let raw = input.trim_start().split_whitespace().next()?;
    let cleaned = raw
        .trim_end_matches(|c: char| c == ';' || c == ',' || c == '{')
        .split('(')
        .next()
        .unwrap_or(raw)
        .trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

fn completion_for_import(
    source: &str,
    file_path: &str,
    offset: usize,
    workspace_roots: &[PathBuf],
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
            let root = find_php_modules_root(Path::new(file_path), workspace_roots)?;
            let is_wasm = line_text.contains(" as wasm");
            let exports = module_exports(&root, &module_spec, is_wasm)?;
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
            return Some(items);
        }
    }

    let before_cursor = &source[line_start..offset.min(source.len())];
    let quote_pos = before_cursor.rfind(&['\'', '"'][..])?;
    let prefix = &before_cursor[quote_pos + 1..];

    let root = find_php_modules_root(Path::new(file_path), workspace_roots)?;
    let modules = if prefix.starts_with("@/") {
        list_project_modules(root.parent()?)
            .into_iter()
            .map(|name| format!("@/{}", name))
            .collect::<Vec<_>>()
    } else {
        list_php_modules(&root)
    };
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

fn unresolved_import_diagnostics(
    source: &str,
    file_path: &str,
    workspace_roots: &[PathBuf],
) -> Vec<Diagnostic> {
    let root = match find_php_modules_root(Path::new(file_path), workspace_roots) {
        Some(root) => root,
        None => return Vec::new(),
    };

    let imports = parse_imports(source);
    if imports.is_empty() {
        return Vec::new();
    }

    let line_index = LineIndex::new(source);
    let mut module_cache: HashMap<(String, bool), Option<Vec<ExportInfo>>> = HashMap::new();
    let mut diagnostics = Vec::new();

    for import in imports {
        if import.imported == "default" {
            continue;
        }

        let key = (import.from.clone(), import.is_wasm);
        let exports = module_cache
            .entry(key)
            .or_insert_with(|| module_exports(&root, &import.from, import.is_wasm));
        let Some(exports) = exports else {
            continue;
        };

        let found = exports.iter().any(|export| export.name == import.imported);
        if found {
            continue;
        }

        diagnostics.push(Diagnostic {
            range: span_to_range(import.span, &line_index),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(tower_lsp::lsp_types::NumberOrString::String(
                "Import Error".to_string(),
            )),
            source: Some("phpx".to_string()),
            message: format!(
                "Import Error: Module '{}' has no export named '{}'.",
                import.from, import.imported
            ),
            ..Diagnostic::default()
        });
    }

    diagnostics
}

fn target_capability_diagnostics(source: &str, target_mode: TargetMode) -> Vec<Diagnostic> {
    if target_mode == TargetMode::Server {
        return Vec::new();
    }

    let imports = parse_imports(source);
    if imports.is_empty() {
        return Vec::new();
    }

    let line_index = LineIndex::new(source);
    let mut diagnostics = Vec::new();
    for import in imports {
        if let Some(block) = adwa_capability_block(&import.from) {
            diagnostics.push(Diagnostic {
                range: span_to_range(import.module_span.unwrap_or(import.span), &line_index),
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(tower_lsp::lsp_types::NumberOrString::String(
                    "Target Capability Error".to_string(),
                )),
                source: Some("phpx".to_string()),
                message: format!(
                    "Target Capability Error: Module '{}' is unavailable for target 'adwa' ({}).\nhelp: {}",
                    import.from, block.reason, block.suggestion
                ),
                ..Diagnostic::default()
            });
        }
    }
    diagnostics
}

struct CapabilityBlock {
    reason: &'static str,
    suggestion: &'static str,
}

fn adwa_capability_block(module_spec: &str) -> Option<CapabilityBlock> {
    if module_spec == "db"
        || module_spec.starts_with("db/")
        || module_spec == "postgres"
        || module_spec.starts_with("postgres/")
        || module_spec == "mysql"
        || module_spec.starts_with("mysql/")
        || module_spec == "sqlite"
        || module_spec.starts_with("sqlite/")
    {
        return Some(CapabilityBlock {
            reason: "database host capability is disabled",
            suggestion: "Run with `phpx.target = server` or move database access behind a server endpoint.",
        });
    }
    if module_spec == "process"
        || module_spec.starts_with("process/")
        || module_spec == "env"
        || module_spec.starts_with("env/")
    {
        return Some(CapabilityBlock {
            reason: "process/env host capability is disabled",
            suggestion: "Inject values through app config/context instead of reading process/env in `adwa`.",
        });
    }
    None
}

fn completion_for_annotation(source: &str, offset: usize) -> Option<Vec<CompletionItem>> {
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

fn annotation_catalog() -> Vec<(&'static str, &'static str)> {
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

fn completion_for_dot(
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

fn completion_for_jsx_props(
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

fn jsx_attr_default_value(ty: Option<&str>) -> &'static str {
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

fn builtin_completion_items() -> Vec<CompletionItem> {
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

fn stdlib_completion_items() -> Vec<CompletionItem> {
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

fn snippet_completion_items() -> Vec<CompletionItem> {
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

fn find_php_modules_root(start: &Path, workspace_roots: &[PathBuf]) -> Option<PathBuf> {
    if let Ok(root) = std::env::var("PHPX_MODULE_ROOT") {
        let root_path = PathBuf::from(root);
        let candidate = root_path.join("php_modules");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let mut current = start.to_path_buf();
    if current.is_file() {
        current.pop();
    }
    loop {
        let candidate = current.join("php_modules");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !current.pop() {
            break;
        }
    }
    for workspace_root in workspace_roots {
        let candidate = workspace_root.join("php_modules");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    if let Ok(current_dir) = std::env::current_dir() {
        let candidate = current_dir.join("php_modules");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

fn list_php_modules(root: &Path) -> Vec<String> {
    let mut modules = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return modules;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if name.starts_with('@') && path.is_dir() {
            if let Ok(children) = fs::read_dir(&path) {
                for child in children.flatten() {
                    let child_name = child.file_name().to_string_lossy().to_string();
                    if child.path().is_dir() {
                        modules.push(format!("{}/{}", name, child_name));
                    }
                }
            }
        } else if path.is_dir() {
            modules.push(name);
        }
    }
    modules.sort();
    modules
}

fn list_project_modules(project_root: &Path) -> Vec<String> {
    let mut modules = Vec::new();
    let Ok(entries) = fs::read_dir(project_root) else {
        return modules;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.')
            || name == "php_modules"
            || name == "target"
            || name == "node_modules"
        {
            continue;
        }
        if path.is_dir() {
            modules.push(name);
        } else if path.is_file() {
            if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
                if ext == "phpx" || ext == "php" {
                    modules.push(name);
                }
            }
        }
    }
    modules.sort();
    modules
}

fn resolve_module_file(root: &Path, module_spec: &str, is_wasm: bool) -> Option<PathBuf> {
    let base = if let Some(rest) = module_spec.strip_prefix("@/") {
        let project_root = root.parent()?;
        project_root.join(rest)
    } else {
        root.join(module_spec)
    };
    if base.is_file() {
        return Some(base);
    }
    let phpx = base.with_extension("phpx");
    if phpx.is_file() {
        return Some(phpx);
    }
    let php = base.with_extension("php");
    if php.is_file() {
        return Some(php);
    }
    if base.is_dir() {
        if is_wasm {
            let stub = base.join("module.d.phpx");
            if stub.is_file() {
                return Some(stub);
            }
        }
        let index = base.join("index.phpx");
        if index.is_file() {
            return Some(index);
        }
        let index_php = base.join("index.php");
        if index_php.is_file() {
            return Some(index_php);
        }
        let module = base.join("module.phpx");
        if module.is_file() {
            return Some(module);
        }
    }
    None
}

fn collect_module_rename_edits(
    roots: &[PathBuf],
    active_uri: &Url,
    active_text: &str,
    old_module: &str,
    new_module: &str,
) -> HashMap<Url, Vec<TextEdit>> {
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for root in roots {
        for file in collect_phpx_files(root) {
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

fn collect_reference_locations(
    roots: &[PathBuf],
    active_uri: &Url,
    active_text: &str,
    symbol: &str,
) -> Vec<Location> {
    let mut locations = Vec::new();
    for root in roots {
        for file in collect_phpx_files(root) {
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
            for span in find_word_occurrences(content.as_bytes(), symbol) {
                locations.push(Location {
                    uri: file_uri.clone(),
                    range: span_to_range(span, &line_index),
                });
            }
        }
    }
    locations
}

fn collect_symbol_rename_edits(
    roots: &[PathBuf],
    active_uri: &Url,
    active_text: &str,
    old_symbol: &str,
    new_symbol: &str,
) -> HashMap<Url, Vec<TextEdit>> {
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for root in roots {
        for file in collect_phpx_files(root) {
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
            for span in find_word_occurrences(content.as_bytes(), old_symbol) {
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
    changes
}

fn collect_phpx_files(root: &Path) -> Vec<PathBuf> {
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
            if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
                if ext == "phpx" || ext == "php" {
                    files.push(path);
                }
            }
        }
    }
    files
}

fn should_skip_dir(path: &Path) -> bool {
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

fn find_word_occurrences(source: &[u8], word: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let needle = word.as_bytes();
    if needle.is_empty() || needle.len() > source.len() {
        return spans;
    }
    let mut offset = 0usize;
    while offset + needle.len() <= source.len() {
        let Some(pos) = source[offset..]
            .windows(needle.len())
            .position(|window| window == needle)
        else {
            break;
        };
        let start = offset + pos;
        let end = start + needle.len();
        let left_ok = start == 0 || !is_ident_char(source[start - 1]);
        let right_ok = end >= source.len() || !is_ident_char(source[end]);
        if left_ok && right_ok {
            spans.push(Span::new(start, end));
        }
        offset = end;
    }
    spans
}

#[cfg(test)]
mod tests;
