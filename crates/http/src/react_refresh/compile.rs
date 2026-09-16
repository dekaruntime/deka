//! On-the-fly compile of a project file for the browser Fast Refresh graph.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use super::transform::{detect_components, wrap_module};

#[derive(Clone, Debug, Default)]
pub struct RefreshContext {
    pub project_root: PathBuf,
    pub dsc: Option<PathBuf>,
}

static CONTEXT: OnceLock<Mutex<RefreshContext>> = OnceLock::new();
static DSC_DEV: OnceLock<Mutex<HashMap<PathBuf, bool>>> = OnceLock::new();

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

pub fn install(context: RefreshContext) {
    let mut context = context;
    context.project_root = fs::canonicalize(&context.project_root).unwrap_or(context.project_root);
    if let Ok(mut guard) = CONTEXT
        .get_or_init(|| Mutex::new(RefreshContext::default()))
        .lock()
    {
        *guard = context;
    }
}

pub fn context() -> RefreshContext {
    CONTEXT
        .get()
        .and_then(|lock| lock.lock().ok().map(|guard| guard.clone()))
        .unwrap_or_default()
}

pub fn compile_relative(rel: &str) -> Result<String, String> {
    let ctx = context();
    if ctx.project_root.as_os_str().is_empty() {
        return Err("fast refresh is not installed for this process".to_string());
    }
    let abs = resolve_under_root(&ctx.project_root, rel)?;
    compile_file(&ctx, &abs, rel)
}

pub fn compile_abs(path: &Path) -> Result<(String, String), String> {
    let ctx = context();
    if ctx.project_root.as_os_str().is_empty() {
        return Err("fast refresh is not installed for this process".to_string());
    }
    let abs = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let rel = abs
        .strip_prefix(&ctx.project_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .map_err(|_| {
            format!(
                "changed path {} is outside the project {}",
                abs.display(),
                ctx.project_root.display()
            )
        })?;
    let js = compile_file(&ctx, &abs, &rel)?;
    Ok((rel, js))
}

fn compile_file(ctx: &RefreshContext, abs: &Path, rel: &str) -> Result<String, String> {
    let source = fs::read_to_string(abs)
        .map_err(|err| format!("failed to read {}: {err}", abs.display()))?;
    let ext = abs
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let emitted = match ext.as_str() {
        "ds" | "dsx" => compile_ds(ctx, abs)?,
        "js" | "mjs" | "jsx" => source.clone(),
        other => return Err(format!("fast refresh does not compile .{other} files")),
    };
    let components = detect_components(&source, &emitted);
    Ok(wrap_module(&emitted, rel, &components))
}

fn compile_ds(ctx: &RefreshContext, abs: &Path) -> Result<String, String> {
    let dsc = ctx
        .dsc
        .as_ref()
        .ok_or_else(|| "dsc is required to compile DekaScript for Fast Refresh".to_string())?;
    // deka#1065: route through the same shared helper every other compiler
    // cache consumer uses, so there is genuinely one resolution path for
    // "where does the dev cache live" instead of a second hardcoded literal
    // that could drift from it.
    let out = runtime_core::dist::compiler_cache_dir_with(&ctx.project_root, true)
        .join("refresh-modules");
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let out = out.join(format!("{}-{}", std::process::id(), stamp));
    fs::create_dir_all(&out).map_err(|err| format!("failed to create {}: {err}", out.display()))?;
    let mut args = vec![
        "transpile".to_string(),
        abs.to_string_lossy().into_owned(),
        "--self-contained".to_string(),
        "--out".to_string(),
        out.to_string_lossy().into_owned(),
    ];
    if dsc_supports_dev(dsc) {
        args.push("--dev".to_string());
    }
    let output = Command::new(dsc)
        .current_dir(&ctx.project_root)
        .args(&args)
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(&out);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            format!("dsc transpile exited {}", output.status)
        } else {
            detail.to_string()
        });
    }
    let js = read_emitted_js(&out, abs).or_else(|_| {
        // `--out` as a file rather than a graph dir (single-file emit).
        let sibling = abs.with_extension("js");
        fs::read_to_string(&sibling)
            .map_err(|err| format!("dsc transpile wrote no JS for {} ({err})", abs.display()))
    })?;
    let _ = fs::remove_dir_all(&out);
    Ok(js)
}

fn read_emitted_js(out: &Path, source: &Path) -> Result<String, String> {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("index");
    let candidates = [
        out.join(source.file_name().unwrap_or_default())
            .with_extension("js"),
        out.join(format!("{stem}.js")),
    ];
    for path in candidates {
        if path.is_file() {
            return fs::read_to_string(&path)
                .map_err(|err| format!("failed to read {}: {err}", path.display()));
        }
    }
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        for entry in
            fs::read_dir(dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))?
        {
            let entry = entry.map_err(|err| err.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files)?;
            } else if path.extension().and_then(|e| e.to_str()) == Some("js") {
                files.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(out, &mut files)?;
    files
        .into_iter()
        .next()
        .ok_or_else(|| "dsc transpile wrote no modules".to_string())
        .and_then(|path| {
            fs::read_to_string(&path)
                .map_err(|err| format!("failed to read {}: {err}", path.display()))
        })
}

pub fn dsc_supports_dev(dsc: &Path) -> bool {
    let key = fs::canonicalize(dsc).unwrap_or_else(|_| dsc.to_path_buf());
    let cache = DSC_DEV.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(known) = guard.get(&key) {
            return *known;
        }
    }
    let output = Command::new(dsc)
        .args(["transpile", "--help"])
        .output()
        .ok();
    let supported = output
        .map(|output| {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            text.contains("--dev")
        })
        .unwrap_or(false);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, supported);
    }
    supported
}

fn resolve_under_root(root: &Path, rel: &str) -> Result<PathBuf, String> {
    if rel.is_empty() || rel.contains('\0') {
        return Err("empty module path".to_string());
    }
    let rel_path = Path::new(rel);
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("module path escaped the project".to_string());
    }
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let abs = fs::canonicalize(root.join(rel_path)).unwrap_or_else(|_| root.join(rel_path));
    if !abs.starts_with(&root) {
        return Err("module path escaped the project".to_string());
    }
    if !abs.is_file() {
        return Err(format!("no such module {}", rel));
    }
    Ok(abs)
}

#[cfg(test)]
mod tests {
    use super::resolve_under_root;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn compile_passes_dev_when_dsc_help_lists_it() {
        let _lock = super::TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fast-refresh-it/stub-dsc");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("Card.dsx"),
            "export fn Card() ReactNode { return <p>hi</p>; }\n",
        )
        .unwrap();
        let log = root.join("dsc.log");
        let stub = root.join("dsc");
        fs::write(
            &stub,
            format!(
                "#!/bin/sh\necho \"$@\" >> \"{}\"\ncase \"$1\" in\n  transpile)\n    if echo \"$@\" | grep -q -- --help; then echo 'usage: dsc transpile [--dev]'; exit 0; fi\n    out=\"\"\n    prev=\"\"\n    for arg in \"$@\"; do\n      if [ \"$prev\" = --out ]; then out=\"$arg\"; fi\n      prev=\"$arg\"\n    done\n    mkdir -p \"$out\"\n    printf '%s\\n' 'import {{ jsxDEV }} from \"@js/react/jsx-dev-runtime\"; export function Card() {{ return jsxDEV(\"p\", {{\"children\":\"hi\"}}, undefined, false, {{fileName:\"Card.dsx\", lineNumber:1, columnNumber:28}}, this); }}' > \"$out/Card.js\"\n    exit 0\n    ;;\nesac\nexit 0\n",
                log.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&stub).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&stub, perms).unwrap();
        }
        super::install(super::RefreshContext {
            project_root: root.clone(),
            dsc: Some(stub.clone()),
        });
        assert!(super::dsc_supports_dev(&stub));
        let js = super::compile_relative("Card.dsx").expect("compile dsx");
        let argv = fs::read_to_string(&log).unwrap();
        assert!(
            argv.contains("--dev"),
            "dev compile must request jsxDEV: {argv}"
        );
        assert!(js.contains("jsxDEV"));
        assert!(js.contains("fileName:\"Card.dsx\"") || js.contains("fileName: \"Card.dsx\""));
        assert!(js.contains("$RefreshReg$(Card, \"Card\")"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_parent_and_absolute_paths() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/fast-refresh-it/compile-resolve");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("ok.js"), "export const n = 1\n").unwrap();
        fs::write(root.join("secret.js"), "nope\n").unwrap();
        assert!(resolve_under_root(&root, "ok.js").is_ok());
        assert!(resolve_under_root(&root, "../secret.js").is_err());
        assert!(resolve_under_root(&root, "/etc/passwd").is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
