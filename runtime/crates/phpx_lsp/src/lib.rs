#![allow(clippy::all)]

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

mod completion;
mod diagnostics;
mod documents;
mod handlers;
mod symbols;

pub use handlers::run_stdio;

pub(crate) use completion::*;
pub(crate) use diagnostics::*;
pub(crate) use documents::*;
#[cfg(test)]
pub(crate) use handlers::should_skip_template_html_diagnostic;
pub(crate) use handlers::TargetMode;
pub(crate) use symbols::*;

#[cfg(test)]
mod tests;
