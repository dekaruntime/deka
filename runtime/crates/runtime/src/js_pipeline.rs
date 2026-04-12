use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bundler::{bundle_virtual_entry, BundleOptions, VirtualSource};
use phpx_js::{
    build_stdlib_prelude, compile_phpx_source_to_js, parse_source_module_meta, SourceModuleMeta,
};
use runtime_core::module_spec::{is_bare_module_specifier, module_spec_aliases};

pub fn build_phpx_handler_bundle(handler_path: &str) -> Result<String, String> {
    let input_path = Path::new(handler_path);
    let input = input_path
        .to_str()
        .ok_or_else(|| format!("invalid utf-8 path: {}", input_path.display()))?;

    let source = fs::read_to_string(input_path)
        .map_err(|err| format!("failed to read {}: {}", input_path.display(), err))?;
    let meta = parse_source_module_meta(&source);

    let project_root = resolve_project_root(input_path)?;
    ensure_project_layout(&project_root, &meta)?;

    let mut entry_js = compile_phpx_source_to_js(&source, input, meta)?;

    // build_stdlib_prelude needs php_modules/stdlib.json under the project root.
    // Tenants may not have their own stdlib.json, so fall back to the system
    // stdlib path (same resolver used by ensure_project_layout).
    let prelude_root = if project_root.join("php_modules").join("stdlib.json").is_file() {
        project_root.clone()
    } else if let Some(stdlib_dir) = resolve_stdlib_path() {
        // The system stdlib lives at <runtime>/php_modules/; its parent is the
        // effective "project root" for prelude generation purposes.
        stdlib_dir.parent().unwrap_or(&stdlib_dir).to_path_buf()
    } else {
        project_root.clone() // let build_stdlib_prelude produce its normal error
    };
    let prelude = build_stdlib_prelude(&prelude_root)?;
    entry_js = format!("{prelude}\n{entry_js}");
    let entry_path = fs::canonicalize(input_path)
        .map_err(|err| format!("failed to resolve {}: {}", input_path.display(), err))?;

    let provider = Arc::new(PhpxBundleProvider::new(entry_path.clone(), entry_js));
    bundle_virtual_entry(
        &entry_path,
        BundleOptions {
            project_root,
            minify: true,
            iife: true,
            stdlib_path: None,
        },
        provider,
    )
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

        if path.extension().and_then(|ext| ext.to_str()) != Some("phpx") {
            return Ok(None);
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

    for dir in start.ancestors() {
        if dir.join("deka.json").is_file() {
            return Ok(dir.to_path_buf());
        }
    }

    Err(format!(
        "deka run requires a deka.json project root (searched from {})",
        input_path.display()
    ))
}

pub fn ensure_project_layout(project_root: &Path, meta: &SourceModuleMeta) -> Result<(), String> {
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
    let stdlib_dir = resolve_stdlib_path();

    if !modules_dir.is_dir() && stdlib_dir.is_none() {
        return Err(format!(
            "deka run requires php_modules/ at project root when using stdlib imports ({})",
            stdlib_imports.join(", ")
        ));
    }

    let mut missing = Vec::new();
    for spec in stdlib_imports {
        let found_local = modules_dir.is_dir() && resolve_module_file(&modules_dir, &spec).is_some();
        let found_stdlib = stdlib_dir.as_ref().map_or(false, |d| resolve_module_file(d, &spec).is_some());
        if !found_local && !found_stdlib {
            missing.push(spec);
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "missing stdlib modules under {}: {}",
            modules_dir.display(),
            missing.join(", ")
        ))
    }
}

/// Resolve the system stdlib php_modules/ path.
/// Checks DEKA_STDLIB_PATH env var first, then tries exe-relative path.
/// TODO: make configurable via CLI flag when deka supports it.
fn resolve_stdlib_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("DEKA_STDLIB_PATH") {
        let p = PathBuf::from(path);
        if p.is_dir() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let runtime_dir = exe.parent()?.parent()?.parent()?;
    let candidate = runtime_dir.join("php_modules");
    if candidate.is_dir() { Some(candidate) } else { None }
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
    let mut candidates = Vec::new();
    for alias in module_spec_aliases(spec) {
        candidates.push(modules_dir.join(format!("{}.phpx", alias)));
        candidates.push(modules_dir.join(format!("{}.php", alias)));
        candidates.push(modules_dir.join(alias.as_str()).join("index.phpx"));
        candidates.push(modules_dir.join(alias.as_str()).join("index.php"));
        if alias.ends_with(".phpx") || alias.ends_with(".php") {
            candidates.push(modules_dir.join(alias));
        }
    }

    candidates.into_iter().find(|path| path.is_file())
}

fn is_bare_specifier(spec: &str) -> bool {
    is_bare_module_specifier(spec)
}
