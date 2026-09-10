//! ESM module loader for the V8 isolate pool.
//!
//! `PhpxEsmLoader` compiles DekaScript through dsc (or serves a pre-compiled
//! module graph), classifies every module path for RFD 27 host-bridge grants,
//! and prepends the per-module host-bindings preamble. Seams live in sibling
//! modules:
//!
//! - [`resolver`]: specifier → filesystem-path resolution and project-root
//!   discovery.
//! - [`grants`]: manifest reads, lockfile digests, package-name derivation
//!   feeding the RFD 27 grant table.
//! - [`graph_hash`]: import-graph hashing for cache invalidation.
//! - [`policy`]: pre-execution gates (`security.allow.dynamic`, project
//!   layout).
//! - [`transforms`]: wrapper template, host-bindings preamble, entry footer.

use std::cell::RefCell;
use std::collections::HashMap;
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

use runtime_core::host_bridge::{self, GrantTable};

mod grants;
mod graph_hash;
mod policy;
mod resolver;
mod transforms;

pub use graph_hash::hash_module_graph;
pub use policy::ensure_project_layout;
pub use resolver::{entry_wrapper_path, is_javascript_entry, resolve_project_root};

use grants::{dependency_package_name, read_manifest_host_kinds, read_manifest_name, read_lock_digests};
use resolver::{parse_module_imports, resolve_phpx_module_spec};
use transforms::{append_entry_footer, entry_wrapper_source, prepend_host_bindings};

/// Write a ui cache file atomically, skipping the write when the content is
/// already identical. deka#745: concurrent isolate loads read these files with
/// plain `read_to_string`, so a truncate-then-write `fs::write` could be
/// observed as an empty module (preamble still parses, exports vanish). Writes
/// go to a temp file in the same directory followed by `rename`, so a reader
/// sees either the old or the new complete file, never a truncation.
/// Returns `true` when a write happened.
fn write_ui_file_if_changed(path: &Path, source: &str) -> std::io::Result<bool> {
    if let Ok(existing) = std::fs::read_to_string(path)
        && existing == source
    {
        return Ok(false);
    }
    let tmp = path.with_file_name(format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|name| name.to_str()).unwrap_or("ui"),
        std::process::id()
    ));
    std::fs::write(&tmp, source)?;
    std::fs::rename(&tmp, path)?;
    Ok(true)
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
    /// RFD 27 grant table (explicit config or `DEKA_HOST_GRANTS` env).
    grant_table: Option<GrantTable>,
    /// Bridge kinds granted to the project root (cached at construction).
    root_kinds: Vec<String>,
    /// Lockfile-pinned fsGraph digests by package name, read once from
    /// `<project_root>/deka.lock` (defensive inline JSON parse).
    lock_digests: HashMap<String, String>,
    /// Per-package-root resolved kind cache (shared across loader clones).
    package_kinds: Rc<RefCell<HashMap<PathBuf, Vec<String>>>>,
}

impl PhpxEsmLoader {
    pub fn new(
        project_root: PathBuf,
        entry_path: PathBuf,
        host_grants: Option<GrantTable>,
    ) -> Result<Self, JsErrorBox> {
        // Canonicalize both paths before any prefix comparison. dsc's graph
        // dump and `fs::canonicalize` both resolve macOS's /var → /private/var
        // symlink, while callers (pool module_root, CLI paths) usually hand us
        // the /var spelling; a project_root that is not a prefix of the module
        // paths would silently fall out of the workspace-grant rule and every
        // bridge call would report "not granted any host kinds".
        let project_root = project_root.canonicalize().unwrap_or(project_root);
        let entry_path = entry_path.canonicalize().unwrap_or(entry_path);
        let cache_dir = runtime_core::framework::compiler_cache_dir(&project_root);
        std::fs::create_dir_all(&cache_dir).map_err(|err| {
            JsErrorBox::generic(format!("failed to create {}: {}", cache_dir.display(), err))
        })?;
        let entry_specifier = ModuleSpecifier::from_file_path(&entry_path)
            .map_err(|_| JsErrorBox::generic("invalid entry module path"))?;
        let wrapper_specifier = ModuleSpecifier::from_file_path(entry_wrapper_path(&project_root))
            .map_err(|_| JsErrorBox::generic("invalid entry wrapper path"))?;

        // RFD 27: resolve the host grant table. An explicit table wins; until
        // the registry/index plumbing lands, production falls back to the
        // DEKA_HOST_GRANTS env var (a grant-table JSON document).
        let grant_table = match host_grants {
            Some(table) => Some(table),
            None => std::env::var("DEKA_HOST_GRANTS")
                .ok()
                .and_then(|json| match GrantTable::from_json(&json) {
                    Ok(table) => Some(table),
                    Err(err) => {
                        tracing::warn!("ignoring invalid DEKA_HOST_GRANTS: {err}");
                        None
                    }
                }),
        };

        // Eager project-root check: only official `@deka/*` packages may
        // self-declare `host.kinds` in deka.json; an application root doing so
        // is a compile error, not a silent grant.
        let (root_kinds, root_name) = read_manifest_host_kinds(&project_root);
        let root_kinds = match host_bridge::resolve_project_root_grants(
            &root_name,
            &project_root.join("deka.json"),
            root_kinds.as_deref(),
        ) {
            Ok(kinds) => kinds,
            Err(err) => {
                return Err(JsErrorBox::generic(format!(
                    "{err}; deka.json 'host.kinds' is reserved for official @deka stdlib packages; \
                     apps acquire bridge authority only via the published grant table"
                )));
            }
        };

        let lock_digests = read_lock_digests(&project_root);

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
            policy::ensure_project_layout(&project_root, &imports)
                .map_err(JsErrorBox::generic)?;
            policy::enforce_dynamic_policy(&modules)?;
            Some(modules)
        } else {
            None
        };

        let loader = Self {
            project_root,
            cache_dir,
            entry_specifier,
            wrapper_specifier,
            sources: Rc::new(RefCell::new(HashMap::new())),
            v2_modules,
            grant_table,
            root_kinds,
            lock_digests,
            package_kinds: Rc::new(RefCell::new(HashMap::new())),
        };

        // Static pre-check: any compiled module that references `__deka_host(`
        // must belong to a package with at least one granted kind. Per-kind
        // precision is the per-call gate's job; this just refuses to boot a
        // graph whose bridge caller can never succeed.
        if let Some(modules) = &loader.v2_modules {
            let mut paths: Vec<&PathBuf> = modules.keys().collect();
            paths.sort();
            for path in paths {
                if !modules[path].contains("__deka_host(") {
                    continue;
                }
                let kinds = loader.kinds_for_path(path);
                if kinds.is_empty() {
                    let name = loader.package_name_for_path(path);
                    return Err(JsErrorBox::generic(format!(
                        "package '{}' is not granted any host kinds but contains bridge calls ({})",
                        name,
                        path.display()
                    )));
                }
            }
        }

        Ok(loader)
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

    /// Classify a module path for RFD 27 grant purposes and return the bridge
    /// kinds it may call. Order matters: the compiler-cache `ui/` dir and
    /// `ds_modules|php_modules` trees are checked before the generic
    /// "under project root" rule.
    fn kinds_for_path(&self, path: &Path) -> Vec<String> {
        // `self.project_root`/`cache_dir` are canonicalized at construction;
        // callers may hand us the macOS /var spelling (dsc graph keys are
        // canonical, pool paths usually are not). Canonicalize for the prefix
        // comparisons so both spellings classify identically.
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path: &Path = &canonical;
        // Materialized `ui/*` toolchain modules (crates/deka_ui/js): part of
        // the deka distribution like the prelude, not userland. deka_ui's
        // embedded server calls crypto.random_bytes (defer nonces) and the
        // AES-GCM helpers — crypto is all that's needed (grep deka_ui/js for
        // `host(`). Grant exactly that.
        if path.starts_with(self.cache_dir.join("ui")) {
            return vec!["crypto".to_string()];
        }

        if let Some(package_root) = self.dependency_package_root(path) {
            if let Some(kinds) = self.package_kinds.borrow().get(&package_root) {
                return kinds.clone();
            }
            let name = dependency_package_name(&package_root, &self.project_root);
            let digest = self.lock_digests.get(&name);
            let kinds = match &self.grant_table {
                Some(table) => {
                    host_bridge::resolve_dependency_grants(table, digest.map(String::as_str))
                }
                None => Vec::new(),
            };
            self.package_kinds
                .borrow_mut()
                .insert(package_root, kinds.clone());
            return kinds;
        }

        if path.starts_with(&self.project_root) {
            return self.root_kinds.clone();
        }

        // Linked packages outside the project root (lazily compiled via
        // `load_ds_source`): read their own deka.json for the name and treat
        // them as a dependency of this project.
        let package_root = path
            .ancestors()
            .find(|ancestor| ancestor.join("deka.json").is_file())
            .map(Path::to_path_buf);
        if let Some(package_root) = package_root {
            if let Some(kinds) = self.package_kinds.borrow().get(&package_root) {
                return kinds.clone();
            }
            let name = read_manifest_name(&package_root).unwrap_or_default();
            let digest = self.lock_digests.get(&name);
            // Dependency manifests' host.kinds are untrusted per RFD 27 — only
            // the grant table via the lockfile digest grants kinds.
            let kinds = match &self.grant_table {
                Some(table) => {
                    host_bridge::resolve_dependency_grants(table, digest.map(String::as_str))
                }
                None => Vec::new(),
            };
            self.package_kinds
                .borrow_mut()
                .insert(package_root, kinds.clone());
            return kinds;
        }

        Vec::new()
    }

    /// If `path` lives under `<project_root>/{ds_modules,php_modules}/<name>`,
    /// return the package root directory. Scoped names (`@deka/crypto`) take
    /// two path segments.
    fn dependency_package_root(&self, path: &Path) -> Option<PathBuf> {
        for dir in ["ds_modules", "php_modules"] {
            let modules_dir = self.project_root.join(dir);
            if let Ok(rel) = path.strip_prefix(&modules_dir) {
                let mut components = rel.components();
                let first = components.next()?;
                let mut root = modules_dir.join(first.as_os_str());
                let file_name = first.as_os_str().to_string_lossy();
                if file_name.starts_with('@') {
                    let second = components.next()?;
                    root = root.join(second.as_os_str());
                }
                return Some(root);
            }
        }
        None
    }

    /// Best-effort package name for diagnostics: the dependency segment when
    /// under a modules dir, else the nearest manifest name, else the path.
    fn package_name_for_path(&self, path: &Path) -> String {
        if let Some(root) = self.dependency_package_root(path) {
            return dependency_package_name(&root, &self.project_root);
        }
        path.ancestors()
            .find(|ancestor| ancestor.join("deka.json").is_file())
            .and_then(|root| read_manifest_name(root))
            .unwrap_or_else(|| path.display().to_string())
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

    /// Build values are compiler-addressed virtual modules. The Deka host
    /// writes them only after a successful build phase, under the project
    /// cache rather than beside user sources or in the published output.
    ///
    /// Resolution is cache-first: `deka dev` materializes into the cache and
    /// keeps working unchanged. Only when the cache module is absent does the
    /// resolver fall back to `dist/app/.build-values/` — the copy `deka
    /// build` ships inside the published tree so a deployment carrying only
    /// `dist/` serves build-backed routes (deka#738 F7).
    fn resolve_build_value_module(&self, specifier: &str) -> Option<PathBuf> {
        let id = specifier.strip_prefix("deka:dev/")?;
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return None;
        }
        let path = runtime_core::framework::compiler_cache_dir(&self.project_root)
            .join("build-values")
            .join(format!("{id}.js"));
        if path.is_file() {
            return Some(path);
        }
        let dist_path = self
            .project_root
            .join("dist")
            .join("app")
            .join(".build-values")
            .join(format!("{id}.js"));
        dist_path.is_file().then_some(dist_path)
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
        if let Err(err) = write_ui_file_if_changed(&path, source) {
            tracing::warn!("failed to write {}: {}", path.display(), err);
            return None;
        }
        // Ensure siblings exist so `import from "./jsx.js"` works.
        for spec in deka_ui::SPECIFIERS {
            if let (Some(src), Some(name)) = (deka_ui::source_for(spec), deka_ui::file_name_for(spec)) {
                let sibling = dir.join(name);
                let _ = write_ui_file_if_changed(&sibling, src);
            }
        }
        Some(path)
    }

    fn resolve_path(&self, specifier: &str, referrer: &str) -> Result<ModuleSpecifier, JsErrorBox> {
        if resolver::is_bare_specifier(specifier) {
            if let Some(path) = self.resolve_build_value_module(specifier) {
                return ModuleSpecifier::from_file_path(path)
                    .map_err(|_| JsErrorBox::generic("invalid build value module path"));
            }
            if let Some(path) = self.materialize_ui_module(specifier) {
                return ModuleSpecifier::from_file_path(path)
                    .map_err(|_| JsErrorBox::generic("invalid ui module path"));
            }
            if let Some(path) = self.resolve_phpx_module_spec(specifier) {
                return ModuleSpecifier::from_file_path(path)
                    .map_err(|_| JsErrorBox::generic("invalid module path"));
            }
            let message = if specifier.starts_with("deka:dev/") {
                format!(
                    "build value '{}' has not been materialized; run deka build",
                    specifier
                )
            } else {
                format!(
                    "unable to resolve module '{}'; check php_modules",
                    specifier
                )
            };
            return Err(JsErrorBox::generic(message));
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
        code = prepend_host_bindings(code, &self.kinds_for_path(&path));
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
        entry_wrapper_source(&self.entry_specifier.to_string())
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

#[cfg(test)]
mod tests {
    use super::{PhpxEsmLoader, write_ui_file_if_changed};
    use std::fs;

    #[test]
    fn source_extensions_have_distinct_cache_paths() {
        let root = tempfile::tempdir().expect("temp project");
        let loader = PhpxEsmLoader::new(
            root.path().to_path_buf(),
            root.path().join("main.ds"),
            None,
        )
        .expect("loader");

        let ds = loader.cache_path_for(&root.path().join("main.ds"));
        let js = loader.cache_path_for(&root.path().join("main.js"));
        assert_ne!(ds, js);
        assert!(ds.ends_with("main.ds.js"));
        assert!(js.ends_with("main.js.js"));
    }

    #[test]
    fn materialize_ui_module_writes_embedded_sources() {
        let root = tempfile::tempdir().expect("temp project");
        let entry = root.path().join("handler.js");
        fs::write(&entry, "export default {};\n").expect("write js handler");
        let loader = PhpxEsmLoader::new(root.path().to_path_buf(), entry, None).expect("loader");

        let path = loader
            .materialize_ui_module("ui/jsx")
            .expect("ui/jsx materializes");

        // Primary file and every sibling match the embedded deka_ui sources.
        for spec in deka_ui::SPECIFIERS {
            let name = deka_ui::file_name_for(spec).expect("file name");
            let source = deka_ui::source_for(spec).expect("source");
            let on_disk = fs::read_to_string(path.parent().unwrap().join(name))
                .expect("sibling materialized");
            assert_eq!(on_disk, source, "{name} mismatch");
        }
    }

    #[test]
    fn write_ui_file_if_changed_skips_identical_content() {
        let root = tempfile::tempdir().expect("temp project");
        let path = root.path().join("reactive.js");
        let source = deka_ui::source_for("ui/reactive").expect("source");

        assert!(
            write_ui_file_if_changed(&path, source).expect("first write"),
            "first write should happen"
        );
        assert_eq!(fs::read_to_string(&path).expect("read"), source);

        assert!(
            !write_ui_file_if_changed(&path, source).expect("second write"),
            "identical content must not be rewritten (deka#745)"
        );
        assert_eq!(fs::read_to_string(&path).expect("read"), source);

        // No temp file left behind.
        let leftovers: Vec<_> = fs::read_dir(root.path())
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");
    }

    /// RFD 27: a ds_modules package module resolves its bridge kinds from the
    /// grant table via the lockfile-pinned fsGraph digest.
    #[test]
    fn dependency_module_kinds_come_from_grant_table_via_lock_digest() {
        use runtime_core::host_bridge::GrantTable;

        let root = tempfile::tempdir().expect("temp project");
        let entry = root.path().join("handler.js");
        fs::write(&entry, "export default {};\n").expect("write js handler");

        let package_dir = root.path().join("ds_modules").join("@deka").join("crypto");
        fs::create_dir_all(&package_dir).expect("package dir");
        let module = package_dir.join("index.ds");
        fs::write(&module, "export const x = 1;\n").expect("package module");

        fs::write(
            root.path().join("deka.lock"),
            r#"{
                "lockfileVersion": 1,
                "packages": {
                    "@deka/crypto": {
                        "metadata": { "fsGraph": { "algo": "sha256", "hash": "sha256:aaa" } }
                    }
                }
            }"#,
        )
        .expect("deka.lock");

        let table = GrantTable::from_json(
            r#"[{"name":"@deka/crypto","version":"1.0.0","digest":"sha256:aaa","kinds":["crypto"]}]"#,
        )
        .expect("grant table");
        let loader =
            PhpxEsmLoader::new(root.path().to_path_buf(), entry, Some(table)).expect("loader");

        assert_eq!(loader.kinds_for_path(&module), vec!["crypto".to_string()]);
        // Root-owned sources get no kinds (this fixture root declares none).
        assert!(loader.kinds_for_path(&root.path().join("src").join("main.ds")).is_empty());
    }
}
