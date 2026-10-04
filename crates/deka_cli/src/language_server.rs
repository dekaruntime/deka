//! Editor callbacks share native source/profile selection and the production
//! host registry with check/build. Validation never executes bytecode.
use crate::{cli_runtime, compile_with_sources, hosts, source};
use deka_lsp::{CompletionItemKind, Export, ModuleInfo, NativeServices};
use deka_vm::{Result, compiler};
use std::sync::Arc;
fn export(export: compiler::EditorExport) -> Export {
    Export {
        name: export.name,
        detail: export.detail,
        kind: match export.kind {
            compiler::EditorExportKind::Function => CompletionItemKind::FUNCTION,
            compiler::EditorExportKind::Constant => CompletionItemKind::CONSTANT,
            compiler::EditorExportKind::Struct => CompletionItemKind::STRUCT,
            compiler::EditorExportKind::Enum => CompletionItemKind::ENUM,
            compiler::EditorExportKind::Interface => CompletionItemKind::INTERFACE,
            compiler::EditorExportKind::Type => CompletionItemKind::TYPE_PARAMETER,
        },
    }
}
pub(super) fn serve() -> Result<()> {
    let registry = hosts()?;
    let services = NativeServices {
        validate: Arc::new(|path, documents| {
            compile_with_sources(&source(path, None)?, documents).map(|_| ())
        }),
        module: Arc::new(|path, name, documents| {
            let module = compiler::editor_module(path, name, &hosts()?, documents)?;
            Ok(ModuleInfo {
                path: module.path,
                source: module.source,
                exports: module.exports.into_iter().map(export).collect(),
            })
        }),
        globals: compiler::editor_globals(&registry)?
            .into_iter()
            .map(export)
            .collect(),
        modules: compiler::editor_modules(&registry)?,
    };
    cli_runtime()?
        .block_on(deka_lsp::run_stdio(services))
        .map_err(|error| error.to_string())
}
