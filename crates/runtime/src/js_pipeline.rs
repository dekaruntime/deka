use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(test)]
use runtime_core::modules::MODULES_DIR;

fn parse_module_imports(source: &str) -> Vec<String> {
    runtime_core::ds_imports::paths(source)
}

pub fn build_deka_handler_bundle(handler_path: &str) -> Result<String, String> {
    let input_path = Path::new(handler_path);

    let source = fs::read_to_string(input_path)
        .map_err(|err| format!("failed to read {}: {}", input_path.display(), err))?;

    let project_root = resolve_project_root(input_path)?;
    build_deka_handler_bundle_in_project(input_path, source, project_root)
}

fn build_deka_handler_bundle_in_project(
    input_path: &Path,
    source: String,
    project_root: PathBuf,
) -> Result<String, String> {
    let imports = parse_module_imports(&source);
    ensure_project_layout(&project_root, Some(&project_root), &imports)?;

    let canonical_root = fs::canonicalize(&project_root).unwrap_or_else(|_| project_root.clone());
    let root_json = serde_json::to_string(&canonical_root.to_string_lossy().to_string())
        .unwrap_or_else(|_| "\"\"".to_string());
    let tenant_root_injection = format!("globalThis.__dekaFsTenantRoot = {};\n", root_json);

    let dsc = runtime_core::dsc::find_dsc()?.ok_or_else(|| {
        "dsc is required to bundle DekaScript handlers. Set DEKA_DSC, install dsc next to deka, or put dsc on PATH.".to_string()
    })?;
    // dsc refuses to overwrite a file it did not generate. tempfile()
    // creates an empty file, which trips that gate.
    let tmp = tempfile::tempdir().map_err(|err| format!("failed to create bundle temp dir: {err}"))?;
    let out = tmp.path().join("bundle.js");
    let entry = input_path
        .to_str()
        .ok_or_else(|| "handler path is not UTF-8".to_string())?;
    let out_str = out
        .to_str()
        .ok_or_else(|| "bundle temp path is not UTF-8".to_string())?;
    let output = Command::new(&dsc)
        .current_dir(&project_root)
        .env("DEKA_MODULE_ROOT", &project_root)
        .args(["transpile", entry, "--bundle", "--treeshake", "--out", out_str])
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let js = fs::read_to_string(&out)
        .map_err(|err| format!("failed to read {}: {err}", out.display()))?;
    Ok(format!("{tenant_root_injection}{js}"))
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

pub fn ensure_project_layout(
    project_root: &Path,
    module_root: Option<&Path>,
    imports: &[String],
) -> Result<(), String> {
    runtime_core::project_gate::validate_project(
        project_root,
        imports,
        &runtime_core::project_gate::GateOptions {
            module_root: module_root.map(|p| p.to_path_buf()),
            require_lockfile: true,
            context: "deka run",
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{
        build_deka_handler_bundle, ensure_project_layout, resolve_project_root, MODULES_DIR,
    };
    use deka_host::integrity::compute_package_integrity;
    use std::path::Path;
    use std::sync::Mutex;

    // DEKA_MODULE_ROOT is process-global. Keep tests that replace it isolated
    // from each other while preserving the runtime's concurrent bundle tests.
    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn write_locked_package(root: &Path, name: &str, package_path: &str) {
        let integrity = compute_package_integrity(&root.join(MODULES_DIR).join(package_path))
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
            .join(MODULES_DIR)
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
            .join(MODULES_DIR)
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
        // Dependencies must be declared, not merely installed (deka#403).
        std::fs::write(
            tmp.path().join("deka.json"),
            r#"{"dependencies":{"@deka/http":"*","@deka/crypto":"*","@deka/time":"*"}}"#,
        )
        .expect("deka.json");
        std::fs::write(tmp.path().join("deka.lock"), "{}").expect("deka.lock");
        for module in ["http", "crypto", "time"] {
            let dir = tmp.path().join(MODULES_DIR).join(module);
            std::fs::create_dir_all(&dir).expect("module dir");
            std::fs::write(
                dir.join("index.ds"),
                "export fn marker() { return true }\n",
            )
            .expect("module index");
        }

        let imports = vec![
            "@deka/http".to_string(),
            "@deka/crypto".to_string(),
            "@deka/time".to_string(),
        ];
        assert!(
            imports
                .iter()
                .all(|spec| runtime_core::project_gate::is_stdlib_module_spec(spec)),
            "scoped stdlib specifiers must be gated"
        );

        ensure_project_layout(tmp.path(), Some(tmp.path()), &imports).expect("layout should pass");
    }

    #[test]
    #[ignore = "fixture ds_modules/@tana/store/index.ds uses pre-v2 syntax; revisit during stdlib fixture cleanup (see dekaruntime/deka#330)"]
    fn bundle_uses_tenant_lock_when_platform_lock_is_empty() {
        let _env_lock = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tmp = tempfile::tempdir().expect("tmp");
        let platform_root = tmp.path();
        let tenant_root = platform_root.join("default");
        let modules = tenant_root.join(MODULES_DIR);

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
            "import { random_hex } from '@deka/crypto'\nexport fn createStore() string { return random_hex(); }\n",
        )
        .expect("store module");
        std::fs::write(
            modules.join("@deka/crypto/index.ds"),
            "export fn random_hex() string { return 'abc'; }\n",
        )
        .expect("crypto module");
        write_locked_package(&tenant_root, "@deka/crypto", "@deka/crypto");
        write_locked_package(&tenant_root, "@tana/store", "@tana/store");
        let handler = tenant_root.join("main.ds");
        std::fs::write(
            &handler,
            "import { createStore } from '@tana/store'\nexport fn App() string { return createStore(); }\n",
        )
        .expect("handler");

        // The bundle selects its project root from source-tree markers, not a
        // process-global module root.
        let bundle = build_deka_handler_bundle(handler.to_str().expect("utf-8 handler"));
        assert!(bundle.is_ok(), "tenant bundle failed: {bundle:?}");
    }

    #[test]
    fn bundle_failure_in_one_tenant_does_not_break_the_next_tenant_bundle() {
        let tmp = tempfile::tempdir().expect("tmp");
        let tenant_a_root = tmp.path().join("tenant-a");
        let tenant_b_root = tmp.path().join("tenant-b");
        std::fs::create_dir_all(&tenant_a_root).expect("tenant A root");
        std::fs::create_dir_all(&tenant_b_root).expect("tenant B root");

        for root in [&tenant_a_root, &tenant_b_root] {
            std::fs::write(root.join("deka.json"), "{}").expect("tenant manifest");
            std::fs::write(root.join("deka.lock"), "{}").expect("tenant lock");
        }

        let tenant_a_handler = tenant_a_root.join("main.ds");
        std::fs::write(
            &tenant_a_handler,
            "export fn App() string { return\n",
        )
        .expect("tenant A handler");

        let tenant_b_handler = tenant_b_root.join("main.ds");
        std::fs::write(
            &tenant_b_handler,
            "export fn App() string { return 'b'; }\n",
        )
        .expect("tenant B handler");

        let tenant_a =
            build_deka_handler_bundle(tenant_a_handler.to_str().expect("utf-8 handler"));
        assert!(tenant_a.is_err(), "tenant A must fail to bundle invalid source");

        let tenant_b_bundle =
            build_deka_handler_bundle(tenant_b_handler.to_str().expect("utf-8 handler"));
        assert!(
            tenant_b_bundle.is_ok(),
            "tenant B must still bundle after tenant A fails: {tenant_b_bundle:?}"
        );
    }
}
