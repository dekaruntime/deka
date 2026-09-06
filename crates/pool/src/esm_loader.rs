use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;

use deno_core::ModuleLoadOptions;
use deno_core::ModuleLoadReferrer;
use deno_core::ModuleLoadResponse;
use deno_core::ModuleLoader;
use deno_core::ModuleSource;
use deno_core::ModuleSourceCode;
use deno_core::ModuleSpecifier;
use deno_core::ModuleType;
use deno_core::ResolutionKind;
use deno_core::resolve_import;
use deno_error::JsErrorBox;

use runtime_core::{
    module_spec::{
        ds_source_candidates, is_bare_module_specifier, module_spec_aliases,
        resolve_ds_source_file,
    },
    DEKA_VALIDATION_ERROR_MARKER,
};
use runtime_core::modules::{read_linked_modules, MODULES_DIR};

fn parse_module_imports(source: &str) -> Vec<String> {
    runtime_core::ds_imports::paths(source)
}

#[derive(Clone)]
pub struct PhpxEsmLoader {
    project_root: PathBuf,
    cache_dir: PathBuf,
    entry_specifier: ModuleSpecifier,
    wrapper_specifier: ModuleSpecifier,
    sources: Rc<RefCell<HashMap<String, ModuleSourceCode>>>,
    /// Pre-compiled module graph for compiler v2. When present, `.ds` files
    /// are served from this map instead of compiled on demand.
    v2_modules: Option<HashMap<PathBuf, String>>,
}

impl PhpxEsmLoader {
    pub fn new(project_root: PathBuf, entry_path: PathBuf) -> Result<Self, JsErrorBox> {
        let cache_dir = runtime_core::framework::compiler_cache_dir(&project_root);
        std::fs::create_dir_all(&cache_dir).map_err(|err| {
            JsErrorBox::generic(format!("failed to create {}: {}", cache_dir.display(), err))
        })?;
        let entry_specifier = ModuleSpecifier::from_file_path(&entry_path)
            .map_err(|_| JsErrorBox::generic("invalid entry module path"))?;
        let wrapper_specifier = ModuleSpecifier::from_file_path(entry_wrapper_path(&project_root))
            .map_err(|_| JsErrorBox::generic("invalid entry wrapper path"))?;

        // JS/MJS/CJS entries are WinterTC workers: load as-is, do not send
        // them through dsc (dsc only compiles .ds/.dsx).
        let v2_modules = if entry_path.is_file() && !is_javascript_entry(&entry_path) {
            let modules = crate::dsc_compile::compile_graph(&project_root, &entry_path)
                .map_err(JsErrorBox::generic)?;
            let imports: Vec<String> = modules
                .keys()
                .filter_map(|path| std::fs::read_to_string(path).ok())
                .flat_map(|source| parse_module_imports(&source))
                .collect();
            ensure_project_layout(&project_root, &imports)
                .map_err(JsErrorBox::generic)?;
            enforce_dynamic_policy(&modules)?;
            Some(modules)
        } else {
            None
        };

        Ok(Self {
            project_root,
            cache_dir,
            entry_specifier,
            wrapper_specifier,
            sources: Rc::new(RefCell::new(HashMap::new())),
            v2_modules,
        })
    }

    fn cache_path_for(&self, path: &Path) -> PathBuf {
        let rel = path.strip_prefix(&self.project_root).unwrap_or(path);
        let mut out = self.cache_dir.join(rel);
        // Keep the input extension in the generated filename so distinct source
        // paths can never share a cache entry.
        let filename = out
            .file_name()
            .map(|name| format!("{}.js", name.to_string_lossy()))
            .unwrap_or_else(|| "module.js".to_string());
        out.set_file_name(filename);
        out
    }

    fn load_js_source(&self, path: &Path) -> Result<ModuleSourceCode, JsErrorBox> {
        let text = std::fs::read_to_string(path).map_err(|err| JsErrorBox::from_err(err))?;
        Ok(ModuleSourceCode::String(text.into()))
    }

    fn load_ds_source(&self, path: &Path) -> Result<ModuleSourceCode, JsErrorBox> {
        if let Some(v2_modules) = &self.v2_modules {
            if let Some(js) = crate::dsc_compile::lookup_js(v2_modules, path) {
                return Ok(ModuleSourceCode::String(js.clone().into()));
            }
        }
        // Linked packages sit outside the consumer project, so the entry
        // graph dump does not include them. Compile that tree through dsc.
        let js = crate::dsc_compile::compile_file(path).map_err(JsErrorBox::generic)?;
        Ok(ModuleSourceCode::String(js.into()))
    }

    fn resolve_phpx_module_spec(&self, specifier: &str) -> Option<PathBuf> {
        resolve_phpx_module_spec(&self.project_root, specifier)
    }

    /// Write compiler-provided `ui/*` modules into the cache so relative
    /// imports between them (`./jsx.js`) resolve as real files.
    fn materialize_ui_module(&self, specifier: &str) -> Option<PathBuf> {
        let source = deka_ui::source_for(specifier)?;
        let file_name = deka_ui::file_name_for(specifier)?;
        let dir = self.cache_dir.join("ui");
        let path = dir.join(file_name);
        if let Err(err) = std::fs::create_dir_all(&dir) {
            tracing::warn!("failed to create {}: {}", dir.display(), err);
            return None;
        }
        if let Err(err) = std::fs::write(&path, source) {
            tracing::warn!("failed to write {}: {}", path.display(), err);
            return None;
        }
        // Ensure siblings exist so `import from "./jsx.js"` works.
        for spec in deka_ui::SPECIFIERS {
            if let (Some(src), Some(name)) = (deka_ui::source_for(spec), deka_ui::file_name_for(spec)) {
                let sibling = dir.join(name);
                let _ = std::fs::write(sibling, src);
            }
        }
        Some(path)
    }

    fn resolve_path(&self, specifier: &str, referrer: &str) -> Result<ModuleSpecifier, JsErrorBox> {
        if is_bare_specifier(specifier) {
            if let Some(path) = self.materialize_ui_module(specifier) {
                return ModuleSpecifier::from_file_path(path)
                    .map_err(|_| JsErrorBox::generic("invalid ui module path"));
            }
            if let Some(path) = self.resolve_phpx_module_spec(specifier) {
                return ModuleSpecifier::from_file_path(path)
                    .map_err(|_| JsErrorBox::generic("invalid module path"));
            }
            return Err(JsErrorBox::generic(format!(
                "unable to resolve module '{}'; check php_modules",
                specifier
            )));
        }

        if specifier.to_ascii_lowercase().ends_with(".phpx") {
            return Err(JsErrorBox::generic(format!(
                "DekaScript uses .ds imports only; migrate '{}'",
                specifier
            )));
        }

        let resolved = resolve_import(specifier, referrer).map_err(JsErrorBox::from_err)?;
        if resolved.scheme() == "file" || resolved.scheme() == "ext" {
            return Ok(resolved);
        }
        Err(JsErrorBox::generic(format!(
            "unsupported module scheme: {}",
            resolved
        )))
    }

    fn load_source(&self, specifier: &ModuleSpecifier) -> Result<ModuleSource, JsErrorBox> {
        if specifier == &self.wrapper_specifier {
            let wrapper = self.wrapper_source();
            return Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(wrapper.into()),
                specifier,
                None,
            ));
        }
        let raw_path = specifier
            .to_file_path()
            .map_err(|_| JsErrorBox::generic("Only file:// URLs are supported"))?;
        // If the specifier has no extension, try DekaScript and JS candidates.
        let path = if raw_path.extension().is_none() {
            let ds = raw_path.with_extension("ds");
            let dsx = raw_path.with_extension("dsx");
            let js = raw_path.with_extension("js");
            let idx_ds = raw_path.join("index.ds");
            let idx_dsx = raw_path.join("index.dsx");
            let idx_js = raw_path.join("index.js");
            if ds.is_file() {
                ds
            } else if dsx.is_file() {
                dsx
            } else if js.is_file() {
                js
            } else if idx_ds.is_file() {
                idx_ds
            } else if idx_dsx.is_file() {
                idx_dsx
            } else if idx_js.is_file() {
                idx_js
            } else {
                raw_path
            }
        } else {
            raw_path
        };
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        let mut code = match ext {
            "ds" | "dsx" => self.load_ds_source(&path)?,
            _ => self.load_js_source(&path)?,
        };
        code = prepend_host_bindings(code);
        if specifier == &self.entry_specifier {
            code = append_entry_footer(code);
        }
        Ok(ModuleSource::new(
            ModuleType::JavaScript,
            code,
            specifier,
            None,
        ))
    }

    fn wrapper_source(&self) -> String {
        let entry = self.entry_specifier.to_string();
        let template = "import * as __jsx from \"ui/jsx\";\n\
import * as __server from \"ui/server\";\n\
import * as __reactive from \"ui/reactive\";\n\
import * as __suspense from \"ui/suspense\";\n\
import * as __router from \"ui/router\";\n\
globalThis.deka = globalThis.deka || {};\n\
globalThis.deka.ui = Object.freeze({\n\
  ...(globalThis.deka.ui || {}),\n\
  ...__jsx,\n\
  ...__server,\n\
  ...__reactive,\n\
  ...__suspense,\n\
  ...__router,\n\
});\n\
const __dekaMain = await import(\"__ENTRY__\");\n\
const __candidate = typeof __dekaMain.default !== \"undefined\"\n\
  ? __dekaMain.default\n\
  : typeof __dekaMain.app !== \"undefined\"\n\
  ? __dekaMain.app\n\
  : typeof __dekaMain.App !== \"undefined\"\n\
  ? __dekaMain.App\n\
  : typeof __dekaMain.handler !== \"undefined\"\n\
  ? __dekaMain.handler\n\
  : __dekaMain;\n\
if (typeof globalThis.app === \"undefined\" && typeof __candidate !== \"undefined\") {\n\
  if (typeof __candidate === \"function\" && typeof globalThis.__dekaNodeExpressAdapter === \"function\" && (typeof __candidate.handle === \"function\" || typeof __candidate.listen === \"function\")) {\n\
    globalThis.app = globalThis.__dekaNodeExpressAdapter(__candidate);\n\
  } else if (__candidate && typeof __candidate === \"object\" && typeof __candidate.fetch === \"function\") {\n\
    globalThis.app = __candidate;\n\
  } else if (__candidate && typeof __candidate === \"object\" && !__candidate.__dekaServer && typeof __candidate.routes === \"object\" && globalThis.__deka && typeof globalThis.__deka.serve === \"function\") {\n\
    globalThis.app = globalThis.__deka.serve(__candidate);\n\
  } else {\n\
    globalThis.app = __candidate;\n\
  }\n\
}\n";
        template
            .replace("__ENTRY__", &entry)
    }
}

impl ModuleLoader for PhpxEsmLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, JsErrorBox> {
        self.resolve_path(specifier, referrer)
    }

    fn load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<&ModuleLoadReferrer>,
        _options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let key = module_specifier.to_string();
        if let Some(code) = self.sources.borrow_mut().remove(&key) {
            return ModuleLoadResponse::Sync(Ok(ModuleSource::new(
                ModuleType::JavaScript,
                code,
                module_specifier,
                None,
            )));
        }

        ModuleLoadResponse::Sync(self.load_source(module_specifier))
    }

    fn prepare_load(
        &self,
        module_specifier: &ModuleSpecifier,
        _maybe_referrer: Option<String>,
        _maybe_content: Option<String>,
        _options: ModuleLoadOptions,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), JsErrorBox>>>> {
        let loader = self.clone();
        let spec = module_specifier.clone();
        Box::pin(async move {
            let source = loader.load_source(&spec)?;
            loader
                .sources
                .borrow_mut()
                .insert(spec.to_string(), source.code);
            Ok(())
        })
    }
}

pub fn is_javascript_entry(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "js" | "mjs" | "cjs"))
}

pub fn resolve_project_root(entry_path: &Path) -> Result<PathBuf, String> {
    let start = if entry_path.is_dir() {
        entry_path.to_path_buf()
    } else {
        entry_path.parent().unwrap_or(Path::new(".")).to_path_buf()
    };

    for dir in start.ancestors() {
        if dir.join("deka.json").is_file() {
            return Ok(dir.to_path_buf());
        }
    }

    // Workers-style JS handlers do not need a deka.json (bun/deno/CF).
    if is_javascript_entry(entry_path) {
        return Ok(start);
    }

    Err(format!(
        "deka runtime requires a deka.json project root (searched from {})",
        entry_path.display()
    ))
}

pub fn entry_wrapper_path(project_root: &Path) -> PathBuf {
    runtime_core::framework::compiler_cache_dir(project_root).join("__deka_entry.js")
}

pub fn hash_module_graph(entry_path: &Path) -> Result<u64, String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let project_root = resolve_project_root(entry_path)?;
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<PathBuf> = if entry_path.is_dir() {
        collect_app_route_files(entry_path)?
    } else {
        vec![entry_path.to_path_buf()]
    };
    let mut hasher = DefaultHasher::new();

    while let Some(path) = stack.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        source.hash(&mut hasher);

        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if ext == "ds" || ext == "dsx" {
            let imports = parse_module_imports(&source);
            for spec in imports {
                if let Some(resolved) = resolve_import_path(&project_root, &path, spec.trim()) {
                    stack.push(resolved);
                }
            }
        }
    }

    Ok(hasher.finish())
}

fn collect_app_route_files(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let app_dir = project_root.join("app");
    if !app_dir.is_dir() {
        return Err(format!(
            "app directory missing for app-mode entry: {}",
            app_dir.display()
        ));
    }
    collect_deka_source_files_recursive(&app_dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_deka_source_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read dir entry: {}", err))?;
        let path = entry.path();
        if path.is_dir() {
            collect_deka_source_files_recursive(&path, out)?;
            continue;
        }
        let is_deka_source = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("ds") || ext.eq_ignore_ascii_case("dsx"))
            .unwrap_or(false);
        if is_deka_source {
            out.push(path);
        }
    }
    Ok(())
}

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
fn enforce_dynamic_policy(modules: &HashMap<PathBuf, String>) -> Result<(), JsErrorBox> {
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

fn resolve_phpx_module_spec(project_root: &Path, specifier: &str) -> Option<PathBuf> {
    // @/ is a project-root alias: @/src/pages/foo -> {project_root}/src/pages/foo.ds
    //
    // Path-traversal guard: a malicious specifier like `@/../../etc/passwd`
    // would escape the tenant's project_root via `Path::join` (which does NOT
    // normalize `..` components). Reject any rel containing `..` segments
    // BEFORE join, then canonicalize the resolved path and assert it stays
    // inside the canonicalized project_root. Belt-and-suspenders: each layer
    // catches a different escape vector (literal `..`, symlinks, casing).
    if let Some(rel) = specifier.strip_prefix("@/") {
        // Reject literal traversal segments before doing any IO.
        let has_traversal = rel.split('/').any(|seg| seg == ".." || seg == ".");
        if !has_traversal {
            let base = project_root.join(rel);
            if let Some(resolved) = resolve_public_source_candidates(&base) {
                // Canonicalize both sides and confirm the resolved file is
                // inside project_root. If canonicalize fails (path doesn't
                // exist, etc.) we fall through to the next resolver — never
                // return a path that might escape.
                if let (Ok(resolved_canon), Ok(root_canon)) = (
                    std::fs::canonicalize(&resolved),
                    std::fs::canonicalize(project_root),
                ) {
                    if resolved_canon.starts_with(&root_canon) {
                        return Some(resolved);
                    }
                }
            }
        }
    }

    // Local development links (deka#470). A `deka link`ed package resolves
    // from its working tree and deliberately wins over anything installed
    // under ds_modules -- that override is the point of linking.
    //
    // Traversal is guarded the same way as the `@/` branch above: reject
    // literal `..` segments before any IO, then confirm the resolved file is
    // still inside the linked root after canonicalization. A link points
    // outside project_root by design, so the linked root is the boundary.
    if let Ok(linked) = read_linked_modules(project_root) {
        for (package, root) in &linked {
            for alias in module_spec_aliases(package) {
                let suffix = if specifier == alias {
                    ""
                } else if let Some(suffix) = specifier.strip_prefix(&format!("{alias}/")) {
                    suffix
                } else {
                    continue;
                };
                if suffix.split('/').any(|seg| seg == ".." || seg == ".") {
                    continue;
                }
                let base = if suffix.is_empty() {
                    root.clone()
                } else {
                    root.join(suffix)
                };
                if let Some(resolved) = resolve_internal_module_candidates(&base)
                    && let (Ok(resolved_canon), Ok(root_canon)) = (
                        std::fs::canonicalize(&resolved),
                        std::fs::canonicalize(root),
                    )
                    && resolved_canon.starts_with(&root_canon)
                {
                    return Some(resolved);
                }
            }
        }
    }

    let modules_dir = project_root.join(MODULES_DIR);
    let mut aliases = module_spec_aliases(specifier);
    // Map prefixed stdlib specifiers into the @deka scope: encoding/json -> @deka/encoding/json.
    if specifier.contains('/')
        && !specifier.starts_with('@')
        && !specifier.starts_with("./")
        && !specifier.starts_with("../")
    {
        aliases.push(format!("@deka/{}", specifier));
    }
    for alias in aliases.iter() {
        let base = if alias.starts_with("@user/") {
            modules_dir
                .join("@user")
                .join(alias.trim_start_matches("@user/"))
        } else {
            modules_dir.join(alias)
        };
        if let Some(resolved) = resolve_internal_module_candidates(&base) {
            return Some(resolved);
        }
    }

    // DEKA_MODULE_ROOT fallback (#220): if the tenant's php_modules/ doesn't
    // contain the spec, try the runtime stdlib root. This lets stdlib-only
    // tenants (e.g. id.tana.gg) deploy without vendoring stdlib.
    if let Some(root_os) = std::env::var_os("DEKA_MODULE_ROOT") {
        let root = std::path::Path::new(&root_os);
        for alias in aliases.iter() {
            let base = if alias.starts_with("@user/") {
                root.join("@user").join(alias.trim_start_matches("@user/"))
            } else {
                root.join(alias)
            };
            if let Some(resolved) = resolve_internal_module_candidates(&base) {
                return Some(resolved);
            }
        }
    }

    None
}

fn resolve_import_path(project_root: &Path, referrer: &Path, specifier: &str) -> Option<PathBuf> {
    if is_bare_specifier(specifier) {
        return resolve_phpx_module_spec(project_root, specifier);
    }

    if specifier.starts_with("http://") || specifier.starts_with("https://") {
        return None;
    }

    let base = if specifier.starts_with('/') {
        PathBuf::from(specifier)
    } else {
        referrer.parent().unwrap_or(Path::new(".")).join(specifier)
    };
    resolve_public_source_candidates(&base)
}

fn resolve_public_source_candidates(target: &Path) -> Option<PathBuf> {
    let mut candidates = ds_source_candidates(target);
    if target.extension().is_none() {
        candidates.push(target.with_extension("js"));
        candidates.push(target.join("index.js"));
    } else if matches!(
        target.extension().and_then(|ext| ext.to_str()),
        Some("ds" | "dsx" | "js")
    ) {
        candidates.push(target.to_path_buf());
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn resolve_internal_module_candidates(target: &Path) -> Option<PathBuf> {
    resolve_ds_source_file(target)
}

fn is_bare_specifier(spec: &str) -> bool {
    is_bare_module_specifier(spec)
}

const HOST_BINDINGS_PREAMBLE: &str = "const __dekaHostBindings = globalThis[Symbol.for('deka.host.internal')];
const __deka_host = __dekaHostBindings && __dekaHostBindings.host;
const __bridge = __dekaHostBindings && __dekaHostBindings.bridge;
const __bridge_async = __dekaHostBindings && __dekaHostBindings.bridgeAsync;
const __deka_wasm_call = __dekaHostBindings && __dekaHostBindings.wasmCall;
const __deka_wasm_call_async = __dekaHostBindings && __dekaHostBindings.wasmCallAsync;
const __deka_to_result = __dekaHostBindings && __dekaHostBindings.toResult;
";

fn prepend_host_bindings(code: ModuleSourceCode) -> ModuleSourceCode {
    match code {
        ModuleSourceCode::String(source) => {
            let mut text = String::with_capacity(HOST_BINDINGS_PREAMBLE.len() + source.len());
            text.push_str(HOST_BINDINGS_PREAMBLE);
            text.push_str(&source);
            ModuleSourceCode::String(text.into())
        }
        other => other,
    }
}

fn append_entry_footer(code: ModuleSourceCode) -> ModuleSourceCode {
    const FOOTER: &str = "\nif (typeof globalThis.app === \"undefined\" && typeof app !== \"undefined\") {\n\
  const __candidate = app;\n\
  if (typeof __candidate === \"function\" && typeof globalThis.__dekaNodeExpressAdapter === \"function\" && (typeof __candidate.handle === \"function\" || typeof __candidate.listen === \"function\")) {\n\
    globalThis.app = globalThis.__dekaNodeExpressAdapter(__candidate);\n\
  } else if (__candidate && typeof __candidate === \"object\" && typeof __candidate.fetch === \"function\") {\n\
    globalThis.app = __candidate;\n\
  } else if (__candidate && typeof __candidate === \"object\" && !__candidate.__dekaServer && typeof __candidate.routes === \"object\" && globalThis.__deka && typeof globalThis.__deka.serve === \"function\") {\n\
    globalThis.app = globalThis.__deka.serve(__candidate);\n\
  } else {\n\
    globalThis.app = __candidate;\n\
  }\n\
}\n";

    match code {
        ModuleSourceCode::String(source) => {
            let mut text = source.to_owned();
            text.push_str(FOOTER);
            ModuleSourceCode::String(text.into())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PhpxEsmLoader, is_javascript_entry, resolve_import_path, resolve_phpx_module_spec,
        resolve_project_root, resolve_public_source_candidates,
    };
    use std::fs;

    #[test]
    fn source_extensions_have_distinct_cache_paths() {
        let root = tempfile::tempdir().expect("temp project");
        let loader = PhpxEsmLoader::new(root.path().to_path_buf(), root.path().join("main.ds"))
            .expect("loader");

        let ds = loader.cache_path_for(&root.path().join("main.ds"));
        let js = loader.cache_path_for(&root.path().join("main.js"));
        assert_ne!(ds, js);
        assert!(ds.ends_with("main.ds.js"));
        assert!(js.ends_with("main.js.js"));
    }

    #[test]
    fn extensionless_import_resolves_dekascript() {
        let root = tempfile::tempdir().expect("temp project");
        let target = root.path().join("shared");
        fs::write(target.with_extension("ds"), "export const value = 1;").expect("ds");

        assert_eq!(
            resolve_public_source_candidates(&target),
            Some(target.with_extension("ds"))
        );
    }

    #[test]
    fn extensionless_public_import_does_not_fall_back_to_phpx() {
        let root = tempfile::tempdir().expect("temp project");
        let target = root.path().join("legacy");
        fs::write(target.with_extension("phpx"), "export const value = 1;").expect("phpx");
        fs::create_dir_all(&target).expect("legacy directory");
        fs::write(target.join("index.phpx"), "export const value = 2;").expect("index phpx");

        let referrer = root.path().join("main.ds");
        assert_eq!(
            resolve_import_path(root.path(), &referrer, "./legacy"),
            None
        );
        assert_eq!(resolve_phpx_module_spec(root.path(), "@/legacy"), None);
        assert_eq!(
            resolve_import_path(root.path(), &referrer, "./legacy.phpx"),
            None
        );
    }

    #[test]
    fn internal_module_spec_does_not_resolve_phpx() {
        let root = tempfile::tempdir().expect("temp project");
        let modules = root.path().join("ds_modules").join("legacy");
        fs::create_dir_all(&modules).expect("ds_modules");
        fs::write(modules.join("index.phpx"), "export const value = 1;").expect("index phpx");
        assert_eq!(resolve_phpx_module_spec(root.path(), "legacy"), None);
    }

    #[test]
    fn wrapper_accepts_exported_dekascript_app() {
        let root = tempfile::tempdir().expect("temp project");
        let loader = PhpxEsmLoader::new(root.path().to_path_buf(), root.path().join("main.ds"))
            .expect("loader");
        assert!(loader.wrapper_source().contains("__dekaMain.App"));
        assert!(loader.wrapper_source().contains("ui/router"));
    }

    #[test]
    fn javascript_entry_skips_dsc_and_does_not_need_deka_json() {
        let root = tempfile::tempdir().expect("temp project");
        let entry = root.path().join("handler.js");
        fs::write(
            &entry,
            "export default { async fetch(request) { return new Response(\"ok\"); } }\n",
        )
        .expect("write js handler");

        assert!(is_javascript_entry(&entry));
        let resolved = resolve_project_root(&entry).expect("js handler has a project root");
        assert_eq!(resolved, root.path());

        PhpxEsmLoader::new(root.path().to_path_buf(), entry).expect("js loader skips dsc");
    }

    #[test]
    fn ds_entry_still_requires_deka_json() {
        let root = tempfile::tempdir().expect("temp project");
        let entry = root.path().join("main.ds");
        fs::write(&entry, "export const app = 1;\n").expect("write ds");
        let err = resolve_project_root(&entry).expect_err("ds needs deka.json");
        assert!(err.contains("deka.json"), "{err}");
    }
}
