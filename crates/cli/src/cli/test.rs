use core::{CommandSpec, Context, ParamSpec, Registry};
use runtime_core::modules::MODULES_DIR;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use stdio;

const COMMAND: CommandSpec = CommandSpec {
    owner: "",
    name: "test",
    category: "runtime",
    summary: "run DekaScript tests",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

const TEST_LIB_SOURCE: &str = include_str!("../../test_lib/index.ds");
const TEST_LIB_MANIFEST: &str = include_str!("../../test_lib/deka.json");
const TEST_PACKAGE: &str = "@deka/test";

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_param(ParamSpec {
        name: "--test-name-pattern",
        description: "only run tests with names matching the pattern",
    });
    registry.add_param(ParamSpec {
        name: "-t",
        description: "only run tests with names matching the pattern",
    });
}

pub fn cmd(context: &Context) {
    if let Err(message) = run_tests(context) {
        stdio::error("test", &message);
        std::process::exit(1);
    }
}

fn run_tests(context: &Context) -> Result<(), String> {
    let cwd = context.env.cwd.clone();
    let filters = context.args.positionals.clone();
    let pattern = context
        .args
        .params
        .get("--test-name-pattern")
        .or_else(|| context.args.params.get("-t"))
        .cloned()
        .unwrap_or_default();

    let files = resolve_test_files(&cwd, &filters)?;
    if files.is_empty() {
        stdio::log("test", "no deka tests found");
        return Ok(());
    }

    stdio::log(
        "test",
        &format!("[runtime] {} test file(s)", files.len()),
    );
    for file in &files {
        let rel = file.strip_prefix(&cwd).unwrap_or(file).to_string_lossy();
        stdio::log("test", &format!("[deka:{}]", rel));
    }

    let _workspace = TestWorkspace::prepare(&cwd, &files, &pattern)?;
    let exe = std::env::current_exe()
        .map_err(|err| format!("failed to resolve executable: {}", err))?;
    let mut cmd = Command::new(&exe);
    cmd.arg("run")
        .arg(&_workspace.runner)
        .arg("--allow-all")
        .arg("--no-prompt")
        .current_dir(&cwd)
        .env("DEKA_BIN", &exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for flag in security_flags_from_context(context) {
        cmd.arg(flag);
    }

    let output = cmd
        .output()
        .map_err(|err| format!("failed to start test runtime: {}", err))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }

    if !output.status.success() {
        return Err("tests failed".to_string());
    }
    match failed_count(&stdout) {
        Some(0) => Ok(()),
        Some(_) => Err("tests failed".to_string()),
        None => Err("test runner produced no summary".to_string()),
    }
}

fn failed_count(stdout: &str) -> Option<u32> {
    stdout.lines().rev().find_map(|line| {
        if !line.contains("[test] total=") {
            return None;
        }
        line.split_whitespace()
            .find_map(|part| part.strip_prefix("failed=")?.parse().ok())
    })
}

struct TestWorkspace {
    runner: PathBuf,
    vendor: VendorGuard,
    keep_runner: bool,
}

impl TestWorkspace {
    fn prepare(cwd: &Path, files: &[PathBuf], pattern: &str) -> Result<Self, String> {
        let vendor = vendor_test_lib(cwd)?;
        let runner = write_runner(cwd, files, pattern)?;
        Ok(Self {
            runner,
            vendor,
            keep_runner: std::env::var("DEKA_TEST_KEEP").ok().as_deref() == Some("1"),
        })
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        if !self.keep_runner {
            let _ = fs::remove_file(&self.runner);
        }
        self.vendor.restore();
    }
}

struct VendorGuard {
    pkg: PathBuf,
    scope: PathBuf,
    modules: PathBuf,
    created_pkg: bool,
    created_scope: bool,
    created_modules: bool,
    lock_path: PathBuf,
    lock_backup: Option<Vec<u8>>,
    manifest_path: PathBuf,
    manifest_backup: Option<Vec<u8>>,
    restored: bool,
}

impl VendorGuard {
    fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        match &self.lock_backup {
            Some(backup) => {
                let _ = fs::write(&self.lock_path, backup);
            }
            None => {
                let _ = fs::remove_file(&self.lock_path);
            }
        }
        // The vendored package is only resolvable while the run lasts; the
        // manifest must return to its pre-run state too (deka#600).
        match &self.manifest_backup {
            Some(backup) => {
                let _ = fs::write(&self.manifest_path, backup);
            }
            None => {
                let _ = fs::remove_file(&self.manifest_path);
            }
        }
        if self.created_pkg {
            let _ = fs::remove_dir_all(&self.pkg);
        }
        if self.created_scope && dir_is_empty(&self.scope) {
            let _ = fs::remove_dir(&self.scope);
        }
        if self.created_modules && dir_is_empty(&self.modules) {
            let _ = fs::remove_dir(&self.modules);
        }
    }
}

impl Drop for VendorGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn vendor_test_lib(cwd: &Path) -> Result<VendorGuard, String> {
    let modules = cwd.join(MODULES_DIR);
    let scope = modules.join("@deka");
    let pkg = scope.join("test");
    let created_modules = !modules.exists();
    let created_scope = !scope.exists();
    let created_pkg = !pkg.exists();

    fs::create_dir_all(&pkg).map_err(|err| format!("failed to vendor {TEST_PACKAGE}: {err}"))?;
    if created_pkg || !pkg.join("index.ds").exists() {
        fs::write(pkg.join("index.ds"), TEST_LIB_SOURCE)
            .map_err(|err| format!("failed to write {TEST_PACKAGE}: {err}"))?;
        fs::write(pkg.join("deka.json"), TEST_LIB_MANIFEST)
            .map_err(|err| format!("failed to write {TEST_PACKAGE} manifest: {err}"))?;
    }

    let lock_path = cwd.join("deka.lock");
    let lock_backup = if lock_path.exists() {
        Some(fs::read(&lock_path).map_err(|err| format!("failed to read deka.lock: {err}"))?)
    } else {
        None
    };

    let integrity = deka_host::integrity::compute_package_integrity(&pkg)
        .map_err(|err| format!("failed to hash {TEST_PACKAGE}: {err}"))?;
    write_lock_entry(
        &lock_path,
        TEST_PACKAGE,
        &integrity.module_graph,
        &integrity.fs_graph,
    )?;

    // The runtime project gate rejects imports of packages not declared in
    // deka.json (deka#403), so vendoring must declare @deka/test for the
    // duration of the run and restore the manifest afterwards.
    let manifest_path = cwd.join("deka.json");
    let manifest_backup = if manifest_path.exists() {
        Some(fs::read(&manifest_path).map_err(|err| format!("failed to read deka.json: {err}"))?)
    } else {
        None
    };
    declare_test_dependency(&manifest_path)?;

    Ok(VendorGuard {
        pkg,
        scope,
        modules,
        created_pkg,
        created_scope,
        created_modules,
        lock_path,
        lock_backup,
        manifest_path,
        manifest_backup,
        restored: false,
    })
}

fn declare_test_dependency(manifest_path: &Path) -> Result<(), String> {
    let raw = if manifest_path.exists() {
        fs::read_to_string(manifest_path).map_err(|err| format!("failed to read deka.json: {err}"))?
    } else {
        String::from("{}")
    };
    let mut manifest: serde_json::Value =
        serde_json::from_str(&raw).map_err(|err| format!("failed to parse deka.json: {err}"))?;
    manifest
        .as_object_mut()
        .ok_or_else(|| "deka.json must be a JSON object".to_string())?
        .entry("dependencies")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| "deka.json `dependencies` must be a JSON object".to_string())?
        .insert(TEST_PACKAGE.to_string(), serde_json::json!("0.1.0"));
    fs::write(
        manifest_path,
        serde_json::to_string_pretty(&manifest).map_err(|err| format!("failed to serialize deka.json: {err}"))?,
    )
    .map_err(|err| format!("failed to write deka.json: {err}"))
}

fn write_lock_entry(
    lock_path: &Path,
    name: &str,
    module_graph: &str,
    fs_graph: &str,
) -> Result<(), String> {
    let lock: serde_json::Value = if lock_path.exists() {
        let raw = fs::read_to_string(lock_path)
            .map_err(|err| format!("failed to read deka.lock: {err}"))?;
        serde_json::from_str(&raw).unwrap_or_else(|_| empty_lock())
    } else {
        empty_lock()
    };

    let mut packages = lock
        .get("packages")
        .and_then(|value| value.as_object())
        .cloned()
        .or_else(|| {
            lock.get("php")
                .and_then(|value| value.get("packages"))
                .and_then(|value| value.as_object())
                .cloned()
        })
        .unwrap_or_default();
    packages.insert(
        name.to_string(),
        serde_json::json!([
            format!("{name}@0.1.0"),
            format!("linkhash:{name}"),
            {
                "moduleGraph": { "hash": module_graph },
                "fsGraph": { "hash": fs_graph }
            },
            ""
        ]),
    );

    let lockfile_version = lock
        .get("lockfileVersion")
        .and_then(|value| value.as_u64())
        .unwrap_or(1);
    let body = serde_json::json!({
        "lockfileVersion": lockfile_version,
        "packages": packages
    });
    fs::write(
        lock_path,
        serde_json::to_string_pretty(&body)
            .map_err(|err| format!("failed to serialize deka.lock: {err}"))?,
    )
    .map_err(|err| format!("failed to write deka.lock: {err}"))?;
    Ok(())
}

fn empty_lock() -> serde_json::Value {
    serde_json::json!({
        "lockfileVersion": 1,
        "packages": {}
    })
}

fn write_runner(cwd: &Path, files: &[PathBuf], pattern: &str) -> Result<PathBuf, String> {
    let runner = cwd.join(format!(".deka-test-runner-{}.ds", std::process::id()));
    let mut source = String::from("import { run } from \"@deka/test\"\n");
    for file in files {
        let spec = import_spec(cwd, file);
        source.push_str("import \"");
        source.push_str(&escape_ds_string(&spec));
        source.push_str("\"\n");
    }
    source.push_str("run(\"");
    source.push_str(&escape_ds_string(pattern));
    source.push_str("\")\n");
    fs::write(&runner, source).map_err(|err| format!("failed to write runner: {err}"))?;
    Ok(runner)
}

fn import_spec(cwd: &Path, file: &Path) -> String {
    match file.strip_prefix(cwd) {
        Ok(rel) => format!("./{}", rel.to_string_lossy().replace('\\', "/")),
        Err(_) => file.to_string_lossy().replace('\\', "/"),
    }
}

fn dir_is_empty(path: &Path) -> bool {
    fs::read_dir(path)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
}

fn escape_ds_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn resolve_test_files(cwd: &Path, filters: &[String]) -> Result<Vec<PathBuf>, String> {
    let all_tests = discover_all_tests(cwd)?;
    if filters.is_empty() {
        return Ok(all_tests);
    }

    let mut resolved = Vec::new();
    let mut substring_filters = Vec::new();
    for filter in filters {
        let candidate = if Path::new(filter).is_absolute() {
            PathBuf::from(filter)
        } else {
            cwd.join(filter)
        };
        if candidate.exists() {
            resolved.extend(scan_for_tests(&candidate)?);
            continue;
        }
        substring_filters.push(filter.as_str());
    }

    if !substring_filters.is_empty() {
        for file in &all_tests {
            let haystack = file.to_string_lossy();
            if substring_filters
                .iter()
                .any(|filter| haystack.contains(filter))
            {
                resolved.push(file.clone());
            }
        }
    }

    Ok(unique_paths(resolved))
}

fn discover_all_tests(cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let tests_dir = cwd.join("tests");
    let test_dir = cwd.join("test");
    if tests_dir.exists() || test_dir.exists() {
        let mut results = Vec::new();
        if tests_dir.exists() {
            results.extend(scan_for_tests(&tests_dir)?);
        }
        if test_dir.exists() {
            results.extend(scan_for_tests(&test_dir)?);
        }
        return Ok(unique_paths(results));
    }
    scan_for_tests(cwd)
}

fn scan_for_tests(root: &Path) -> Result<Vec<PathBuf>, String> {
    if root.is_file() {
        return Ok(if is_test_file(root) {
            vec![root.to_path_buf()]
        } else {
            Vec::new()
        });
    }

    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                return Err(format!("failed to read {}: {}", dir.display(), err));
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                if should_skip_dir(&path) {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() && is_test_file(&path) {
                results.push(path);
            }
        }
    }
    Ok(results)
}

fn should_skip_dir(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    matches!(name, "target" | "dist" | ".git" | "ds_modules" | "php_modules")
}

fn is_test_file(path: &Path) -> bool {
    let file_name = match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name.to_ascii_lowercase(),
        None => return false,
    };
    if !file_name.ends_with(".ds") && !file_name.ends_with(".dsx") {
        return false;
    }
    file_name == "test.ds"
        || file_name == "spec.ds"
        || file_name.contains(".test.ds")
        || file_name.contains(".spec.ds")
}

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for path in paths {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            output.push(path);
        }
    }
    output
}

fn security_flags_from_context(context: &Context) -> Vec<String> {
    const SECURITY_FLAGS: &[&str] = &[
        "--allow-read",
        "--allow-write",
        "--allow-net",
        "--allow-env",
        "--allow-run",
        "--allow-db",
        "--allow-dynamic",
        "--allow-wasm",
        "--allow-all",
        "--deny-read",
        "--deny-write",
        "--deny-net",
        "--deny-env",
        "--deny-run",
        "--deny-db",
        "--deny-dynamic",
        "--deny-wasm",
        "--no-prompt",
    ];
    SECURITY_FLAGS
        .iter()
        .filter(|name| context.args.flags.get(**name).copied().unwrap_or(false))
        .map(|name| (*name).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::is_test_file;
    use std::path::Path;

    #[test]
    fn discovers_dot_test_ds() {
        assert!(is_test_file(Path::new("tests/math.test.ds")));
        assert!(is_test_file(Path::new("tests/math.spec.ds")));
        assert!(is_test_file(Path::new("test.ds")));
    }

    #[test]
    fn ignores_hats_and_tour() {
        assert!(!is_test_file(Path::new(
            "corpus/functions/pipe_operator/pipe_operator.pass.ds"
        )));
        assert!(!is_test_file(Path::new("tests/tour/pipe-operator.ds")));
        assert!(!is_test_file(Path::new("src/math.ds")));
        assert!(!is_test_file(Path::new("foo.test.ts")));
    }
}
