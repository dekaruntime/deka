use std::path::{Path, PathBuf};

pub fn detect_phpx_module_root_with<Exists, CurrentExe>(
    handler_path: &str,
    lock_exists: &Exists,
    current_exe: &CurrentExe,
) -> Option<PathBuf>
where
    Exists: Fn(&Path) -> bool,
    CurrentExe: Fn() -> Option<PathBuf>,
{
    let path = Path::new(handler_path);
    let start = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };

    let mut current = start;
    loop {
        let candidate = current.join("deka.lock");
        if lock_exists(&candidate) {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }

    // Fallback for ad-hoc execution outside a project: derive root from
    // binary path (.../target/release/cli -> repo root).
    if let Some(exe) = current_exe() {
        let resolved_exe = exe.canonicalize().unwrap_or(exe);
        if let Some(release_dir) = resolved_exe.parent()
            && let Some(target_dir) = release_dir.parent()
            && let Some(repo_root) = target_dir.parent()
        {
            let lock_path = repo_root.join("deka.lock");
            if lock_exists(&lock_path) {
                return Some(repo_root.to_path_buf());
            }
        }
    }

    None
}

pub fn ensure_phpx_module_root_env_with<Exists, CurrentExe, Get, Set>(
    handler_path: &str,
    lock_exists: &Exists,
    current_exe: &CurrentExe,
    env_get: &Get,
    env_set: &mut Set,
) where
    Exists: Fn(&Path) -> bool,
    CurrentExe: Fn() -> Option<PathBuf>,
    Get: Fn(&str) -> Option<String>,
    Set: FnMut(&str, &str),
{
    if env_get("PHPX_MODULE_ROOT").is_some() {
        return;
    }
    if let Some(root) = detect_phpx_module_root_with(handler_path, lock_exists, current_exe)
        && let Some(root_str) = root.to_str()
    {
        env_set("PHPX_MODULE_ROOT", root_str);
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_phpx_module_root_with, ensure_phpx_module_root_env_with};
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    fn lockset(paths: &[&str]) -> HashSet<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn detect_prefers_local_project_lock() {
        let locks = lockset(&["/repo/deka.lock", "/global/deka.lock"]);
        let exists = |path: &Path| locks.contains(path);
        let current_exe = || Some(PathBuf::from("/repo/target/release/cli"));
        let resolved = detect_phpx_module_root_with("/repo/app/main.phpx", &exists, &current_exe)
            .expect("resolve root");
        assert_eq!(resolved, PathBuf::from("/repo"));
    }

    #[test]
    fn detect_falls_back_to_exe_repo_root() {
        let locks = lockset(&["/repo/deka.lock"]);
        let exists = |path: &Path| locks.contains(path);
        let current_exe = || Some(PathBuf::from("/repo/target/release/cli"));
        let resolved =
            detect_phpx_module_root_with("/tmp/outside/main.phpx", &exists, &current_exe)
                .expect("resolve fallback root");
        assert_eq!(resolved, PathBuf::from("/repo"));
    }

    #[test]
    fn ensure_keeps_existing_phpx_module_root() {
        let locks = lockset(&["/repo/deka.lock"]);
        let exists = |path: &Path| locks.contains(path);
        let current_exe = || Some(PathBuf::from("/repo/target/release/cli"));
        let env_map = HashMap::from([("PHPX_MODULE_ROOT".to_string(), "/already".to_string())]);
        let env_get = |key: &str| env_map.get(key).cloned();
        let mut captured: Vec<(String, String)> = Vec::new();
        let mut env_set = |k: &str, v: &str| captured.push((k.to_string(), v.to_string()));
        ensure_phpx_module_root_env_with(
            "/tmp/outside/main.phpx",
            &exists,
            &current_exe,
            &env_get,
            &mut env_set,
        );
        assert!(captured.is_empty(), "existing env must not be overridden");
    }

    #[test]
    fn adwa_process_model_commands_use_same_runtime_resolution_path() {
        let locks = lockset(&["/repo/deka.lock"]);
        let exists = |path: &Path| locks.contains(path);
        let current_exe = || Some(PathBuf::from("/repo/target/release/cli"));
        for command in ["ls", "deka db"] {
            let env = Arc::new(Mutex::new(HashMap::<String, String>::new()));
            let env_get_store = Arc::clone(&env);
            let env_get = move |key: &str| {
                env_get_store
                    .lock()
                    .ok()
                    .and_then(|map| map.get(key).cloned())
            };
            let env_set_store = Arc::clone(&env);
            let mut env_set = |k: &str, v: &str| {
                if let Ok(mut map) = env_set_store.lock() {
                    map.insert(k.to_string(), v.to_string());
                }
            };
            let handler = format!("/tmp/adwa/{}.phpx", command.replace(' ', "_"));
            ensure_phpx_module_root_env_with(
                &handler,
                &exists,
                &current_exe,
                &env_get,
                &mut env_set,
            );
            assert_eq!(
                env.lock()
                    .ok()
                    .and_then(|map| map.get("PHPX_MODULE_ROOT").cloned()),
                Some("/repo".to_string()),
                "command '{}' should inherit runtime root resolver",
                command
            );
        }
    }
}
