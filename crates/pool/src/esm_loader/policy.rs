//! Pre-execution policy gates: the `security.allow.dynamic` validator and the
//! project-layout gate. Both run after compilation, before any module graph
//! reaches V8.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use deno_error::JsErrorBox;

use runtime_core::DEKA_VALIDATION_ERROR_MARKER;

/// Enforce the resolved `security.allow.dynamic` policy on every module in the
/// graph, before any of it reaches V8.
///
/// deka#425. The inline-handler check in `worker_execution` covers the platform
/// path, whose tenant bundles arrive as source. `deka run` and `deka serve` are
/// ESM and leave `handler_code` empty, so nothing gated them and the runtime
/// printed `dynamic=false` while `eval` worked.
///
/// This validates the compiler's own output for each user module rather than
/// the assembled script. The host-bindings preamble legitimately reaches
/// `globalThis` and would trip the validator; user modules never need to.
pub(crate) fn enforce_dynamic_policy(modules: &HashMap<PathBuf, String>) -> Result<(), JsErrorBox> {
    // Deterministic order, so a project with two offending modules reports the
    // same one every run.
    let mut paths: Vec<&PathBuf> = modules.keys().collect();
    paths.sort();
    for path in paths {
        let source = &modules[path];
        let name = path.to_string_lossy();
        crate::validation::validate_dynamic_code_from_process_env(source, &name)
            .map_err(|err| JsErrorBox::generic(format!("{DEKA_VALIDATION_ERROR_MARKER}{err}")))?;
    }
    Ok(())
}

pub fn ensure_project_layout(project_root: &Path, imports: &[String]) -> Result<(), String> {
    // DEKA_MODULE_ROOT is the stdlib-only-tenant escape (#220): when it points
    // at a root *other* than this project, the runtime supplies the stdlib and
    // a local ds_modules/ tree is not expected.
    //
    // It used to bypass on presence alone, which made this whole function
    // dead: the CLI sets the variable to the project root itself on every
    // ordinary run, so the early return always fired (deka#229, deka#430).
    // Comparing against the project root preserves what #220 actually needed
    // and drops the accidental blanket bypass.
    let module_root = std::env::var_os("DEKA_MODULE_ROOT").map(PathBuf::from);

    runtime_core::project_gate::validate_project(
        project_root,
        imports,
        &runtime_core::project_gate::GateOptions {
            module_root,
            require_lockfile: true,
            context: "deka runtime",
        },
    )
}
