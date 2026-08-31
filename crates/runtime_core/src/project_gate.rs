//! One gate for "is this project valid to run", shared by every entry point.
//!
//! This replaces three separate `ensure_project_layout` implementations — in
//! `pool::esm_loader`, `cli::build` and `runtime::js_pipeline` — that had the
//! same name and job but three different bypass rules. A fix applied to one was
//! invisible to the others, which is how the declared-dependency check ended up
//! missing from all three (deka#403, deka#414).
//!
//! Bypasses are explicit parameters here. The `esm_loader` copy keyed its bypass
//! off a `DEKA_MODULE_ROOT` environment variable read in the middle of the
//! function, so one ambient variable disabled every check including the
//! lockfile requirement (deka#229).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// How strict to be, stated by the caller rather than inferred from ambient state.
#[derive(Debug, Clone)]
pub struct GateOptions {
    /// An external module root supplying the stdlib. When set to something other
    /// than the project root, the runtime provides these modules and the local
    /// `ds_modules/` tree is not required.
    pub module_root: Option<PathBuf>,
    /// Whether `deka.lock` must be present. Stdlib-only tenants deploy without
    /// one (deka#220); everything else requires it.
    pub require_lockfile: bool,
}

impl Default for GateOptions {
    fn default() -> Self {
        Self {
            module_root: None,
            require_lockfile: true,
        }
    }
}

/// Validate a project against the imports its module graph actually contains.
///
/// Rules, in order:
///   1. `deka.lock` exists, unless the caller waived it
///   2. every stdlib import is **declared** in `deka.json` dependencies
///   3. every stdlib import resolves to a file under the modules directory
///
/// Rule 2 is the one that did not exist. Resolution was satisfied by a directory
/// happening to be present, so a package could import something it never
/// declared and install fine for whoever happened to have it.
pub fn validate_project(
    project_root: &Path,
    imports: &[String],
    opts: &GateOptions,
    is_stdlib_spec: &dyn Fn(&str) -> bool,
    modules_dir_for: &dyn Fn(&Path) -> PathBuf,
    resolves: &dyn Fn(&Path, &str) -> bool,
) -> Result<(), String> {
    // An external module root means the runtime supplies the stdlib; the local
    // tree is not expected to contain it.
    if opts
        .module_root
        .as_deref()
        .is_some_and(|root| root != project_root)
    {
        return Ok(());
    }

    if opts.require_lockfile {
        let lock_path = project_root.join("deka.lock");
        if !lock_path.is_file() {
            return Err(format!(
                "deka requires deka.lock at project root: {}",
                lock_path.display()
            ));
        }
    }

    let stdlib_imports: BTreeSet<String> = imports
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| is_stdlib_spec(s))
        .collect();

    if stdlib_imports.is_empty() {
        return Ok(());
    }

    // Rule 2 — declared in deka.json.
    let declared = declared_dependencies(project_root);
    let undeclared: Vec<&String> = stdlib_imports
        .iter()
        .filter(|spec| !is_declared(spec, &declared))
        .collect();

    if !undeclared.is_empty() {
        let list: Vec<String> = undeclared.iter().map(|s| (*s).clone()).collect();
        let add: Vec<String> = list.iter().map(|s| bare_name(s).to_string()).collect();
        return Err(format!(
            "imported but not declared in deka.json: {}. Add {} with `deka add {}`.",
            list.join(", "),
            if list.len() == 1 { "it" } else { "them" },
            add.join(" ")
        ));
    }

    // Rule 3 — present on disk.
    let modules_dir = modules_dir_for(project_root);
    if !modules_dir.is_dir() {
        return Err(format!(
            "deka requires {} when using stdlib imports ({}). Run `deka install`.",
            modules_dir.display(),
            stdlib_imports.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }

    let missing: Vec<String> = stdlib_imports
        .iter()
        .filter(|spec| !resolves(&modules_dir, spec))
        .cloned()
        .collect();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "declared but not installed under {}: {}. Run `deka install`.",
            modules_dir.display(),
            missing.join(", ")
        ))
    }
}

/// Dependency names from `deka.json`, normalised to their bare form so
/// `@deka/crypto` and `crypto` compare equal.
fn declared_dependencies(project_root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(raw) = std::fs::read_to_string(project_root.join("deka.json")) else {
        return out;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return out;
    };
    for key in ["dependencies", "devDependencies"] {
        if let Some(map) = json.get(key).and_then(|v| v.as_object()) {
            for name in map.keys() {
                out.insert(bare_name(name).to_string());
            }
        }
    }
    out
}

/// `@deka/crypto` and `crypto` are the same package; `component/foo` keeps its
/// first segment, which is what a manifest would name.
fn bare_name(spec: &str) -> &str {
    let spec = spec.trim();
    if let Some(rest) = spec.strip_prefix("@deka/") {
        return rest.split('/').next().unwrap_or(rest);
    }
    spec.split('/').next().unwrap_or(spec)
}

fn is_declared(spec: &str, declared: &BTreeSet<String>) -> bool {
    declared.contains(bare_name(spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdlib(spec: &str) -> bool {
        matches!(spec, "crypto" | "io" | "bytes") || spec.starts_with("@deka/")
    }

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn setup(manifest: &str) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "deka.json", manifest);
        write(tmp.path(), "deka.lock", r#"{"lockfileVersion":1,"packages":{}}"#);
        std::fs::create_dir_all(tmp.path().join("ds_modules/@deka/crypto")).unwrap();
        tmp
    }

    fn run(root: &Path, imports: &[&str]) -> Result<(), String> {
        let owned: Vec<String> = imports.iter().map(|s| s.to_string()).collect();
        validate_project(
            root,
            &owned,
            &GateOptions::default(),
            &stdlib,
            &|r| r.join("ds_modules"),
            &|dir, spec| dir.join("@deka").join(super::bare_name(spec)).is_dir(),
        )
    }

    #[test]
    fn undeclared_import_is_rejected() {
        let tmp = setup(r#"{"name":"t","dependencies":{"@deka/crypto":"0.2.0"}}"#);
        let err = run(tmp.path(), &["crypto", "io"]).unwrap_err();
        assert!(err.contains("not declared"), "{err}");
        assert!(err.contains("io"), "{err}");
    }

    #[test]
    fn declared_and_installed_passes() {
        let tmp = setup(r#"{"name":"t","dependencies":{"@deka/crypto":"0.2.0"}}"#);
        run(tmp.path(), &["crypto"]).expect("should pass");
    }

    #[test]
    fn scoped_and_bare_names_are_the_same_package() {
        let tmp = setup(r#"{"name":"t","dependencies":{"crypto":"0.2.0"}}"#);
        run(tmp.path(), &["@deka/crypto"]).expect("bare declaration covers scoped import");
    }

    #[test]
    fn declared_but_not_installed_says_install() {
        let tmp = setup(r#"{"name":"t","dependencies":{"@deka/bytes":"0.2.1"}}"#);
        let err = run(tmp.path(), &["bytes"]).unwrap_err();
        assert!(err.contains("not installed"), "{err}");
    }

    #[test]
    fn missing_lockfile_is_rejected_but_waivable() {
        let tmp = setup(r#"{"name":"t","dependencies":{"@deka/crypto":"0.2.0"}}"#);
        std::fs::remove_file(tmp.path().join("deka.lock")).unwrap();
        assert!(run(tmp.path(), &["crypto"]).is_err());

        let opts = GateOptions {
            require_lockfile: false,
            ..GateOptions::default()
        };
        let owned = vec!["crypto".to_string()];
        validate_project(
            tmp.path(),
            &owned,
            &opts,
            &stdlib,
            &|r| r.join("ds_modules"),
            &|dir, spec| dir.join("@deka").join(super::bare_name(spec)).is_dir(),
        )
        .expect("waived lockfile should pass");
    }

    #[test]
    fn external_module_root_skips_local_checks() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "deka.json", r#"{"name":"t"}"#);
        let opts = GateOptions {
            module_root: Some(PathBuf::from("/elsewhere")),
            ..GateOptions::default()
        };
        let owned = vec!["crypto".to_string()];
        validate_project(
            tmp.path(),
            &owned,
            &opts,
            &stdlib,
            &|r| r.join("ds_modules"),
            &|_, _| false,
        )
        .expect("external module root supplies the stdlib");
    }
}
