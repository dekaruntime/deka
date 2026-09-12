//! `api-entry.ds` / `worker-entry.ds`: the `api/**/route.ds` graph compiled
//! into a single `runApiRouter` registry, keyed by route then HTTP method.

use std::path::{Path, PathBuf};

use super::super::compiler_cache_dir;
use super::super::manifest::{FrameworkEntry, exported_http_methods, scan_api_dir};
use super::super::routes::assert_dynamic_route_supported;
use super::{alias, json_str, pathdiff_dsx};
/// `.dsx` cannot import `api/`, so API dispatch lives in a sibling `.ds` file.
pub fn write_api_router_entry(project_root: &Path) -> Result<PathBuf, String> {
    let api_entries = scan_api_dir(&project_root.join("api"));
    if api_entries.is_empty() {
        return Err("no api/route.ds modules".to_string());
    }
    let cache_dir = compiler_cache_dir(project_root);
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let entry = cache_dir.join("api-entry.ds");
    let source = generate_api_entry_source(&entry, &api_entries)?;
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    Ok(entry)
}
/// API graph compiled into `dist/_worker.js`.
pub fn write_worker_router_entry(project_root: &Path) -> Result<PathBuf, String> {
    let cache_dir = compiler_cache_dir(project_root);
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let entry = cache_dir.join("worker-entry.ds");
    let source = generate_worker_entry(&entry, project_root)?;
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    Ok(entry)
}
/// Build the `api-entry.ds` source for `entry` without writing anything;
/// `deka build` reuses this for build-time server-entry emission.
pub fn generate_api_entry_source(
    entry: &Path,
    api_entries: &[FrameworkEntry],
) -> Result<String, String> {
    let (imports, registry) = api_imports_and_registry(entry, api_entries)?;
    Ok(format!(
        r#"{imports}import {{ runApiRouter }} from "ui/router"

interface RequestHeaders {{ accept: string }}
interface ResponseHeaders {{ location: string }}
interface Request {{ url: string, pathname: string, method: string, headers: RequestHeaders, body: string }}
interface Response {{ status: number, body: string, headers: ResponseHeaders }}

fn App(request: Request) Response {{
    const boxed = unsafe {{ runApiRouter(request, {registry}) }}
    return match (boxed) {{
        Ok(r) => r,
        Err(_) => {{ status: 500, body: "Internal Server Error", headers: {{ location: "" }} }},
    }}
}}
export {{ App }}
"#
    ))
}

fn api_imports_and_registry(
    entry: &Path,
    api_entries: &[FrameworkEntry],
) -> Result<(String, String), String> {
    let mut imports = String::new();
    let mut imported: Vec<String> = Vec::new();
    let mut import_alias = |path: &str, name: &str, alias: &str| -> Result<(), String> {
        if imported.iter().any(|k| k == alias) {
            return Ok(());
        }
        imported.push(alias.to_string());
        let rel = json_str(&pathdiff_dsx(entry, Path::new(path)))?;
        imports.push_str(&format!("import {{ {name} as {alias} }} from {rel}\n"));
        Ok(())
    };
    let mut registry = String::from("{ ");
    let mut first_route = true;
    for api in api_entries {
        let methods = exported_http_methods(Path::new(&api.file));
        if methods.is_empty() {
            continue;
        }
        assert_dynamic_route_supported(&api.route)?;
        let stem = alias("api", &api.route);
        for method in &methods {
            import_alias(&api.file, method, &format!("{method}_{stem}"))?;
        }
        if !first_route {
            registry.push_str(", ");
        }
        first_route = false;
        registry.push_str(&format!("{}: {{ ", json_str(&api.route)?));
        for (i, method) in methods.iter().enumerate() {
            if i > 0 {
                registry.push_str(", ");
            }
            registry.push_str(&format!("{method}: {method}_{stem}"));
        }
        registry.push_str(" }");
    }
    registry.push_str(" }");
    Ok((imports, registry))
}

fn generate_worker_entry(entry: &Path, project_root: &Path) -> Result<String, String> {
    let mut imports = String::new();
    let api_entries = scan_api_dir(&project_root.join("api"));
    let (api_imports, registry) = api_imports_and_registry(entry, &api_entries)?;
    imports.push_str(&api_imports);
    Ok(format!(
        r#"{imports}import {{ runApiRouter }} from "ui/router"

interface RequestHeaders {{ accept: string }}
interface ResponseHeaders {{ location: string }}
interface Request {{ url: string, pathname: string, method: string, headers: RequestHeaders, body: string }}
interface Response {{ status: number, body: string, headers: ResponseHeaders }}

fn App(request: Request) Response {{
    const boxed = unsafe {{ runApiRouter(request, {registry}) }}
    return match (boxed) {{
        Ok(r) => r,
        Err(_) => {{ status: 500, body: "Internal Server Error", headers: {{ location: "" }} }},
    }}
}}
export {{ App }}
"#
    ))
}
