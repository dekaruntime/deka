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

use phpx_js::SourceModuleMeta;
use phpx_js::build_stdlib_prelude;
use phpx_js::compile_phpx_source_to_js;
use phpx_js::parse_source_module_meta;
use runtime_core::module_spec::{is_bare_module_specifier, module_spec_aliases};

#[derive(Clone)]
pub struct PhpxEsmLoader {
    project_root: PathBuf,
    cache_dir: PathBuf,
    entry_specifier: ModuleSpecifier,
    entry_is_app_directory: bool,
    app_directory_path: Option<PathBuf>,
    wrapper_specifier: ModuleSpecifier,
    prelude_specifier: ModuleSpecifier,
    prelude_source: String,
    sources: Rc<RefCell<HashMap<String, ModuleSourceCode>>>,
}

impl PhpxEsmLoader {
    pub fn new(project_root: PathBuf, entry_path: PathBuf) -> Result<Self, JsErrorBox> {
        let cache_dir = project_root.join(".cache").join("phpx_js");
        std::fs::create_dir_all(&cache_dir).map_err(|err| {
            JsErrorBox::generic(format!("failed to create {}: {}", cache_dir.display(), err))
        })?;
        let entry_is_app_directory = entry_path.is_dir();
        let app_directory_path = if entry_is_app_directory {
            Some(entry_path.clone())
        } else {
            None
        };
        let entry_module_path = if entry_is_app_directory {
            app_directory_entry_path(&project_root)
        } else {
            entry_path
        };
        let entry_specifier = ModuleSpecifier::from_file_path(&entry_module_path)
            .map_err(|_| JsErrorBox::generic("invalid entry module path"))?;
        let wrapper_specifier = ModuleSpecifier::from_file_path(entry_wrapper_path(&project_root))
            .map_err(|_| JsErrorBox::generic("invalid entry wrapper path"))?;
        let prelude_specifier = ModuleSpecifier::from_file_path(entry_prelude_path(&project_root))
            .map_err(|_| JsErrorBox::generic("invalid prelude path"))?;
        let prelude_source = build_stdlib_prelude(&project_root).unwrap_or_else(|err| {
            format!(
                "if (!globalThis.panic) {{ globalThis.panic = (msg) => {{ throw new Error(String(msg)); }}; }}\n\
// stdlib prelude failed: {}\n",
                err.replace('\n', " ")
            )
        });
        Ok(Self {
            project_root,
            cache_dir,
            entry_specifier,
            entry_is_app_directory,
            app_directory_path,
            wrapper_specifier,
            prelude_specifier,
            prelude_source,
            sources: Rc::new(RefCell::new(HashMap::new())),
        })
    }

    fn cache_path_for(&self, path: &Path) -> PathBuf {
        let rel = path.strip_prefix(&self.project_root).unwrap_or(path);
        let mut out = self.cache_dir.join(rel);
        out.set_extension("js");
        out
    }

    fn load_js_source(&self, path: &Path) -> Result<ModuleSourceCode, JsErrorBox> {
        let text = std::fs::read_to_string(path).map_err(|err| JsErrorBox::from_err(err))?;
        Ok(ModuleSourceCode::String(text.into()))
    }

    fn load_phpx_source(&self, path: &Path) -> Result<ModuleSourceCode, JsErrorBox> {
        let input = path
            .to_str()
            .ok_or_else(|| JsErrorBox::generic(format!("invalid path: {}", path.display())))?;
        let source = std::fs::read_to_string(path).map_err(|err| {
            JsErrorBox::generic(format!("Failed to read {}: {}", path.display(), err))
        })?;
        let meta = parse_source_module_meta(&source);
        ensure_project_layout(&self.project_root, &meta).map_err(|err| JsErrorBox::generic(err))?;
        let js = compile_phpx_source_to_js(&source, input, meta)
            .map_err(|err| JsErrorBox::generic(err))?;

        let cache_path = self.cache_path_for(path);
        if let Some(parent) = cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&cache_path, &js);

        Ok(ModuleSourceCode::String(js.into()))
    }

    fn resolve_phpx_module_spec(&self, specifier: &str) -> Option<PathBuf> {
        resolve_phpx_module_spec(&self.project_root, specifier)
    }

    fn resolve_path(&self, specifier: &str, referrer: &str) -> Result<ModuleSpecifier, JsErrorBox> {
        if is_bare_specifier(specifier) {
            if let Some(path) = self.resolve_phpx_module_spec(specifier) {
                return ModuleSpecifier::from_file_path(path)
                    .map_err(|_| JsErrorBox::generic("invalid module path"));
            }
            return Err(JsErrorBox::generic(format!(
                "unable to resolve module '{}'; check php_modules",
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
        if specifier == &self.prelude_specifier {
            return Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(self.prelude_source.clone().into()),
                specifier,
                None,
            ));
        }
        if specifier == &self.wrapper_specifier {
            let wrapper = self.wrapper_source();
            return Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(wrapper.into()),
                specifier,
                None,
            ));
        }
        if self.entry_is_app_directory && specifier == &self.entry_specifier {
            let app_root = self
                .app_directory_path
                .as_ref()
                .ok_or_else(|| JsErrorBox::generic("missing app directory path"))?;
            let app_root_json = serde_json::to_string(&app_root.to_string_lossy().to_string())
                .map_err(|err| {
                    JsErrorBox::generic(format!("failed to encode app root: {}", err))
                })?;
            return Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(app_directory_entry_source(&app_root_json).into()),
                specifier,
                None,
            ));
        }
        let raw_path = specifier
            .to_file_path()
            .map_err(|_| JsErrorBox::generic("Only file:// URLs are supported"))?;
        // If the specifier has no extension, try .phpx, .js, index.phpx, index.js candidates.
        let path = if raw_path.extension().is_none() {
            let phpx = raw_path.with_extension("phpx");
            let js = raw_path.with_extension("js");
            let idx_phpx = raw_path.join("index.phpx");
            let idx_js = raw_path.join("index.js");
            if phpx.is_file() {
                phpx
            } else if js.is_file() {
                js
            } else if idx_phpx.is_file() {
                idx_phpx
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
            "phpx" => self.load_phpx_source(&path)?,
            _ => self.load_js_source(&path)?,
        };
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
        let template = "import \"__PRELUDE__\";\n\
import * as __dekaMain from \"__ENTRY__\";\n\
const __candidate = typeof __dekaMain.default !== \"undefined\"\n\
  ? __dekaMain.default\n\
  : typeof __dekaMain.app !== \"undefined\"\n\
  ? __dekaMain.app\n\
  : typeof __dekaMain.handler !== \"undefined\"\n\
  ? __dekaMain.handler\n\
  : __dekaMain;\n\
if (__dekaMain && __dekaMain.phpxBuildMode === \"scaffold\") {\n\
  throw new Error(\"[phpx-js] subset transpile failed for \" + String(__dekaMain.phpxFile || \"unknown\") +\n\
    \" (reason: \" + String(__dekaMain.phpxBuildReason || \"unknown\") + \"). Fallback execution is disabled.\");\n\
}\n\
if (typeof globalThis.app === \"undefined\" && typeof __candidate !== \"undefined\") {\n\
  if (typeof __candidate === \"function\" && typeof globalThis.__dekaNodeExpressAdapter === \"function\" && (typeof __candidate.handle === \"function\" || typeof __candidate.listen === \"function\")) {\n\
    globalThis.app = globalThis.__dekaNodeExpressAdapter(__candidate);\n\
  } else if (__candidate && typeof __candidate === \"object\" && !__candidate.__dekaServer && (typeof __candidate.fetch === \"function\" || typeof __candidate.routes === \"object\")) {\n\
    globalThis.app = globalThis.__deka.serve(__candidate);\n\
  } else {\n\
    globalThis.app = __candidate;\n\
  }\n\
}\n";
        template
            .replace("__PRELUDE__", &self.prelude_specifier.to_string())
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

    Err(format!(
        "deka runtime requires a deka.json project root (searched from {})",
        entry_path.display()
    ))
}

pub fn entry_wrapper_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".cache")
        .join("phpx_js")
        .join("__deka_entry.js")
}

pub fn app_directory_entry_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".cache")
        .join("phpx_js")
        .join("__deka_app_entry.js")
}

pub fn entry_prelude_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".cache")
        .join("phpx_js")
        .join("__deka_prelude.js")
}

fn app_directory_entry_source(app_root_json: &str) -> String {
    format!(
        "import {{ servePhp }} from \"ext:deka_php/php.js\";\n\
const app = servePhp({});\n\
export default app;\n",
        app_root_json
    )
    .to_string()
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
        if ext == "phpx" {
            let meta = parse_source_module_meta(&source);
            for decl in meta.imports {
                if let Some(resolved) = resolve_import_path(&project_root, &path, decl.from.trim())
                {
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
    collect_phpx_files_recursive(&app_dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_phpx_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read dir entry: {}", err))?;
        let path = entry.path();
        if path.is_dir() {
            collect_phpx_files_recursive(&path, out)?;
            continue;
        }
        let is_phpx = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("phpx"))
            .unwrap_or(false);
        if is_phpx {
            out.push(path);
        }
    }
    Ok(())
}

pub fn ensure_project_layout(project_root: &Path, meta: &SourceModuleMeta) -> Result<(), String> {
    // PHPX_MODULE_ROOT bypass (#220): when set, the tenant relies on the runtime stdlib at
    // that root and we trust the runtime-provided modules without requiring a local
    // deka.lock or php_modules/. Tenant-local packages would still need a lockfile, but
    // stdlib-only tenants (id.tana.gg) deploy without ceremony.
    if std::env::var_os("PHPX_MODULE_ROOT").is_some() {
        return Ok(());
    }

    let lock_path = project_root.join("deka.lock");
    if !lock_path.is_file() {
        return Err(format!(
            "deka runtime requires deka.lock at project root: {}",
            lock_path.display()
        ));
    }

    let stdlib_imports = collect_stdlib_imports(meta);
    if stdlib_imports.is_empty() {
        return Ok(());
    }

    let modules_dir = project_root.join("php_modules");
    if !modules_dir.is_dir() {
        return Err(format!(
            "deka runtime requires php_modules/ at project root when using stdlib imports ({}). Run `deka install`.",
            stdlib_imports.join(", ")
        ));
    }

    let mut missing = Vec::new();
    for spec in stdlib_imports {
        if resolve_module_file(&modules_dir, &spec).is_none() {
            missing.push(spec);
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "missing stdlib modules under {}: {}. Run `deka install`.",
            modules_dir.display(),
            missing.join(", ")
        ))
    }
}

fn collect_stdlib_imports(meta: &SourceModuleMeta) -> Vec<String> {
    let mut seen = HashSet::new();
    for decl in &meta.imports {
        let spec = decl.from.trim();
        if is_stdlib_module_spec(spec) {
            seen.insert(spec.to_string());
        }
    }
    seen.into_iter().collect()
}

fn is_stdlib_module_spec(spec: &str) -> bool {
    if !is_bare_specifier(spec) || spec.starts_with("@user/") {
        return false;
    }

    spec.starts_with("component/")
        || spec.starts_with("deka/")
        || spec.starts_with("encoding/")
        || spec.starts_with("db/")
        || spec.starts_with("@deka/")
        || matches!(
            spec,
            "json"
                | "postgres"
                | "mysql"
                | "sqlite"
                | "bytes"
                | "buffer"
                | "tcp"
                | "tls"
                | "fs"
                | "crypto"
                | "jwt"
                | "cookies"
                | "auth"
                | "db"
        )
}

fn resolve_module_file(modules_dir: &Path, spec: &str) -> Option<PathBuf> {
    // Expand aliases, and for prefixed stdlib imports (e.g. encoding/json)
    // also try the scoped layout (@deka/encoding/json) since modules installed
    // from linkhash land under php_modules/@deka/<pkg>/<subpath>.
    let mut aliases = module_spec_aliases(spec);
    if spec.contains('/')
        && !spec.starts_with('@')
        && !spec.starts_with("./")
        && !spec.starts_with("../")
    {
        aliases.push(format!("@deka/{}", spec));
    }

    let mut candidates = Vec::new();
    for alias in aliases {
        candidates.push(modules_dir.join(format!("{}.phpx", alias)));
        candidates.push(modules_dir.join(format!("{}.php", alias)));
        candidates.push(modules_dir.join(alias.as_str()).join("index.phpx"));
        candidates.push(modules_dir.join(alias.as_str()).join("index.php"));
        candidates.push(modules_dir.join(alias.as_str()).join("index.js"));
        if alias.ends_with(".phpx") || alias.ends_with(".php") || alias.ends_with(".js") {
            candidates.push(modules_dir.join(alias));
        }
    }

    candidates.into_iter().find(|path| path.is_file())
}

fn resolve_phpx_module_spec(project_root: &Path, specifier: &str) -> Option<PathBuf> {
    // @/ is a project-root alias: @/src/pages/foo -> {project_root}/src/pages/foo.phpx
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
            if let Some(resolved) = resolve_with_candidates(&base) {
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

    let modules_dir = project_root.join("php_modules");
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
        if let Some(resolved) = resolve_with_candidates(&base) {
            return Some(resolved);
        }
    }

    // PHPX_MODULE_ROOT fallback (#220): if the tenant's php_modules/ doesn't
    // contain the spec, try the runtime stdlib root. This lets stdlib-only
    // tenants (e.g. id.tana.gg) deploy without vendoring stdlib.
    if let Some(root_os) = std::env::var_os("PHPX_MODULE_ROOT") {
        let root = std::path::Path::new(&root_os);
        for alias in aliases.iter() {
            let base = if alias.starts_with("@user/") {
                root.join("@user").join(alias.trim_start_matches("@user/"))
            } else {
                root.join(alias)
            };
            if let Some(resolved) = resolve_with_candidates(&base) {
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
    resolve_with_candidates(&base)
}

fn resolve_with_candidates(target: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if target.extension().is_none() {
        candidates.push(target.with_extension("phpx"));
        candidates.push(target.with_extension("js"));
        candidates.push(target.join("index.phpx"));
        candidates.push(target.join("index.js"));
    }
    candidates.push(target.to_path_buf());

    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn is_bare_specifier(spec: &str) -> bool {
    is_bare_module_specifier(spec)
}

fn append_entry_footer(code: ModuleSourceCode) -> ModuleSourceCode {
    const FOOTER: &str = "\nif (typeof globalThis.app === \"undefined\" && typeof app !== \"undefined\") {\n\
  const __candidate = app;\n\
  if (typeof __candidate === \"function\" && typeof globalThis.__dekaNodeExpressAdapter === \"function\" && (typeof __candidate.handle === \"function\" || typeof __candidate.listen === \"function\")) {\n\
    globalThis.app = globalThis.__dekaNodeExpressAdapter(__candidate);\n\
  } else if (__candidate && typeof __candidate === \"object\" && !__candidate.__dekaServer && (typeof __candidate.fetch === \"function\" || typeof __candidate.routes === \"object\")) {\n\
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
