//! Native language server migrated from dsc; validation is supplied by the CLI.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, CompletionItem, CompletionOptions,
    CompletionParams, CompletionResponse, Diagnostic, DiagnosticOptions,
    DiagnosticServerCapabilities, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentDiagnosticParams,
    DocumentDiagnosticReport, DocumentDiagnosticReportResult, Documentation,
    FullDocumentDiagnosticReport, GotoDefinitionParams, GotoDefinitionResponse, Hover,
    HoverContents, InitializeParams, InitializeResult, InitializedParams, InsertTextFormat,
    Location, MarkupContent, MarkupKind, MessageType, OneOf, Position, Range, ReferenceParams,
    RelatedFullDocumentDiagnosticReport, RenameParams, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url, WorkspaceEdit,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod completion;
mod definition;
mod documents;
mod handlers;
mod hover;
mod project;
mod symbols;
pub use handlers::run_stdio;

pub(crate) use completion::*;
pub(crate) use definition::*;
pub(crate) use documents::*;
pub(crate) use hover::*;
pub(crate) use project::*;
pub(crate) use symbols::*;

pub(crate) const LANGUAGE_ID: &str = "dekascript";

pub(crate) fn is_dekascript_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("ds") || extension.eq_ignore_ascii_case("dsx")
        })
}

pub(crate) fn is_dekascript_uri(uri: &Url) -> bool {
    uri.to_file_path()
        .is_ok_and(|path| is_dekascript_path(&path))
}

/// The CLI supplies its actual host registry and source/profile selection.
/// This callback validates only: it never runs a program or changes files.
pub type Validator = Arc<dyn Fn(&Path, &SourceTexts) -> Result<(), String> + Send + Sync>;
pub type SourceTexts = BTreeMap<PathBuf, String>;
pub type ModuleQuery =
    Arc<dyn Fn(&Path, &str, &SourceTexts) -> Result<ModuleInfo, String> + Send + Sync>;

/// Resolved native module metadata, supplied by the same compiler resolver.
#[derive(Clone, Debug, Default)]
pub struct ModuleInfo {
    pub path: Option<PathBuf>,
    pub source: Option<String>,
    pub exports: Vec<Export>,
}
#[derive(Clone, Debug)]
pub struct Export {
    pub name: String,
    pub detail: String,
    pub kind: CompletionItemKind,
}
#[derive(Clone)]
pub struct NativeServices {
    pub validate: Validator,
    pub module: ModuleQuery,
    pub globals: Vec<Export>,
    pub modules: Vec<String>,
}

pub use tower_lsp::lsp_types::CompletionItemKind;

#[cfg(test)]
mod tests;
