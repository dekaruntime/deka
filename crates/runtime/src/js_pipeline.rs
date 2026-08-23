#[cfg(test)]
use std::cell::Cell;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use bundler::{BundleOptions, VirtualSource, bundle_virtual_entry};
use deka_js::{
    SourceModuleMeta, compile_phpx_source_to_js, parse_source_module_meta,
};
use runtime_core::module_spec::{is_bare_module_specifier, module_spec_aliases};

pub fn build_deka_handler_bundle(handler_path: &str) -> Result<String, String> {
    let input_path = Path::new(handler_path);
    let input = input_path
        .to_str()
        .ok_or_else(|| format!("invalid utf-8 path: {}", input_path.display()))?;

    let source = fs::read_to_string(input_path)
        .map_err(|err| format!("failed to read {}: {}", input_path.display(), err))?;
    let meta = parse_source_module_meta(&source);

    let project_root = resolve_project_root(input_path)?;
    // `DEKA_MODULE_ROOT` is process-global because it is also consumed by the
    // module validator. A platform process has many project roots, however: the
    // store root and every tenant are separate projects. Keep the global
    // platform default from selecting the wrong lockfile while this handler
    // (and its virtual imports) are compiled.
    with_project_module_root(&project_root, || {
        build_deka_handler_bundle_in_project(input_path, input, source, meta, project_root.clone())
    })
}

fn build_deka_handler_bundle_in_project(
    input_path: &Path,
    input: &str,
    source: String,
    meta: SourceModuleMeta,
    project_root: PathBuf,
) -> Result<String, String> {
    ensure_project_layout(&project_root, &meta)?;

    let mut entry_js = compile_phpx_source_to_js(&source, input, meta)?;

    let prelude = String::new();

    // Inject the tenant root so that globalThis.__dekaFs (installed by
    // php/php.js at extension-init time) can enforce per-tenant path
    // confinement.  The root is the canonicalised project_root — i.e. the
    // directory containing deka.json for this tenant.
    let canonical_root = fs::canonicalize(&project_root).unwrap_or_else(|_| project_root.clone());
    let root_json = serde_json::to_string(&canonical_root.to_string_lossy().to_string())
        .unwrap_or_else(|_| "\"\"".to_string());
    let tenant_root_injection = format!("globalThis.__dekaFsTenantRoot = {};\n", root_json);

    entry_js = format!("{prelude}\n{tenant_root_injection}{entry_js}");
    let entry_path = fs::canonicalize(input_path)
        .map_err(|err| format!("failed to resolve {}: {}", input_path.display(), err))?;

    let provider = Arc::new(PhpxBundleProvider::new(entry_path.clone(), entry_js));
    bundle_virtual_entry(
        &entry_path,
        BundleOptions {
            project_root,
            minify: true,
            iife: true,
        },
        provider,
    )
}

/// Serializes the short period in which the process-global module-root
/// setting is pointed at one handler's project. This includes virtual module
/// compilation performed by the bundler, so package integrity is checked
/// against the same nearest `deka.lock` that `deka build <handler>` uses.
fn with_project_module_root<T>(
    project_root: &Path,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    static MODULE_ROOT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = MODULE_ROOT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let _module_root = ModuleRootRestoreGuard::replace(project_root);
    action()
}

/// Restores the process-global module root when the tenant bundling scope
/// exits, including when compilation or the bundler unwinds through a panic.
struct ModuleRootRestoreGuard {
    previous: Option<OsString>,
}

impl ModuleRootRestoreGuard {
    fn replace(module_root: &Path) -> Self {
        let previous = std::env::var_os("DEKA_MODULE_ROOT");
        unsafe {
            std::env::set_var("DEKA_MODULE_ROOT", module_root);
        }
        Self { previous }
    }
}

impl Drop for ModuleRootRestoreGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var("DEKA_MODULE_ROOT", value),
                None => std::env::remove_var("DEKA_MODULE_ROOT"),
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    static PANIC_DURING_VIRTUAL_LOAD: Cell<bool> = const { Cell::new(false) };
}

struct PhpxBundleProvider {
    entry_path: PathBuf,
    entry_source: String,
}

impl PhpxBundleProvider {
    fn new(entry_path: PathBuf, entry_source: String) -> Self {
        Self {
            entry_path,
            entry_source,
        }
    }
}

impl VirtualSource for PhpxBundleProvider {
    fn load_virtual(&self, path: &Path) -> Result<Option<String>, String> {
        if path == self.entry_path {
            return Ok(Some(self.entry_source.clone()));
        }

        if path.extension().and_then(|ext| ext.to_str()) != Some("ds") {
            return Ok(None);
        }

        #[cfg(test)]
        if PANIC_DURING_VIRTUAL_LOAD.replace(false) {
            panic!("test-only panic during virtual module bundling");
        }

        let input = path
            .to_str()
            .ok_or_else(|| format!("invalid utf-8 path: {}", path.display()))?;
        let source =
            fs::read_to_string(path).map_err(|err| format!("failed to read {}: {}", input, err))?;
        let meta = parse_source_module_meta(&source);
        let js = compile_phpx_source_to_js(&source, input, meta)?;
        Ok(Some(js))
    }
}

pub fn resolve_project_root(input_path: &Path) -> Result<PathBuf, String> {
    let start = if input_path.is_dir() {
        input_path.to_path_buf()
    } else {
        input_path.parent().unwrap_or(Path::new(".")).to_path_buf()
    };

    let mut nearest_manifest_root = None;
    for dir in start.ancestors() {
        if dir.join("deka.json").is_file() {
            let dir = dir.to_path_buf();
            if dir.join("deka.lock").is_file() {
                return Ok(dir);
            }
            if nearest_manifest_root.is_none() {
                nearest_manifest_root = Some(dir);
            }
        }
    }

    if let Some(root) = nearest_manifest_root {
        return Ok(root);
    }

    Err(format!(
        "deka run requires a deka.json project root (searched from {})",
        input_path.display()
    ))
}

pub fn ensure_project_layout(project_root: &Path, meta: &SourceModuleMeta) -> Result<(), String> {
    // DEKA_MODULE_ROOT bypass (#220): when set, the tenant relies on the runtime stdlib at
    // that root and we trust the runtime-provided modules without requiring a local
    // deka.lock or php_modules/. Tenant-local packages would still need a lockfile, but
    // stdlib-only tenants (id.tana.gg) deploy without ceremony.
    if std::env::var_os("DEKA_MODULE_ROOT").is_some() {
        return Ok(());
    }

    let lock_path = project_root.join("deka.lock");
    if !lock_path.is_file() {
        return Err(format!(
            "deka run requires deka.lock at project root: {}",
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
            "deka run requires php_modules/ at project root when using stdlib imports ({}). Run `deka install`.",
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
    let mut seen = BTreeSet::new();
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

    if let Some(rest) = spec.strip_prefix("@deka/") {
        return is_stdlib_module_spec(rest);
    }

    spec.starts_with("component/")
        || spec.starts_with("deka/")
        || spec.starts_with("encoding/")
        || spec.starts_with("db/")
        || matches!(
            spec,
            "json"
                | "postgres"
                | "mysql"
                | "sqlite"
                | "bytes"
                | "buffer"
                | "http"
                | "tcp"
                | "tls"
                | "fs"
                | "crypto"
                | "jwt"
                | "cookies"
                | "auth"
                | "db"
                | "time"
        )
}

fn resolve_module_file(modules_dir: &Path, spec: &str) -> Option<PathBuf> {
    // For prefixed stdlib specifiers (e.g. encoding/json) also try the scoped
    // @deka layout so `deka install`-ed modules are found.
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
        candidates.push(modules_dir.join(format!("{}.ds", alias)));
        candidates.push(modules_dir.join(alias.as_str()).join("index.ds"));
        if alias.ends_with(".ds") {
            candidates.push(modules_dir.join(alias));
        }
    }

    candidates.into_iter().find(|path| path.is_file())
}

fn is_bare_specifier(spec: &str) -> bool {
    is_bare_module_specifier(spec)
}

#[cfg(test)]
mod tests {
    use super::{
        PANIC_DURING_VIRTUAL_LOAD, build_deka_handler_bundle, ensure_project_layout,
        resolve_project_root,
    };
    use modules_php::integrity::compute_package_integrity;
    use deka_js::parse_source_module_meta;
    use std::path::Path;
    use std::sync::Mutex;

    // DEKA_MODULE_ROOT is process-global. Keep tests that replace it isolated
    // from each other while preserving the runtime's concurrent bundle tests.
    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn write_locked_package(root: &Path, name: &str, package_path: &str) {
        let integrity = compute_package_integrity(&root.join("php_modules").join(package_path))
            .expect("package integrity");
        let lock_path = root.join("deka.lock");
        let mut lock: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&lock_path).expect("read lock"))
                .expect("parse lock");
        lock["php"]["packages"][name] = serde_json::json!([
            "1.0.0",
            format!("linkhash:{name}"),
            {
                "moduleGraph": { "hash": integrity.module_graph },
                "fsGraph": { "hash": integrity.fs_graph }
            },
            ""
        ]);
        std::fs::write(
            lock_path,
            serde_json::to_string(&lock).expect("serialize lock"),
        )
        .expect("write lock");
    }

    #[test]
    fn project_root_prefers_outer_lock_bearing_root_over_nested_package_manifest() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("deka.json"), "{}").expect("root deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("root deka.lock");

        let package_dir = tmp
            .path()
            .join("php_modules")
            .join("@deka")
            .join("payments");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        std::fs::write(package_dir.join("deka.json"), "{}").expect("package deka.json");
        let input = package_dir.join("index.ds");
        std::fs::write(&input, "<div />").expect("input");

        let root = resolve_project_root(&input).expect("project root");
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn project_root_falls_back_to_nearest_manifest_when_no_lock_exists() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("deka.json"), "{}").expect("root deka.json");

        let package_dir = tmp
            .path()
            .join("php_modules")
            .join("@deka")
            .join("payments");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        std::fs::write(package_dir.join("deka.json"), "{}").expect("package deka.json");
        let input = package_dir.join("index.ds");
        std::fs::write(&input, "<div />").expect("input");

        let root = resolve_project_root(&input).expect("project root");
        assert_eq!(root, package_dir);
    }

    #[test]
    fn ensure_project_layout_accepts_scoped_stdlib_imports_installed_unscoped() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("deka.json"), "{}").expect("deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("deka.lock");
        for module in ["http", "crypto", "time"] {
            let dir = tmp.path().join("php_modules").join(module);
            std::fs::create_dir_all(&dir).expect("module dir");
            std::fs::write(
                dir.join("index.ds"),
                "export fn marker() { return true }\n",
            )
            .expect("module index");
        }

        let source = "\
import { http_get } from '@deka/http'
import { random_hex } from '@deka/crypto'
import { now_ms } from '@deka/time'
";
        let meta = parse_source_module_meta(source);
        assert_eq!(
            super::collect_stdlib_imports(&meta),
            vec!["@deka/crypto", "@deka/http", "@deka/time"]
        );

        ensure_project_layout(tmp.path(), &meta).expect("layout should pass");
    }

    #[test]
    fn bundle_uses_tenant_lock_when_platform_lock_is_empty() {
        let _env_lock = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().expect("tmp");
        let platform_root = tmp.path();
        let tenant_root = platform_root.join("default");
        let modules = tenant_root.join("php_modules");

        std::fs::write(platform_root.join("deka.json"), "{}").expect("platform manifest");
        std::fs::write(
            platform_root.join("deka.lock"),
            r#"{"lockfileVersion":1,"php":{"packages":{}}}"#,
        )
        .expect("empty platform lock");
        std::fs::create_dir_all(modules.join("@tana/store")).expect("store module dir");
        std::fs::create_dir_all(modules.join("@deka/crypto")).expect("crypto module dir");
        std::fs::write(tenant_root.join("deka.json"), "{}").expect("tenant manifest");
        std::fs::write(
            tenant_root.join("deka.lock"),
            r#"{"lockfileVersion":1,"php":{"packages":{}}}"#,
        )
        .expect("tenant lock");
        std::fs::write(
            modules.join("@tana/store/index.ds"),
            "import { random_hex } from '@deka/crypto'\nexport fn createStore(): string { return random_hex(); }\n",
        )
        .expect("store module");
        std::fs::write(
            modules.join("@deka/crypto/index.ds"),
            "export fn random_hex(): string { return 'abc'; }\n",
        )
        .expect("crypto module");
        write_locked_package(&tenant_root, "@deka/crypto", "@deka/crypto");
        write_locked_package(&tenant_root, "@tana/store", "@tana/store");
        let handler = tenant_root.join("main.ds");
        std::fs::write(
            &handler,
            "import { createStore } from '@tana/store'\nexport fn App(): string { return createStore(); }\n",
        )
        .expect("handler");

        // The platform process starts with its root selected globally. The
        // bundle must instead use default/deka.lock for @tana/store and its
        // transitive @deka/crypto import.
        let previous_root = std::env::var_os("DEKA_MODULE_ROOT");
        unsafe { std::env::set_var("DEKA_MODULE_ROOT", platform_root) };
        let bundle = build_deka_handler_bundle(handler.to_str().expect("utf-8 handler"));
        assert!(bundle.is_ok(), "tenant bundle failed: {bundle:?}");
        assert_eq!(
            std::env::var_os("DEKA_MODULE_ROOT").as_deref(),
            Some(platform_root.as_os_str()),
            "tenant bundle must restore the platform module root"
        );
        unsafe {
            match previous_root {
                Some(value) => std::env::set_var("DEKA_MODULE_ROOT", value),
                None => std::env::remove_var("DEKA_MODULE_ROOT"),
            }
        }
    }

    #[test]
    fn bundle_panic_restores_module_root_before_next_tenant_bundle() {
        let _env_lock = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().expect("tmp");
        let platform_root = tmp.path().join("platform");
        let tenant_a_root = tmp.path().join("tenant-a");
        let tenant_b_root = tmp.path().join("tenant-b");
        std::fs::create_dir_all(&platform_root).expect("platform root");
        std::fs::create_dir_all(&tenant_a_root).expect("tenant A root");
        std::fs::create_dir_all(&tenant_b_root).expect("tenant B root");

        for root in [&tenant_a_root, &tenant_b_root] {
            std::fs::write(root.join("deka.json"), "{}").expect("tenant manifest");
        }

        let tenant_a_handler = tenant_a_root.join("main.ds");
        std::fs::write(
            &tenant_a_handler,
            "import { marker } from './dependency.ds'\nexport fn App(): string { return marker(); }\n",
        )
        .expect("tenant A handler");
        std::fs::write(
            tenant_a_root.join("dependency.ds"),
            "export fn marker(): string { return 'a'; }\n",
        )
        .expect("tenant A dependency");

        let tenant_b_handler = tenant_b_root.join("main.ds");
        std::fs::write(
            &tenant_b_handler,
            "export fn App(): string { return 'b'; }\n",
        )
        .expect("tenant B handler");

        let previous_root = std::env::var_os("DEKA_MODULE_ROOT");
        unsafe { std::env::set_var("DEKA_MODULE_ROOT", &platform_root) };

        PANIC_DURING_VIRTUAL_LOAD.with(|panic_once| panic_once.set(true));
        let panic = std::panic::catch_unwind(|| {
            build_deka_handler_bundle(tenant_a_handler.to_str().expect("utf-8 handler"))
        });
        assert!(panic.is_err(), "tenant A bundle should panic mid-bundle");
        assert_eq!(
            std::env::var_os("DEKA_MODULE_ROOT").as_deref(),
            Some(platform_root.as_os_str()),
            "a panicking tenant bundle must restore the platform module root"
        );

        let tenant_b_bundle =
            build_deka_handler_bundle(tenant_b_handler.to_str().expect("utf-8 handler"));
        assert!(
            tenant_b_bundle.is_ok(),
            "tenant B must still bundle after tenant A unwinds: {tenant_b_bundle:?}"
        );
        assert_eq!(
            std::env::var_os("DEKA_MODULE_ROOT").as_deref(),
            Some(platform_root.as_os_str()),
            "tenant B bundle must not inherit tenant A's module root"
        );

        unsafe {
            match previous_root {
                Some(value) => std::env::set_var("DEKA_MODULE_ROOT", value),
                None => std::env::remove_var("DEKA_MODULE_ROOT"),
            }
        }
    }
}
