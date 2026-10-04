use super::*;
use deka_syntax::parse::FUNCTION_KEYWORD_ERROR;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// didChange validations are debounced so a keystroke burst runs the project
/// graph check once, not per change notification. didOpen validates
/// immediately — the first paint of a file should already carry squiggles.
const VALIDATION_DEBOUNCE: Duration = Duration::from_millis(300);

pub(crate) struct Backend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, String>>>,
    workspace_roots: Arc<RwLock<Vec<PathBuf>>>,
    services: NativeServices,
    /// Monotonic counter bumped on every change; a debounced validation runs
    /// only if it is still the latest.
    validation_seq: Arc<AtomicU64>,
    /// URIs the project-aware path last published diagnostics for, so files
    /// that go clean get an empty publish instead of keeping stale squiggles.
    project_published: Arc<RwLock<HashSet<Url>>>,
}

impl Backend {
    pub(crate) async fn get_document(&self, uri: &Url) -> Option<String> {
        self.documents.read().await.get(uri).cloned()
    }
}

/// The migrated publication lifecycle clears previously reported dependency
/// findings after the native graph becomes valid. No single-file fallback.
async fn validate_documents(
    client: &Client,
    documents: &Arc<RwLock<HashMap<Url, String>>>,
    services: &NativeServices,
    project_published: &Arc<RwLock<HashSet<Url>>>,
    validation_seq: &Arc<AtomicU64>,
) {
    let seq = validation_seq.load(Ordering::SeqCst);
    let docs = documents.read().await.clone();
    let sources = open_document_paths(&docs);
    let services = services.clone();
    let files = tokio::task::spawn_blocking(move || {
        let mut files = std::collections::BTreeMap::<PathBuf, Vec<Diagnostic>>::new();
        for entry in sources.keys() {
            for (path, findings) in file_diagnostics(entry, &sources, &services) {
                let target = files.entry(path).or_default();
                for finding in findings {
                    if !target.contains(&finding) {
                        target.push(finding);
                    }
                }
            }
            files.entry(entry.clone()).or_default();
        }
        files
    })
    .await;
    let files = match files {
        Ok(files) => files,
        Err(error) => {
            client
                .log_message(
                    MessageType::ERROR,
                    format!("Native validation worker failed: {error}"),
                )
                .await;
            return;
        }
    };
    let mut published = project_published.write().await;
    if validation_seq.load(Ordering::SeqCst) != seq {
        return;
    }
    let mut current = HashSet::new();
    for (path, diagnostics) in files {
        let Some(uri) = document_uri(&docs, &path) else {
            continue;
        };
        current.insert(uri.clone());
        client.publish_diagnostics(uri, diagnostics, None).await;
    }
    for stale in published.difference(&current) {
        client
            .publish_diagnostics(stale.clone(), Vec::new(), None)
            .await;
    }
    *published = current;
}

/// Completion items for one document. Import-clause and module-path contexts
/// come first (the imported module's exports, or stdlib items for bare stdlib
/// specifiers), then `@` annotations, then the scope-aware path: component
/// names in JSX tag position, otherwise everything in scope at the cursor —
/// locals/params from the enclosing blocks, top-level items, imports — plus
/// the static builtin/stdlib/snippet lists, all filtered by the identifier
/// prefix before the cursor.
pub(crate) fn entry_completions(
    documents: &HashMap<Url, String>,
    services: &NativeServices,
    text: &str,
    file_path: &str,
    offset: usize,
) -> Vec<CompletionItem> {
    let open_documents = open_document_paths(documents);
    if let Some(items) = completion_for_import(text, file_path, offset, services, &open_documents) {
        return items;
    }
    if let Some(items) = completion_for_annotation(text, offset) {
        return items;
    }

    let arena = bumpalo::Bump::new();
    let program = deka_syntax::parse_recovering(text, &arena).program;
    if let Some(items) = component_completion_items(program.as_ref(), text, offset) {
        return items;
    }

    let prefix = identifier_prefix_at(text, offset);
    let mut items = match &program {
        Some(program) => scope_completion_items(program, offset),
        None => Vec::new(),
    };
    items.extend(services.globals.iter().map(|export| CompletionItem {
        label: export.name.clone(),
        kind: Some(export.kind),
        detail: Some(export.detail.clone()),
        ..CompletionItem::default()
    }));
    items.extend(snippet_completion_items());
    filter_by_prefix(items, prefix)
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(
        &self,
        params: InitializeParams,
    ) -> tower_lsp::jsonrpc::Result<InitializeResult> {
        let mut roots = Vec::new();
        if let Some(folders) = params.workspace_folders {
            for folder in folders {
                if let Ok(path) = folder.uri.to_file_path() {
                    roots.push(path);
                }
            }
        } else if let Some(root_uri) = params.root_uri
            && let Ok(path) = root_uri.to_file_path()
        {
            roots.push(path);
        }
        if roots.is_empty()
            && let Ok(current) = std::env::current_dir()
        {
            roots.push(current);
        }
        *self.workspace_roots.write().await = roots;

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(true.into()),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "'".to_string(),
                        "\"".to_string(),
                        "@".to_string(),
                    ]),
                    ..CompletionOptions::default()
                }),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        identifier: Some(LANGUAGE_ID.to_string()),
                        inter_file_dependencies: true,
                        workspace_diagnostics: false,
                        work_done_progress_options: Default::default(),
                    },
                )),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
                ..ServerCapabilities::default()
            },
            server_info: None,
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "DekaScript LSP initialized")
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
        if !is_dekascript_uri(&uri) {
            return Ok(empty_diagnostic_report());
        }
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
        let mut docs = self.documents.read().await.clone();
        docs.insert(uri.clone(), text);
        let entry = PathBuf::from(&file_path);
        let sources = open_document_paths(&docs);
        let services = self.services.clone();
        let mut files =
            tokio::task::spawn_blocking(move || file_diagnostics(&entry, &sources, &services))
                .await
                .map_err(|_| tower_lsp::jsonrpc::Error::internal_error())?;
        let diagnostics = files
            .remove(&deka_vm::compiler::source_path(Path::new(&file_path)))
            .unwrap_or_default();
        let related_documents = files
            .into_iter()
            .filter_map(|(path, items)| {
                Some((
                    document_uri(&docs, &path)?,
                    tower_lsp::lsp_types::DocumentDiagnosticReportKind::Full(
                        FullDocumentDiagnosticReport {
                            result_id: None,
                            items,
                        },
                    ),
                ))
            })
            .collect::<HashMap<_, _>>();
        Ok(DocumentDiagnosticReportResult::Report(
            DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: (!related_documents.is_empty()).then_some(related_documents),
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: diagnostics,
                },
            }),
        ))
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        if !is_dekascript_uri(&uri) {
            return Ok(None);
        }

        let mut actions = Vec::new();
        for diagnostic in &params.context.diagnostics {
            if !diagnostic.message.contains(FUNCTION_KEYWORD_ERROR) {
                continue;
            }
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: "fn".to_string(),
                }],
            );
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Replace `function` with `fn`".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            }));
        }

        Ok(if actions.is_empty() {
            None
        } else {
            Some(actions)
        })
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        if params.text_document.language_id != LANGUAGE_ID {
            return;
        }
        self.client
            .log_message(
                MessageType::INFO,
                format!("Opened {}", params.text_document.uri),
            )
            .await;

        let uri = params.text_document.uri;
        if !is_dekascript_uri(&uri) {
            return;
        }
        let text = params.text_document.text;
        self.documents.write().await.insert(uri.clone(), text);
        self.validation_seq.fetch_add(1, Ordering::SeqCst);
        validate_documents(
            &self.client,
            &self.documents,
            &self.services,
            &self.project_published,
            &self.validation_seq,
        )
        .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.client
            .log_message(
                MessageType::INFO,
                format!("Changed {}", params.text_document.uri),
            )
            .await;

        let uri = params.text_document.uri;
        if !is_dekascript_uri(&uri) {
            return;
        }
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

        let seq = self.validation_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let client = self.client.clone();
        let documents = self.documents.clone();
        let services = self.services.clone();
        let project_published = self.project_published.clone();
        let validation_seq = self.validation_seq.clone();
        tokio::spawn(async move {
            tokio::time::sleep(VALIDATION_DEBOUNCE).await;
            if validation_seq.load(Ordering::SeqCst) != seq {
                return;
            }
            validate_documents(
                &client,
                &documents,
                &services,
                &project_published,
                &validation_seq,
            )
            .await;
        });
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.write().await.remove(&uri);
        self.validation_seq.fetch_add(1, Ordering::SeqCst);
        self.client
            .publish_diagnostics(uri.clone(), Vec::new(), None)
            .await;
        validate_documents(
            &self.client,
            &self.documents,
            &self.services,
            &self.project_published,
            &self.validation_seq,
        )
        .await;
    }

    async fn hover(
        &self,
        params: tower_lsp::lsp_types::HoverParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        if !is_dekascript_uri(&uri) {
            return Ok(None);
        }
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

        let documents = self.documents.read().await.clone();
        let hover_text = entry_hover(&documents, &text, &file_path, offset, &self.services);

        let Some(value) = hover_text else {
            return Ok(None);
        };

        let range = word_span_at_offset(text.as_bytes(), offset)
            .map(|span| span_to_range(span, &line_index));
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range,
        }))
    }

    /// Native import resolution and declared source locations.
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        if !is_dekascript_uri(&uri) {
            return Ok(None);
        }
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
        let Some(offset) = line_index.position_to_offset(position) else {
            return Ok(None);
        };

        let documents = self.documents.read().await.clone();
        let location = entry_definition(&documents, &text, &file_path, offset, &self.services);
        Ok(location.map(GotoDefinitionResponse::Scalar))
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        if !is_dekascript_uri(&uri) {
            return Ok(None);
        }
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

        let documents = self.documents.read().await.clone();
        let items = entry_completions(&documents, &self.services, &text, &file_path, offset);
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn references(
        &self,
        params: ReferenceParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        if !is_dekascript_uri(&uri) {
            return Ok(None);
        }
        let position = params.text_document_position.position;
        let mut text = self.get_document(&uri).await;
        if text.is_none()
            && let Ok(path) = uri.to_file_path()
        {
            text = fs::read_to_string(path).ok();
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
        if roots.is_empty()
            && let Ok(path) = uri.to_file_path()
            && let Some(parent) = path.parent()
        {
            roots.push(parent.to_path_buf());
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
        if !is_dekascript_uri(&uri) {
            return Ok(None);
        }
        let position = params.text_document_position.position;
        let new_name = params.new_name;
        let mut text = self.get_document(&uri).await;
        if text.is_none()
            && let Ok(path) = uri.to_file_path()
        {
            text = fs::read_to_string(path).ok();
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
        if roots.is_empty()
            && let Ok(path) = uri.to_file_path()
            && let Some(parent) = path.parent()
        {
            roots.push(parent.to_path_buf());
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

fn empty_diagnostic_report() -> DocumentDiagnosticReportResult {
    DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(
        RelatedFullDocumentDiagnosticReport {
            related_documents: None,
            full_document_diagnostic_report: FullDocumentDiagnosticReport {
                result_id: None,
                items: Vec::new(),
            },
        },
    ))
}

pub async fn run_stdio(services: NativeServices) -> anyhow::Result<()> {
    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: Arc::new(RwLock::new(HashMap::new())),
        workspace_roots: Arc::new(RwLock::new(Vec::new())),
        services: services.clone(),
        validation_seq: Arc::new(AtomicU64::new(0)),
        project_published: Arc::new(RwLock::new(HashSet::new())),
    });
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
    Ok(())
}
