//! Project-mode WASM compiler ABI.
//!
//! A project is a virtual file system of DekaScript modules. The browser can
//! write source files into the project, compile the whole graph, and read back
//! per-module JavaScript output. Imports and exports are lowered to a factory-
//! function convention (`__dekaRequire` / `exports.*`) so the browser loader can
//! link modules inside the existing sandbox without native ES module evaluation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bumpalo::Bump;
use modules_php::compiler_api::compile_deka_project_module;
use serde::Serialize;

use crate::{Diagnostic, WasmResult, box_result, json};

/// A project holds a virtual file system and the results of the last compile.
pub struct ProjectState {
    files: HashMap<String, String>,
    compiled: HashMap<String, CompiledModule>,
    diagnostics: Vec<Diagnostic>,
    ok: bool,
}

struct CompiledModule {
    code: String,
}

#[derive(Serialize, Debug)]
struct ProjectCompileResponse {
    abi_version: u32,
    ok: bool,
    modules: HashMap<String, ModuleOutput>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Serialize, Debug, Clone)]
struct ModuleOutput {
    code: String,
}

#[derive(Serialize, Debug)]
struct ProjectReadResponse {
    abi_version: u32,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<ModuleOutput>,
    diagnostics: Vec<Diagnostic>,
}

impl ProjectState {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            compiled: HashMap::new(),
            diagnostics: Vec::new(),
            ok: false,
        }
    }

    pub fn write(&mut self, path: &str, source: &str) {
        self.files.insert(normalize_path(path), source.to_string());
    }

    pub fn compile(&mut self) -> String {
        self.compiled.clear();
        self.diagnostics.clear();
        self.ok = true;

        let paths: Vec<String> = self.files.keys().cloned().collect();
        for path in &paths {
            match self.compile_module(path) {
                Ok(code) => {
                    self.compiled.insert(
                        path.clone(),
                        CompiledModule { code },
                    );
                }
                Err(diagnostic) => {
                    self.diagnostics.push(diagnostic);
                    self.ok = false;
                }
            }
        }

        let modules: HashMap<String, ModuleOutput> = self
            .compiled
            .iter()
            .map(|(path, module)| {
                (
                    path.clone(),
                    ModuleOutput {
                        code: module.code.clone(),
                    },
                )
            })
            .collect();

        json(&ProjectCompileResponse {
            abi_version: crate::ABI_VERSION,
            ok: self.ok,
            modules,
            diagnostics: self.diagnostics.clone(),
        })
    }

    pub fn read(&self, path: &str) -> String {
        let normalized = normalize_path(path);
        if let Some(module) = self.compiled.get(&normalized) {
            return json(&ProjectReadResponse {
                abi_version: crate::ABI_VERSION,
                ok: true,
                output: Some(ModuleOutput {
                    code: module.code.clone(),
                }),
                diagnostics: Vec::new(),
            });
        }
        json(&ProjectReadResponse {
            abi_version: crate::ABI_VERSION,
            ok: false,
            output: None,
            diagnostics: vec![crate::internal_diagnostic(
                path,
                "",
                format!("Module '{}' has not been compiled or does not exist in the project.", path),
            )],
        })
    }

    fn compile_module(&self, path: &str) -> Result<String, Diagnostic> {
        let source = self
            .files
            .get(path)
            .ok_or_else(|| crate::internal_diagnostic(path, "", format!("File '{}' not found in project.", path)))?;

        // Validate that every import resolves to another virtual file in the project.
        let meta = deka_js::parse_source_module_meta(source);
        for decl in &meta.imports {
            if let Err(message) = self.resolve_import(path, &decl.from) {
                return Err(crate::internal_diagnostic(
                    path,
                    source,
                    format!(
                        "Cannot resolve import '{}' from '{}': {}",
                        decl.from, path, message
                    ),
                ));
            }
        }

        let arena = Bump::new();
        let result = compile_deka_project_module(source, path, &arena);

        if !result.errors.is_empty() {
            return Err(crate::internal_diagnostic(
                path,
                source,
                result
                    .errors
                    .iter()
                    .map(|e| e.message.clone())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
        }

        let program = result.ast.ok_or_else(|| {
            crate::internal_diagnostic(path, source, "no AST available after validation".to_string())
        })?;

        let mut meta = deka_js::parse_source_module_meta(source);
        meta.is_ds = true;
        meta.project_mode = true;

        match deka_js::emit_js_from_ast_with_warnings(&program, source.as_bytes(), meta) {
            Ok((code, _warnings)) => Ok(code),
            Err(message) => Err(crate::internal_diagnostic(path, source, message)),
        }
    }

    fn resolve_import(&self, current_path: &str, specifier: &str) -> Result<(), String> {
        if specifier.starts_with("./") || specifier.starts_with("../") {
            let resolved = resolve_relative_path(current_path, specifier)?;
            let candidates = relative_candidates(&resolved);
            for candidate in &candidates {
                if self.files.contains_key(candidate) {
                    return Ok(());
                }
            }
            return Err(format!(
                "no module '{}' resolved to any of: {}",
                specifier,
                candidates.join(", ")
            ));
        }
        Err(format!(
            "non-relative import '{}' is not supported in project-mode spike; use './foo.ds'",
            specifier
        ))
    }
}

fn normalize_path(path: &str) -> String {
    let normalized = Path::new(path)
        .to_string_lossy()
        .replace('\\', "/");
    normalized.strip_prefix("./").unwrap_or(&normalized).to_string()
}

fn resolve_relative_path(current_path: &str, specifier: &str) -> Result<PathBuf, String> {
    let parent = Path::new(current_path)
        .parent()
        .ok_or_else(|| format!("'{}' has no parent directory", current_path))?;
    let joined = parent.join(specifier);
    let normalized = normalize_path(&joined.to_string_lossy());
    Ok(PathBuf::from(normalized))
}

fn relative_candidates(resolved: &Path) -> Vec<String> {
    let base = resolved.to_string_lossy().replace('\\', "/");
    let mut candidates = Vec::new();
    if base.ends_with(".ds") || base.ends_with(".phpx") {
        candidates.push(base.clone());
    } else {
        candidates.push(format!("{}.ds", base));
        candidates.push(format!("{}/index.ds", base));
        candidates.push(format!("{}.phpx", base));
        candidates.push(format!("{}/index.phpx", base));
    }
    candidates
}

/// Allocate a new project and return its opaque handle.
#[unsafe(no_mangle)]
pub extern "C" fn deka_compiler_project_new() -> u32 {
    let project = Box::new(ProjectState::new());
    Box::into_raw(project) as u32
}

/// Free a project previously allocated with `deka_compiler_project_new`.
///
/// # Safety
/// `project_id` must be a handle returned by `deka_compiler_project_new` that
/// has not already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deka_compiler_project_free(project_id: u32) {
    if project_id == 0 {
        return;
    }
    unsafe {
        let _ = Box::from_raw(project_id as *mut ProjectState);
    }
}

/// Write a source file into the project, replacing any existing file at `path`.
///
/// # Safety
/// Pointer/length pairs must point to valid, immutable UTF-8 buffers in WASM
/// memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deka_compiler_project_write(
    project_id: u32,
    path_ptr: *const u8,
    path_len: u32,
    source_ptr: *const u8,
    source_len: u32,
) {
    let project = match project_from_id(project_id) {
        Some(p) => p,
        None => return,
    };
    let path = crate::read_utf8(path_ptr, path_len, "path");
    let source = crate::read_utf8(source_ptr, source_len, "source");
    if let (Ok(path), Ok(source)) = (path, source) {
        project.write(path, source);
    }
}

/// Compile every module in the project and return a JSON-encoded result.
///
/// # Safety
/// `project_id` must be a valid project handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deka_compiler_project_compile(project_id: u32) -> *mut WasmResult {
    let project = match project_from_id(project_id) {
        Some(p) => p,
        None => {
            return box_result(&json(&ProjectCompileResponse {
                abi_version: crate::ABI_VERSION,
                ok: false,
                modules: HashMap::new(),
                diagnostics: vec![crate::internal_diagnostic(
                    "<project>",
                    "",
                    "invalid project handle".to_string(),
                )],
            }))
        }
    };
    let json = project.compile();
    box_result(&json)
}

/// Read the emitted JavaScript for one module after a successful compile.
///
/// # Safety
/// `project_id` must be a valid project handle. Pointer/length pairs must point
/// to valid, immutable UTF-8 buffers in WASM memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deka_compiler_project_read(
    project_id: u32,
    path_ptr: *const u8,
    path_len: u32,
) -> *mut WasmResult {
    let project = match project_from_id(project_id) {
        Some(p) => p,
        None => {
            return box_result(&json(&ProjectReadResponse {
                abi_version: crate::ABI_VERSION,
                ok: false,
                output: None,
                diagnostics: vec![crate::internal_diagnostic(
                    "<project>",
                    "",
                    "invalid project handle".to_string(),
                )],
            }))
        }
    };
    let path = crate::read_utf8(path_ptr, path_len, "path");
    let json = match path {
        Ok(path) => project.read(path),
        Err(message) => json(&ProjectReadResponse {
            abi_version: crate::ABI_VERSION,
            ok: false,
            output: None,
            diagnostics: vec![crate::internal_diagnostic("<project>", "", message.to_string())],
        }),
    };
    box_result(&json)
}

fn project_from_id(project_id: u32) -> Option<&'static mut ProjectState> {
    if project_id == 0 {
        return None;
    }
    unsafe { (project_id as *mut ProjectState).as_mut() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn project_compiles_relative_import_and_emits_factory_functions() {
        let mut project = ProjectState::new();
        project.write(
            "math.ds",
            "export fn add(a: number, b: number): number {\n  return a + b;\n}\n",
        );
        project.write(
            "main.ds",
            "import { add } from \"./math.ds\";\nconsole.log(add(1, 2));\n",
        );

        let json = project.compile();
        let response: Value = serde_json::from_str(&json).expect("valid compile response JSON");

        assert_eq!(response["ok"], true, "compile failed: {}", json);
        assert!(response["modules"]["main.ds"]["code"].is_string(), "missing main.ds");
        assert!(response["modules"]["math.ds"]["code"].is_string(), "missing math.ds");

        let main = response["modules"]["main.ds"]["code"].as_str().unwrap();
        assert!(
            main.contains("const { add } = __dekaRequire(\"./math.ds\")"),
            "expected factory import, got:\n{}",
            main
        );

        let math = response["modules"]["math.ds"]["code"].as_str().unwrap();
        assert!(
            math.contains("exports.add = add"),
            "expected factory export, got:\n{}",
            math
        );
    }

    #[test]
    fn project_reports_unresolved_relative_import() {
        let mut project = ProjectState::new();
        project.write(
            "main.ds",
            "import { missing } from \"./nowhere.ds\";\nconsole.log(missing());\n",
        );

        let json = project.compile();
        let response: Value = serde_json::from_str(&json).expect("valid compile response JSON");

        assert_eq!(response["ok"], false, "expected compile failure");
        let messages: Vec<&str> = response["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["message"].as_str())
            .collect();
        assert!(
            messages.iter().any(|m| m.contains("Cannot resolve import")),
            "expected unresolved import diagnostic, got: {:?}",
            messages
        );
    }
}
