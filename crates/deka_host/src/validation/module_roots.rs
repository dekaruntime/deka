//! Whether a `ds_modules` directory counts as a usable module-resolution
//! root -- as opposed to merely existing on disk.
//!
//! Split out of `validation::modules` (deka#1065) because it is a
//! self-contained predicate, not validation logic, and `modules.rs` is
//! already well over the repo's file-size cap (CLAUDE.md: >1000 lines is
//! "wrong", >2000 is "a fire to put out") -- new code must not grow it
//! further.

use std::path::{Path, PathBuf};

use deka_modules::modules::existing_modules_dirs;

/// A `ds_modules` directory holding nothing but the compiler's own scratch
/// cache (`ds_modules/.cache/{dev,prod}` -- see
/// `runtime_core::dist::compiler_cache_dir_with`) must never register as a
/// module-resolution root purely because the compiler wrote there: doing
/// so would silently change module-root and security-policy resolution for
/// a project that never installed anything.
///
/// An EMPTY `ds_modules` still counts as usable -- that predates this fix
/// and is a legitimate state (a module root deka.lock was set up for, with
/// nothing installed into it yet; `validation::modules::resolve_modules_root_with`'s
/// own tests fix this exact directory shape). What must NOT count is a
/// `ds_modules` whose entire on-disk content is the compiler's `.cache`
/// scratch space.
pub(crate) fn has_installed_modules(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut saw_any_entry = false;
    let mut saw_non_cache_entry = false;
    for entry in entries.filter_map(|entry| entry.ok()) {
        saw_any_entry = true;
        if entry.file_name() != ".cache" {
            saw_non_cache_entry = true;
        }
    }
    !saw_any_entry || saw_non_cache_entry
}

/// Like `deka_modules::modules::existing_modules_dirs`, but filters out a
/// `ds_modules` that exists on disk purely as the compiler's cache home
/// (deka#1065). Every module-root / security-policy resolution call site
/// in `validation::modules` goes through this instead of the raw upstream
/// helper so "ds_modules exists" and "ds_modules is a usable module root"
/// can never drift back apart at a single call site.
pub(crate) fn existing_populated_modules_dirs(project: &Path) -> Vec<PathBuf> {
    existing_modules_dirs(project)
        .into_iter()
        .filter(|dir| has_installed_modules(dir))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{existing_populated_modules_dirs, has_installed_modules};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("deka_module_roots_test_{name}_{nanos}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn empty_directory_still_counts_as_installed() {
        let dir = temp_dir("empty");
        assert!(
            has_installed_modules(&dir),
            "an empty ds_modules is a legitimate pre-install state and must still resolve"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cache_only_directory_does_not_count_as_installed() {
        let dir = temp_dir("cache_only");
        fs::create_dir_all(dir.join(".cache").join("dev")).expect("create cache dir");
        assert!(
            !has_installed_modules(&dir),
            "a ds_modules holding nothing but the compiler cache must not count as installed"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn real_package_alongside_cache_counts_as_installed() {
        let dir = temp_dir("cache_plus_package");
        fs::create_dir_all(dir.join(".cache").join("prod")).expect("create cache dir");
        fs::create_dir_all(dir.join("@deka").join("crypto")).expect("create package dir");
        assert!(
            has_installed_modules(&dir),
            "a real package alongside the compiler cache must count as installed"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_directory_does_not_count_as_installed() {
        let dir = std::env::temp_dir().join("deka_module_roots_test_does_not_exist_at_all");
        let _ = fs::remove_dir_all(&dir);
        assert!(!has_installed_modules(&dir));
    }

    /// deka#1065: `existing_populated_modules_dirs` is what
    /// `validation::modules::resolve_modules_root_with` actually calls at
    /// every resolution site -- proves the filtering, not just the
    /// standalone predicate.
    #[test]
    fn existing_populated_modules_dirs_filters_out_cache_only_ds_modules() {
        let project = temp_dir("populated_filter");
        let ds_modules = project.join("ds_modules");
        fs::create_dir_all(ds_modules.join(".cache").join("dev")).expect("create cache dir");

        assert!(
            existing_populated_modules_dirs(&project).is_empty(),
            "a cache-only ds_modules must not be returned as a populated modules dir"
        );

        fs::create_dir_all(ds_modules.join("@deka").join("crypto")).expect("create package dir");
        assert_eq!(
            existing_populated_modules_dirs(&project),
            vec![ds_modules],
            "a ds_modules with real package content must resolve once installed"
        );

        let _ = fs::remove_dir_all(project);
    }

    /// deka#1065: a `ds_modules` that exists on disk purely as the
    /// compiler's cache home must not resolve as a usable module root through
    /// the real entry point (`validation::modules::resolve_modules_root_with`)
    /// -- resolution must fall through a cache-only local `ds_modules` to a
    /// populated fallback root instead of stopping there. Builds its own
    /// minimal fixtures rather than reusing `modules.rs`'s private test
    /// helpers, since this module is a sibling, not a child, of `modules`.
    #[test]
    fn cache_only_ds_modules_does_not_count_as_a_usable_module_root() {
        use crate::validation::modules::resolve_modules_root_with;

        let local = temp_dir("resolve_cache_only_local");
        fs::write(local.join("deka.lock"), "{}").expect("write local lockfile");
        // Simulate the compiler cache (deka#1065) as the ONLY content of
        // ds_modules -- no real packages installed.
        fs::create_dir_all(local.join("ds_modules").join(".cache").join("dev"))
            .expect("create compiler cache dir");
        let entry = local.join("app").join("main.phpx");
        fs::create_dir_all(entry.parent().expect("entry parent")).expect("mkdir app");
        fs::write(&entry, "import { foo } from 'a'
").expect("write entry");

        let global = temp_dir("resolve_cache_only_global");
        fs::write(global.join("deka.lock"), "{}").expect("write global lockfile");
        fs::create_dir_all(global.join("ds_modules").join("@deka").join("crypto"))
            .expect("create real package dir");

        // `local`'s ds_modules holds only the compiler cache, so resolution
        // must fall through to the populated global root instead of
        // stopping at the cache-only local one.
        let resolved = resolve_modules_root_with(
            entry.to_string_lossy().as_ref(),
            Some(global.to_string_lossy().as_ref()),
        );
        assert_eq!(
            resolved,
            Some(global.join("ds_modules")),
            "a cache-only ds_modules must not win module-root resolution over a populated one"
        );

        let _ = fs::remove_dir_all(local);
        let _ = fs::remove_dir_all(global);
    }
}
