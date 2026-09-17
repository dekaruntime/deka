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
use std::sync::OnceLock;

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

use deka_modules::modules::MODULES_DIR;
use permissions::host_bridge::{self, GrantTable};

mod grants;
mod graph_hash;
pub mod import_meta_ops;
mod policy;
mod resolver;
mod transforms;

pub use graph_hash::hash_module_graph;
pub use policy::ensure_project_layout;
pub use resolver::{entry_wrapper_path, entry_wrapper_path_with, is_javascript_entry, resolve_project_root};
pub use transforms::entry_wrapper_source;

use grants::{
    dependency_package_name, read_lock_digests, read_manifest_host_kinds, read_manifest_name,
    read_project_grant_table,
};
use resolver::{parse_module_imports, resolve_phpx_module_spec};
use transforms::{append_entry_footer, prepend_host_bindings, prepend_import_meta};

#[derive(Clone)]
pub struct PhpxEsmLoader {
    project_root: PathBuf,
    /// Explicit stdlib root for a stdlib-only tenant. This is intentionally
    /// distinct from `project_root`; ordinary projects pass their own root.
    module_root: Option<PathBuf>,
    /// Compiler selected by the caller for source-posture execution. It is
    /// absent for artifact-only serve, which must never compile.
    dsc: Option<PathBuf>,
    cache_dir: PathBuf,
    entry_specifier: ModuleSpecifier,
    wrapper_specifier: ModuleSpecifier,
    sources: Rc<RefCell<HashMap<String, ModuleSourceCode>>>,
    /// Pre-compiled module graph for compiler v2. When present, `.ds` files
    /// are served from this map instead of compiled on demand.
    v2_modules: Option<HashMap<PathBuf, String>>,
    /// RFD 27 grant table: explicit caller configuration, else the
    /// project-installed `deka.grants.json` (deka#797).
    grant_table: Option<GrantTable>,
    /// Bridge kinds granted to the project root (cached at construction).
    root_kinds: Vec<String>,
    /// Lockfile-pinned fsGraph digests by package name, read once from
    /// `<project_root>/deka.lock` (defensive inline JSON parse).
    lock_digests: HashMap<String, String>,
    /// Per-package-root resolved kind cache (shared across loader clones).
    package_kinds: Rc<RefCell<HashMap<PathBuf, Vec<String>>>>,
    /// Set only for a manifest-verified `dist/server` entry. Artifact
    /// posture has a deliberately smaller resolver than source posture: no
    /// compiler input, no extension guessing, no package/cache fallback, and
    /// no path outside this root (deka#743/#763).
    artifact_server_root: Option<PathBuf>,
    /// Set when this loader executes a compiled or staged copy of the real
    /// source tree rather than the tree itself: a loose `deka run` file's
    /// user-cache artifact (`~/.deka/cache/loose/<hash>/out/`), or a
    /// `deka build`/`deka dev` build-slot staging directory. `import.meta.url`
    /// / `.dirname` / `.filename` must report the *original* source path in
    /// both cases (rfd#12 amendment, deka#1139) — never the compiled copy.
    source_override: Option<SourceOverride>,
}

/// See [`PhpxEsmLoader::source_override`]. The compiled tree mirrors the
/// original tree's relative directory structure (both `dsc transpile` and
/// the build-slot stager preserve it), so mapping a compiled path back to
/// its original is: strip `compiled_root`, re-join under `original_root`,
/// then recover the source extension (dsc always names compiled output
/// `<stem>.js`, whatever the source extension was).
#[derive(Clone, Debug)]
struct SourceOverride {
    compiled_root: PathBuf,
    original_root: PathBuf,
}

/// Process-wide hint that this process executes a compiled/staged copy of
/// the real source tree rather than the tree itself (deka#1139, rfd#12
/// amendment). The dispatch layer (`crates/runtime`) installs this once,
/// before engine/isolate construction, exactly when it materializes such a
/// copy — a loose `deka run` file's user-cache artifact today; a `deka
/// build`/`deka dev` build-slot staging directory is the other case the RFD
/// amendment names. Every [`PhpxEsmLoader`] constructed afterward in this
/// process picks it up, mirroring the "first install wins" pattern
/// `deka_host::host_config` already uses for `module_root`/`handler_path`
/// (deka#801) — kept local to this crate (rather than routed through
/// `deka_host`) since the module loader is the only consumer and
/// `deka_host`'s `runtime` feature pulls in dependencies (postgres, mysql,
/// tokio-tls, …) this crate's production build otherwise never needs.
#[derive(Clone, Debug)]
pub struct SourceOverrideHint {
    pub compiled_root: PathBuf,
    pub original_root: PathBuf,
}

static SOURCE_OVERRIDE_HINT: OnceLock<SourceOverrideHint> = OnceLock::new();

/// Install the source-override hint for this process. Later calls are
/// ignored (first install wins) — see [`SourceOverrideHint`].
pub fn install_source_override_hint(hint: SourceOverrideHint) {
    let _ = SOURCE_OVERRIDE_HINT.set(hint);
}

fn source_override_hint() -> Option<&'static SourceOverrideHint> {
    SOURCE_OVERRIDE_HINT.get()
}

impl PhpxEsmLoader {
    pub fn new(
        project_root: PathBuf,
        entry_path: PathBuf,
        module_root: Option<PathBuf>,
        host_grants: Option<GrantTable>,
        dsc: Option<PathBuf>,
        dev_mode: bool,
    ) -> Result<Self, JsErrorBox> {
        // Canonicalize both paths before any prefix comparison. dsc's graph
        // dump and `fs::canonicalize` both resolve macOS's /var → /private/var
        // symlink, while callers (pool module_root, CLI paths) usually hand us
        // the /var spelling; a project_root that is not a prefix of the module
        // paths would silently fall out of the workspace-grant rule and every
        // bridge call would report "not granted any host kinds".
        let project_root = project_root.canonicalize().unwrap_or(project_root);
        let module_root = match module_root {
            Some(root) => Some(root.canonicalize().unwrap_or(root)),
            None => policy::configured_module_root(&project_root)?,
        };
        let entry_path = entry_path.canonicalize().unwrap_or(entry_path);
        let artifact_server_root = artifact_server_root(&entry_path)?;
        let cache_dir = runtime_core::dist::compiler_cache_dir_with(&project_root, dev_mode);
        if artifact_server_root.is_none() {
            std::fs::create_dir_all(&cache_dir).map_err(|err| {
                JsErrorBox::generic(format!("failed to create {}: {}", cache_dir.display(), err))
            })?;
        }
        let entry_specifier = ModuleSpecifier::from_file_path(&entry_path)
            .map_err(|_| JsErrorBox::generic("invalid entry module path"))?;
        let wrapper_specifier = ModuleSpecifier::from_file_path(entry_wrapper_path_with(&project_root, dev_mode))
            .map_err(|_| JsErrorBox::generic("invalid entry wrapper path"))?;

        // RFD 27 (deka#797): an explicit caller table wins; otherwise the
        // project-installed table is the production source. A process-wide
        // grant override would let unrelated code widen bridge authority.
        let grant_table = match host_grants {
            Some(table) => Some(table),
            None => read_project_grant_table(&project_root),
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
            let modules = match dsc.as_deref() {
                Some(dsc) => {
                    crate::dsc_compile::compile_graph_with_dsc(&project_root, &entry_path, dsc)
                }
                None => crate::dsc_compile::compile_graph(&project_root, &entry_path)
                    .map(|(modules, _dsc)| modules),
            }
            .map_err(JsErrorBox::generic)?;
            let imports: Vec<String> = modules
                .keys()
                .filter_map(|path| std::fs::read_to_string(path).ok())
                .flat_map(|source| parse_module_imports(&source))
                .collect();
            policy::ensure_project_layout(&project_root, module_root.clone(), &imports)
                .map_err(JsErrorBox::generic)?;
            policy::enforce_dynamic_policy(&modules)?;
            Some(modules)
        } else {
            None
        };

        // The dispatch layer installs this once, ahead of engine/isolate
        // construction, exactly when it runs a compiled/staged copy of the
        // source instead of the source itself (a loose `deka run` file's
        // user-cache artifact today; a `deka build`/`deka dev` staging
        // directory is the other case rfd#12's amendment names). Every
        // loader in the process picks it up the same way `deka_host`'s
        // `host_config` module already does for `module_root`/`handler_path`
        // (deka#801) — deka#1139.
        // Canonicalized for the same reason `project_root`/`entry_path` are
        // above: macOS resolves `/var` to `/private/var`, and
        // `logical_source_path`'s `strip_prefix` must compare like spellings
        // or every path silently falls through to "no override" (deka#1139).
        let source_override = source_override_hint().map(|hint| SourceOverride {
            compiled_root: hint
                .compiled_root
                .canonicalize()
                .unwrap_or_else(|_| hint.compiled_root.clone()),
            original_root: hint
                .original_root
                .canonicalize()
                .unwrap_or_else(|_| hint.original_root.clone()),
        });

        let loader = Self {
            project_root,
            module_root,
            dsc,
            cache_dir,
            entry_specifier,
            wrapper_specifier,
            sources: Rc::new(RefCell::new(HashMap::new())),
            v2_modules,
            grant_table,
            root_kinds,
            lock_digests,
            package_kinds: Rc::new(RefCell::new(HashMap::new())),
            artifact_server_root,
            source_override,
        };

        // Static pre-check: any compiled module that references `__deka_host(`
        // must belong to a package with at least one granted kind. Per-kind
        // precision is the per-call gate's job; this just refuses to boot a
        // graph whose bridge caller can never succeed.
        if let Some(modules) = &loader.v2_modules {
            let mut paths: Vec<&PathBuf> = modules.keys().collect();
            paths.sort();
            for path in &paths {
                if !modules[*path].contains("__deka_host(") {
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

    /// Declare that this loader executes a compiled/staged copy of
    /// `original_root` living under `compiled_root`. Callers whose entry path
    /// is not the true source (the loose-file user cache, a build-slot
    /// staging directory) set this once, right after construction, so
    /// `import.meta.url`/`dirname`/`filename` report the real source instead
    /// of the copy (rfd#12 amendment, deka#1139).
    pub fn set_source_override(&mut self, compiled_root: PathBuf, original_root: PathBuf) {
        // Canonicalized so `strip_prefix` in `logical_source_path` compares
        // like spellings against the canonical paths every module specifier
        // already carries (macOS's `/var` vs `/private/var`, see `new()`).
        self.source_override = Some(SourceOverride {
            compiled_root: compiled_root
                .canonicalize()
                .unwrap_or(compiled_root),
            original_root: original_root
                .canonicalize()
                .unwrap_or(original_root),
        });
    }

    /// Map a resolved module path back to the original source file it was
    /// compiled or staged from. Identity when no override is set, or when
    /// `path` is not under the override's `compiled_root` (project-root
    /// imports outside a loose file's own directory, `ds_modules`
    /// dependencies, etc. — those were never copied anywhere).
    fn logical_source_path(&self, path: &Path) -> PathBuf {
        let Some(over) = &self.source_override else {
            return path.to_path_buf();
        };
        let Ok(rel) = path.strip_prefix(&over.compiled_root) else {
            return path.to_path_buf();
        };
        let candidate = over.original_root.join(rel);
        for ext in ["ds", "dsx"] {
            let with_source_ext = candidate.with_extension(ext);
            if with_source_ext.is_file() {
                return with_source_ext;
            }
        }
        candidate
    }

    /// Backing implementation for `import.meta.resolve()`: the same
    /// specifier resolution an `import` statement goes through
    /// (`resolve_path`, lockfile-first for bare specifiers), then reported
    /// through the same original-source mapping as `import.meta.url`.
    pub(crate) fn resolve_for_import_meta(
        &self,
        specifier: &str,
        referrer: &str,
    ) -> Result<String, String> {
        let resolved = self
            .resolve_path(specifier, referrer)
            .map_err(|err| err.to_string())?;
        let Ok(path) = resolved.to_file_path() else {
            return Ok(resolved.to_string());
        };
        let logical = self.logical_source_path(&path);
        ModuleSpecifier::from_file_path(&logical)
            .map(|url| url.to_string())
            .map_err(|_| format!("cannot form a module URL for {}", logical.display()))
    }

    /// JavaScript prepended ahead of a DekaScript/JS module's compiled body
    /// that overrides `import.meta` in place (rfd#12 amendment, deka#1139):
    /// deno_core's host-provided object already exists by the time module
    /// code runs, with a writable/configurable `url` (set to the *load-time*
    /// specifier — the compiled/staged copy when one is in play) and `main`
    /// (computed from deno_core's own main-module bookkeeping, which is
    /// wrong here: `deka run`/`deka serve` always evaluate a loader-owned
    /// wrapper as the true main module and dynamically `import()` the user's
    /// entry, so deno_core's `main` is `false` for every DekaScript module).
    /// Overriding from inside the module itself is the only way to correct
    /// both without a native V8 host-callback override.
    fn import_meta_prelude(&self, specifier: &ModuleSpecifier, path: &Path) -> String {
        let logical = self.logical_source_path(path);
        let url = ModuleSpecifier::from_file_path(&logical)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| logical.display().to_string());
        let dirname = logical
            .parent()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default();
        let filename = logical.display().to_string();
        let main = specifier == &self.entry_specifier;
        let raw_specifier = specifier.to_string();
        format!(
            "Object.defineProperty(import.meta, \"url\", {{ value: {url}, enumerable: true, configurable: true }});\n\
             Object.defineProperty(import.meta, \"dirname\", {{ value: {dirname}, enumerable: true, configurable: true }});\n\
             Object.defineProperty(import.meta, \"filename\", {{ value: {filename}, enumerable: true, configurable: true }});\n\
             Object.defineProperty(import.meta, \"main\", {{ value: {main}, enumerable: true, configurable: true }});\n\
             Object.defineProperty(import.meta, \"resolve\", {{ value: function(specifier) {{ const __h = globalThis[Symbol.for('deka.host.internal')]; if (!__h || typeof __h.resolveImportMeta !== \"function\") {{ throw new Error(\"import.meta.resolve is unavailable\"); }} return __h.resolveImportMeta(String(specifier), {raw_specifier}); }}, enumerable: true, configurable: false }});\n\
             Object.freeze(import.meta);\n",
            url = serde_json::to_string(&url).unwrap_or_else(|_| "\"\"".to_string()),
            dirname = serde_json::to_string(&dirname).unwrap_or_else(|_| "\"\"".to_string()),
            filename = serde_json::to_string(&filename).unwrap_or_else(|_| "\"\"".to_string()),
            main = main,
            raw_specifier = serde_json::to_string(&raw_specifier).unwrap_or_else(|_| "\"\"".to_string()),
        )
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
    /// kinds it may call. Order matters: the compiler-cache and
    /// `ds_modules` trees are checked before the generic
    /// "under project root" rule.
    fn kinds_for_path(&self, path: &Path) -> Vec<String> {
        // `self.project_root`/`cache_dir` are canonicalized at construction;
        // callers may hand us the macOS /var spelling (dsc graph keys are
        // canonical, pool paths usually are not). Canonicalize for the prefix
        // comparisons so both spellings classify identically.
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path: &Path = &canonical;

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

    /// If `path` lives under `<project_root>/ds_modules/<name>`, return the
    /// package root directory. Scoped names (`@deka/crypto`) take two path
    /// segments. A sibling `php_modules/` tree is not a resolution fallback.
    fn dependency_package_root(&self, path: &Path) -> Option<PathBuf> {
        let modules_dir = self.project_root.join(MODULES_DIR);
        let rel = path.strip_prefix(&modules_dir).ok()?;
        let mut components = rel.components();
        let first = components.next()?;
        let mut root = modules_dir.join(first.as_os_str());
        let file_name = first.as_os_str().to_string_lossy();
        if file_name.starts_with('@') {
            let second = components.next()?;
            root = root.join(second.as_os_str());
        }
        Some(root)
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
        let js = match self.dsc.as_deref() {
            Some(dsc) => crate::dsc_compile::compile_file_with_dsc(path, dsc),
            None => crate::dsc_compile::compile_file(path),
        }
        .map_err(JsErrorBox::generic)?;
        Ok(ModuleSourceCode::String(js.into()))
    }

    fn resolve_phpx_module_spec(&self, specifier: &str) -> Option<PathBuf> {
        resolve_phpx_module_spec(&self.project_root, self.module_root.as_deref(), specifier)
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
        let path = self
            .cache_dir
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

    fn resolve_path(&self, specifier: &str, referrer: &str) -> Result<ModuleSpecifier, JsErrorBox> {
        if let Some(url) = crate::js_builtins::resolve_specifier(specifier) {
            return Ok(url);
        }
        if let Some(server_root) = &self.artifact_server_root {
            return self.resolve_artifact_path(server_root, specifier, referrer);
        }
        if resolver::is_bare_specifier(specifier) {
            if let Some(path) = self.resolve_build_value_module(specifier) {
                return ModuleSpecifier::from_file_path(path)
                    .map_err(|_| JsErrorBox::generic("invalid build value module path"));
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
                    "unable to resolve module '{}'; check {MODULES_DIR}",
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
        if resolved.scheme() == "file"
            || resolved.scheme() == "ext"
            || crate::js_builtins::is_builtin_url(&resolved)
        {
            return Ok(resolved);
        }
        Err(JsErrorBox::generic(format!(
            "unsupported module scheme: {}",
            resolved
        )))
    }

    fn resolve_artifact_path(
        &self,
        server_root: &Path,
        specifier: &str,
        referrer: &str,
    ) -> Result<ModuleSpecifier, JsErrorBox> {
        // The isolate starts at this loader-owned synthetic wrapper. It is
        // intentionally outside dist/server, but never read from disk: the
        // `load_source` branch below supplies its generated source. Treat
        // only its exact canonical specifier as an artifact entrypoint; all
        // application imports remain jailed to explicit relative .js files.
        if specifier == self.wrapper_specifier.as_str() {
            return Ok(self.wrapper_specifier.clone());
        }
        if let Some(url) = crate::js_builtins::resolve_specifier(specifier) {
            return Ok(url);
        }
        if resolver::is_bare_specifier(specifier) {
            if specifier.starts_with("deka:dev/") {
                return Err(JsErrorBox::generic(format!(
                    "artifact module imports `{specifier}`, but deka:dev/* is a dev-only scheme; rebuild with `deka build`"
                )));
            }
            return Err(JsErrorBox::generic(format!(
                "artifact module imports unsupported bare specifier `{specifier}`; artifact server modules may resolve only explicit relative .js files"
            )));
        }
        let wrapper_import =
            referrer == self.wrapper_specifier.as_str() && specifier.starts_with("file:");
        if !wrapper_import && !specifier.starts_with("./") && !specifier.starts_with("../") {
            return Err(JsErrorBox::generic(format!(
                "artifact module specifier `{specifier}` must be an explicit relative .js path"
            )));
        }
        if !specifier.ends_with(".js") {
            return Err(JsErrorBox::generic(format!(
                "artifact module specifier `{specifier}` must name an explicit .js file"
            )));
        }
        let resolved = resolve_import(specifier, referrer).map_err(JsErrorBox::from_err)?;
        if crate::js_builtins::is_builtin_url(&resolved) {
            return Ok(resolved);
        }
        let path = resolved.to_file_path().map_err(|_| {
            JsErrorBox::generic(format!(
                "artifact module specifier `{specifier}` is not a file path"
            ))
        })?;
        let root = server_root
            .canonicalize()
            .unwrap_or_else(|_| server_root.to_path_buf());
        let path = path.canonicalize().map_err(|err| {
            JsErrorBox::generic(format!(
                "artifact module specifier `{specifier}` does not resolve inside dist/server: {err}"
            ))
        })?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(JsErrorBox::generic(format!(
                "artifact module specifier `{specifier}` resolves outside dist/server"
            )));
        }
        ModuleSpecifier::from_file_path(path)
            .map_err(|_| JsErrorBox::generic("invalid artifact module path"))
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
        if let Some(js) = crate::js_builtins::load_esm(specifier) {
            return Ok(ModuleSource::new(
                ModuleType::JavaScript,
                ModuleSourceCode::String(js.to_string().into()),
                specifier,
                None,
            ));
        }
        let raw_path = specifier
            .to_file_path()
            .map_err(|_| JsErrorBox::generic("Only file:// URLs are supported"))?;
        if let Some(server_root) = &self.artifact_server_root {
            let root = server_root
                .canonicalize()
                .unwrap_or_else(|_| server_root.to_path_buf());
            let path = raw_path.canonicalize().map_err(JsErrorBox::from_err)?;
            if !path.starts_with(&root) || !path.is_file() {
                return Err(JsErrorBox::generic(format!(
                    "artifact module {} is outside dist/server",
                    raw_path.display()
                )));
            }
            if !matches!(path.extension().and_then(|ext| ext.to_str()), Some("js")) {
                return Err(JsErrorBox::generic(format!(
                    "artifact module {} is not an ES2022 .js file",
                    path.display()
                )));
            }
            let mut code = self.load_js_source(&path)?;
            code = prepend_import_meta(code, &self.import_meta_prelude(specifier, &path));
            code = prepend_host_bindings(code, &self.kinds_for_path(&path));
            if specifier == &self.entry_specifier {
                code = append_entry_footer(code);
            }
            return Ok(ModuleSource::new(
                ModuleType::JavaScript,
                code,
                specifier,
                None,
            ));
        }
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
        code = prepend_import_meta(code, &self.import_meta_prelude(specifier, &path));
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

/// Detect the artifact posture from the entry itself, independently of the
/// source-project root used for grants. `engine::config` has already verified
/// the descriptor before binding; doing the inexpensive structural check here
/// keeps direct pool users from accidentally treating dist/server as source.
fn artifact_server_root(entry_path: &Path) -> Result<Option<PathBuf>, JsErrorBox> {
    let Some(server_root) = entry_path.ancestors().find(|candidate| {
        candidate.file_name().and_then(|name| name.to_str()) == Some("server")
            && candidate
                .parent()
                .is_some_and(|dist| dist.join("build-manifest.json").is_file())
    }) else {
        return Ok(None);
    };
    let dist_root = server_root.parent().expect("server root has dist parent");
    runtime_core::dist::ArtifactManifestV2::load_verified(dist_root)
        .and_then(|manifest| {
            manifest.ensure_native_compat()?;
            Ok(manifest)
        })
        .map_err(JsErrorBox::generic)?;
    Ok(Some(server_root.to_path_buf()))
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
    use super::PhpxEsmLoader;
    use std::fs;

    #[test]
    fn runtime_js_builtins_resolve_without_install() {
        use deno_core::{ModuleLoader, ModuleSourceCode, ResolutionKind};

        let root = tempfile::tempdir().expect("temp project");
        let entry = root.path().join("handler.js");
        fs::write(&entry, "export default {};\n").expect("write js handler");
        let loader =
            PhpxEsmLoader::new(root.path().to_path_buf(), entry, None, None, None, false)
                .expect("loader");

        let resolved = loader
            .resolve(
                "@js/react/jsx-runtime",
                "file:///handler.js",
                ResolutionKind::Import,
            )
            .expect("builtin specifier");
        assert_eq!(resolved.as_str(), "deka:///js/jsx-runtime.js");
        let client = loader
            .resolve(
                "@js/react-dom/client",
                "file:///handler.js",
                ResolutionKind::Import,
            )
            .expect("client builtin");
        assert_eq!(client.as_str(), "deka:///js/react-dom-client.js");
        // deka#1065: the compiler cache nests under ds_modules/.cache, so
        // ds_modules legitimately exists once the loader is constructed
        // even with zero installed packages. The real invariant this test
        // guards is "no package install happened" -- assert that instead
        // of bare directory absence.
        assert!(!root.path().join("ds_modules").join("@deka").exists());
        assert!(!root.path().join("ds_modules").join("deka.lock").exists());
        assert!(!root.path().join("js_modules").exists());

        let source = match loader.load_source(&resolved) {
            Ok(source) => source,
            Err(err) => panic!("load builtin: {err}"),
        };
        let ModuleSourceCode::String(code) = source.code else {
            panic!("expected string source");
        };
        let body = code.as_str();
        assert!(body.contains("exports.jsx"));
        assert!(!body.contains("react.development.js"));
    }

    #[test]
    fn source_extensions_have_distinct_cache_paths() {
        let root = tempfile::tempdir().expect("temp project");
        let loader =
            PhpxEsmLoader::new(root.path().to_path_buf(), root.path().join("main.ds"), None, None, None, false)
                .expect("loader");

        let ds = loader.cache_path_for(&root.path().join("main.ds"));
        let js = loader.cache_path_for(&root.path().join("main.js"));
        assert_ne!(ds, js);
        assert!(ds.ends_with("main.ds.js"));
        assert!(js.ends_with("main.js.js"));
    }


    #[test]
    fn artifact_loader_accepts_its_synthetic_wrapper_as_the_entrypoint() {
        let root = tempfile::tempdir().expect("temp project");
        let server = root.path().join("dist").join("server");
        fs::create_dir_all(&server).expect("create server root");
        let entry = server.join("serve-entry.js");
        fs::write(&entry, "export default {};\n").expect("write server entry");

        // Model the post-verification artifact posture without constructing a
        // full manifest fixture. The wrapper is loader-owned and therefore
        // not a server payload, but Deno resolves it as the main module
        // before the wrapper imports the verified entry.
        let mut loader =
            PhpxEsmLoader::new(root.path().to_path_buf(), entry, None, None, None, false).expect("loader");
        loader.artifact_server_root = Some(server.clone());
        let wrapper = loader.wrapper_specifier.clone();

        let resolved = loader
            .resolve_artifact_path(&server, wrapper.as_str(), "file:///main")
            .expect("synthetic wrapper entrypoint resolves");
        assert_eq!(resolved, wrapper);

        let outside = deno_core::ModuleSpecifier::from_file_path(root.path().join("other.js"))
            .expect("outside fixture URL");
        let err = loader
            .resolve_artifact_path(&server, outside.as_str(), "file:///main")
            .expect_err("unrelated file URL remains rejected");
        assert!(
            err.to_string().contains("explicit relative .js path"),
            "{err}"
        );
    }

    #[test]
    fn dependency_module_kinds_come_from_grant_table_via_lock_digest() {
        use permissions::host_bridge::GrantTable;

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
            PhpxEsmLoader::new(root.path().to_path_buf(), entry, None, Some(table), None, false).expect("loader");

        assert_eq!(loader.kinds_for_path(&module), vec!["crypto".to_string()]);
        // Root-owned sources get no kinds (this fixture root declares none).
        assert!(
            loader
                .kinds_for_path(&root.path().join("src").join("main.ds"))
                .is_empty()
        );
    }

    /// RFD 27 (deka#797): with no explicit table the loader falls back to the project-installed grant table
    /// (`deka.grants.json`, written by `deka add` / `deka install`).
    #[test]
    fn dependency_module_kinds_come_from_project_grant_table_file() {
        let root = tempfile::tempdir().expect("temp project");
        let entry = root.path().join("handler.js");
        fs::write(&entry, "export default {};\n").expect("write js handler");

        let package_dir = root.path().join("ds_modules").join("@deka").join("fs");
        fs::create_dir_all(&package_dir).expect("package dir");
        let module = package_dir.join("index.ds");
        fs::write(&module, "export const x = 1;\n").expect("package module");

        fs::write(
            root.path().join("deka.lock"),
            r#"{
                "lockfileVersion": 1,
                "packages": {
                    "@deka/fs": {
                        "metadata": { "fsGraph": { "algo": "sha256", "hash": "sha256:bbb" } }
                    }
                }
            }"#,
        )
        .expect("deka.lock");

        // The production source is written by the installer and keyed by the
        // lockfile-pinned digest.
        fs::write(
            root.path().join("deka.grants.json"),
            r#"[{"name":"@deka/fs","version":"1.0.0","digest":"sha256:bbb","kinds":["fs"]}]"#,
        )
        .expect("deka.grants.json");

        let loader = PhpxEsmLoader::new(root.path().to_path_buf(), entry, None, None, None, false).expect("loader");

        assert_eq!(loader.kinds_for_path(&module), vec!["fs".to_string()]);
    }

    /// The digest-keyed lookup stays the trust root: a project grant table
    /// whose digest does not match the lockfile pin unlocks nothing.
    #[test]
    fn project_grant_table_with_mismatched_digest_grants_nothing() {
        let root = tempfile::tempdir().expect("temp project");
        let entry = root.path().join("handler.js");
        fs::write(&entry, "export default {};\n").expect("write js handler");

        let package_dir = root.path().join("ds_modules").join("@deka").join("fs");
        fs::create_dir_all(&package_dir).expect("package dir");
        let module = package_dir.join("index.ds");
        fs::write(&module, "export const x = 1;\n").expect("package module");

        fs::write(
            root.path().join("deka.lock"),
            r#"{
                "lockfileVersion": 1,
                "packages": {
                    "@deka/fs": {
                        "metadata": { "fsGraph": { "algo": "sha256", "hash": "sha256:pin" } }
                    }
                }
            }"#,
        )
        .expect("deka.lock");
        // Grant keyed by a *different* digest (e.g. a tampered lockfile pin):
        // the lookup must miss.
        fs::write(
            root.path().join("deka.grants.json"),
            r#"[{"name":"@deka/fs","version":"1.0.0","digest":"sha256:other","kinds":["fs"]}]"#,
        )
        .expect("deka.grants.json");

        let loader = PhpxEsmLoader::new(root.path().to_path_buf(), entry, None, None, None, false).expect("loader");

        assert!(
            loader.kinds_for_path(&module).is_empty(),
            "a grant for another digest must not unlock this package"
        );
    }

    /// deka#896: `php_modules/` is not a pool resolution or grant fallback.
    #[test]
    fn legacy_tree_is_not_a_resolution_fallback() {
        use deka_modules::modules::MODULES_DIR;
        use permissions::host_bridge::GrantTable;

        let project = tempfile::tempdir().expect("temp project");
        let root = project.path();
        let entry = root.join("handler.js");
        fs::write(&entry, "export default {};\n").expect("write js handler");

        let legacy_pkg = root.join("php_modules").join("@deka").join("crypto");
        fs::create_dir_all(&legacy_pkg).expect("legacy package");
        let legacy_module = legacy_pkg.join("index.ds");
        fs::write(&legacy_module, "export const x = 1;\n").expect("legacy module");

        fs::write(
            root.join("deka.lock"),
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
            PhpxEsmLoader::new(root.to_path_buf(), entry, None, Some(table), None, false)
                .expect("loader");

        let legacy_canon = legacy_module.canonicalize().expect("legacy module");
        assert_eq!(loader.dependency_package_root(&legacy_canon), None);
        assert_eq!(
            super::resolve_phpx_module_spec(root, None, "@deka/crypto"),
            None
        );
        assert!(
            loader.kinds_for_path(&legacy_module).is_empty(),
            "php_modules packages must not receive dependency grants"
        );
        let legacy_name = super::dependency_package_name(&legacy_pkg, &loader.project_root);
        assert_ne!(legacy_name, "@deka/crypto");
        assert!(
            legacy_name.contains("php_modules"),
            "legacy tree must not be stripped as a modules prefix: {legacy_name}"
        );

        let err = loader
            .resolve_path("missing-pkg", "file:///main")
            .expect_err("unresolved specifier");
        let message = err.to_string();
        assert!(
            message.contains(MODULES_DIR),
            "unresolved specifier must name {MODULES_DIR}: {message}"
        );
        assert!(
            !message.contains("php_modules"),
            "unresolved specifier must not name php_modules: {message}"
        );

        let modern_pkg = root.join(MODULES_DIR).join("@deka").join("crypto");
        fs::create_dir_all(&modern_pkg).expect("modern package");
        let modern_module = modern_pkg.join("index.ds");
        fs::write(&modern_module, "export const x = 1;\n").expect("modern module");
        let modern_canon = modern_module.canonicalize().expect("modern module");
        let modern_root = modern_pkg
            .canonicalize()
            .unwrap_or_else(|_| modern_pkg.clone());

        assert_eq!(
            loader.dependency_package_root(&modern_canon),
            Some(modern_root.clone())
        );
        let resolved = super::resolve_phpx_module_spec(root, None, "@deka/crypto")
            .expect("ds_modules package must resolve");
        assert!(
            resolved.starts_with(root.join(MODULES_DIR)),
            "resolved path must stay under {MODULES_DIR}: {}",
            resolved.display()
        );
        assert!(
            !resolved
                .components()
                .any(|c| c.as_os_str() == "php_modules"),
            "resolved path must not walk php_modules: {}",
            resolved.display()
        );
        assert_eq!(
            loader.kinds_for_path(&modern_module),
            vec!["crypto".to_string()]
        );
        assert_eq!(
            super::dependency_package_name(&modern_root, &loader.project_root),
            "@deka/crypto"
        );
        assert_eq!(loader.dependency_package_root(&legacy_canon), None);
    }

    // rfd#12 amendment, deka#1139: import.meta runtime values.

    #[test]
    fn entry_reports_its_own_source_and_main_true() {
        // A `.js` entry (a WinterTC worker) needs no dsc on the test
        // machine; the AST/typeck side (dsc#282) already proves `.ds`
        // parses `import.meta` identically, and `import_meta_prelude` does
        // not look at the source language at all.
        let root = tempfile::tempdir().expect("temp project");
        let entry = root.path().join("main.js");
        fs::write(&entry, "export const x = 1;\n").expect("write entry");
        let loader = PhpxEsmLoader::new(root.path().to_path_buf(), entry.clone(), None, None, None, false)
            .expect("loader");

        let entry_specifier = loader.entry_specifier.clone();
        // `load_source` always derives its `path` argument from the (already
        // canonical) specifier, never the caller's original spelling —
        // match that here rather than the raw `entry` variable, which on
        // macOS can differ from the specifier by the `/var` vs
        // `/private/var` symlink and make this assertion flaky depending on
        // where the test's TMPDIR happens to land.
        let canonical_entry_path = entry_specifier.to_file_path().expect("file path");
        let prelude = loader.import_meta_prelude(&entry_specifier, &canonical_entry_path);
        let expected_url = entry_specifier.to_string();
        assert!(
            prelude.contains(&format!("\"url\", {{ value: \"{expected_url}\"")),
            "{prelude}"
        );
        assert!(prelude.contains("\"main\", { value: true"), "{prelude}");
        assert!(prelude.trim_end().ends_with("Object.freeze(import.meta);"));
    }

    #[test]
    fn imported_module_reports_its_own_path_and_main_false() {
        let root = tempfile::tempdir().expect("temp project");
        let entry = root.path().join("main.js");
        let helper = root.path().join("helper.js");
        fs::write(&entry, "import { h } from \"./helper.js\";\n").expect("write entry");
        fs::write(&helper, "export const h = 1;\n").expect("write helper");
        let loader = PhpxEsmLoader::new(root.path().to_path_buf(), entry.clone(), None, None, None, false)
            .expect("loader");

        let helper_specifier =
            deno_core::ModuleSpecifier::from_file_path(helper.canonicalize().expect("canon"))
                .unwrap();
        // See the matching comment in `entry_reports_its_own_source_and_main_true`.
        let canonical_helper_path = helper_specifier.to_file_path().expect("file path");
        let prelude = loader.import_meta_prelude(&helper_specifier, &canonical_helper_path);
        assert!(
            prelude.contains(&helper_specifier.to_string()),
            "helper's own url must appear, not the entry's: {prelude}"
        );
        assert!(
            !prelude.contains(&loader.entry_specifier.to_string()),
            "helper's import.meta must not name the entry: {prelude}"
        );
        assert!(prelude.contains("\"main\", { value: false"), "{prelude}");
    }

    #[test]
    fn source_override_reports_the_original_path_not_the_compiled_copy() {
        // Models a loose `deka run` file: `original_root` is the real source
        // directory, `compiled_root` is the user-cache `out/` tree dsc wrote
        // the compiled `.js` into. Both the entry and a sibling import must
        // report their real `.ds` path, never the compiled `.js` one.
        let original_root = tempfile::tempdir().expect("original root");
        let compiled_root = tempfile::tempdir().expect("compiled root");
        fs::write(original_root.path().join("main.ds"), "import {} from \"./helper.ds\";\n")
            .expect("write original entry");
        fs::write(original_root.path().join("helper.ds"), "export const h = 1;\n")
            .expect("write original helper");
        fs::write(compiled_root.path().join("main.js"), "export {};\n").expect("write compiled entry");
        fs::write(compiled_root.path().join("helper.js"), "export const h = 1;\n")
            .expect("write compiled helper");

        let compiled_entry = compiled_root.path().join("main.js");
        let mut loader = PhpxEsmLoader::new(
            compiled_root.path().to_path_buf(),
            compiled_entry.clone(),
            None,
            None,
            None,
            false,
        )
        .expect("loader");
        loader.set_source_override(
            compiled_root.path().to_path_buf(),
            original_root.path().to_path_buf(),
        );

        let entry_specifier = loader.entry_specifier.clone();
        // See the matching comment in `entry_reports_its_own_source_and_main_true`.
        let canonical_compiled_entry = entry_specifier.to_file_path().expect("file path");
        let prelude = loader.import_meta_prelude(&entry_specifier, &canonical_compiled_entry);
        let expected_entry_url = deno_core::ModuleSpecifier::from_file_path(
            original_root
                .path()
                .canonicalize()
                .expect("canon")
                .join("main.ds"),
        )
        .unwrap()
        .to_string();
        assert!(
            prelude.contains(&format!("\"url\", {{ value: \"{expected_entry_url}\"")),
            "entry must report the original .ds path, not the compiled .js one: {prelude}"
        );
        // `resolve`'s referrer argument is legitimately the compiled
        // specifier (relative resolution must happen in the tree that is
        // actually executing); only `url`/`dirname`/`filename` — what a
        // reader actually sees as "the file's path" — must never carry it.
        let canonical_compiled_root = compiled_root.path().canonicalize().expect("canon");
        for field in ["url", "dirname", "filename"] {
            let line = prelude
                .lines()
                .find(|line| line.contains(&format!("\"{field}\",")))
                .unwrap_or_else(|| panic!("no {field} line in {prelude}"));
            assert!(
                !line.contains(&canonical_compiled_entry.display().to_string())
                    && !line.contains(canonical_compiled_root.to_str().unwrap()),
                "the compiled path must not leak into import.meta.{field}: {line}"
            );
        }
        assert!(prelude.contains("\"main\", { value: true"), "{prelude}");

        let compiled_helper = canonical_compiled_root.join("helper.js");
        let helper_specifier =
            deno_core::ModuleSpecifier::from_file_path(&compiled_helper).unwrap();
        let helper_prelude = loader.import_meta_prelude(&helper_specifier, &compiled_helper);
        let expected_helper_url = deno_core::ModuleSpecifier::from_file_path(
            original_root.path().canonicalize().expect("canon").join("helper.ds"),
        )
        .unwrap()
        .to_string();
        assert!(
            helper_prelude.contains(&format!("\"url\", {{ value: \"{expected_helper_url}\"")),
            "the imported module must report its own original path: {helper_prelude}"
        );
        assert!(
            helper_prelude.contains("\"main\", { value: false"),
            "{helper_prelude}"
        );
    }

    #[test]
    fn resolve_for_import_meta_matches_where_a_relative_import_loads_from() {
        let root = tempfile::tempdir().expect("temp project");
        let entry = root.path().join("main.js");
        let sibling = root.path().join("sibling.js");
        fs::write(&entry, "import {} from \"./sibling.js\";\n").expect("write entry");
        fs::write(&sibling, "export const s = 1;\n").expect("write sibling");
        let loader = PhpxEsmLoader::new(root.path().to_path_buf(), entry.clone(), None, None, None, false)
            .expect("loader");

        let referrer = loader.entry_specifier.to_string();
        let resolved = loader
            .resolve_for_import_meta("./sibling.js", &referrer)
            .expect("resolves");
        let expected = deno_core::ModuleSpecifier::from_file_path(
            sibling.canonicalize().unwrap_or(sibling.clone()),
        )
        .unwrap()
        .to_string();
        assert_eq!(
            resolved, expected,
            "import.meta.resolve must match where `import \"./sibling.js\"` actually loads from"
        );
    }

    #[test]
    fn resolve_for_import_meta_follows_the_source_override_too() {
        let original_root = tempfile::tempdir().expect("original root");
        let compiled_root = tempfile::tempdir().expect("compiled root");
        fs::write(original_root.path().join("main.ds"), "").expect("write original entry");
        fs::write(original_root.path().join("sibling.ds"), "export const s = 1;\n")
            .expect("write original sibling");
        fs::write(compiled_root.path().join("main.js"), "").expect("write compiled entry");
        fs::write(compiled_root.path().join("sibling.js"), "export const s = 1;\n")
            .expect("write compiled sibling");

        let compiled_entry = compiled_root.path().join("main.js");
        let mut loader = PhpxEsmLoader::new(
            compiled_root.path().to_path_buf(),
            compiled_entry.clone(),
            None,
            None,
            None,
            false,
        )
        .expect("loader");
        loader.set_source_override(
            compiled_root.path().to_path_buf(),
            original_root.path().to_path_buf(),
        );

        let referrer = loader.entry_specifier.to_string();
        let resolved = loader
            .resolve_for_import_meta("./sibling.js", &referrer)
            .expect("resolves");
        let expected = deno_core::ModuleSpecifier::from_file_path(
            original_root.path().canonicalize().expect("canon").join("sibling.ds"),
        )
        .unwrap()
        .to_string();
        assert_eq!(resolved, expected);
    }
}
