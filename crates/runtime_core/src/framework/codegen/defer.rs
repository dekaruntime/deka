//! `defer-entry.dsx`: the `/_deka/defer` batch endpoint behind
//! `runDeferBatch`, bound to the per-project defer secret and session cookie.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::super::defer::{DeferredIsland, scan_server_defer};
use super::{ensure_defer_secret, json_str, pathdiff_dsx, session_cookie_name};
pub fn write_defer_router_entry(project_root: &Path) -> Result<PathBuf, String> {
    let deferred = scan_server_defer(&project_root.join("app"));
    if deferred.is_empty() {
        return Err("no server:defer islands".to_string());
    }
    let missing: Vec<_> = deferred.iter().filter(|d| !d.has_fallback).collect();
    if !missing.is_empty() {
        let names = missing
            .iter()
            .map(|d| d.component.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "server:defer requires a child with slot=\"fallback\" ({names})"
        ));
    }
    let cache_dir = project_root.join(".cache").join("dekascript");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|err| format!("failed to create {}: {err}", cache_dir.display()))?;
    let entry = cache_dir.join("defer-entry.dsx");
    let source = generate_defer_entry(&entry, project_root, &deferred)?;
    std::fs::write(&entry, source.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", entry.display()))?;
    Ok(entry)
}

pub(super) fn generate_defer_entry(
    entry: &Path,
    project_root: &Path,
    deferred: &[DeferredIsland],
) -> Result<String, String> {
    let mut imports = String::new();
    let mut registry = String::from("{ ");
    let mut seen = BTreeSet::new();
    let mut first = true;
    for (idx, item) in deferred.iter().enumerate() {
        let key = format!("{}:{}", item.file, item.component);
        if !seen.insert(key) {
            continue;
        }
        let alias = format!("Defer_{idx}");
        let rel = json_str(&pathdiff_dsx(entry, Path::new(&item.file)))?;
        let name = json_str(&item.component)?;
        imports.push_str(&format!(
            "import {{ {} as {alias} }} from {rel}\n",
            item.component
        ));
        if !first {
            registry.push_str(", ");
        }
        first = false;
        registry.push_str(&format!("{name}: {alias}"));
    }
    registry.push_str(" }");
    let cache_control = json_str(&format!("private, {}", defer_cache_header(deferred)))?;
    let secret = json_str(&ensure_defer_secret(project_root)?)?;
    let cookie = json_str(&session_cookie_name(project_root))?;
    Ok(format!(
        r#"{imports}import {{ runDeferBatch }} from "ui/server"

interface RequestHeaders {{ accept: string, cookie: string }}
interface Request {{ url: string, pathname: string, method: string, headers: RequestHeaders, body: string }}
interface Response {{ status: number, body: string }}

async fn App(request: Request) Promise<Response> {{
    const boxed = unsafe {{ runDeferBatch(request.body, {secret}, {registry}, {cache_control}, request, {cookie}) }}
    const prom = match (boxed) {{
        Ok(p) => p,
        Err(_) => {{ status: 500, body: "Internal Server Error" }},
    }}
    return await prom
}}
export {{ App }}
"#
    ))
}
// Shared-cache `cache="60s"` shells reuse one encrypted marker. Do not put
// viewer-specific props on those islands; AAD binds name+cookie, not a login.
fn defer_cache_header(deferred: &[DeferredIsland]) -> String {
    let mut max_age: Option<u64> = None;
    for item in deferred {
        match item.cache.as_deref() {
            Some("no-store") | None => return "no-store".to_string(),
            Some(raw) => {
                let Some(secs) = raw.trim().trim_end_matches('s').parse::<u64>().ok() else {
                    return "no-store".to_string();
                };
                max_age = Some(max_age.map(|a| a.min(secs)).unwrap_or(secs));
            }
        }
    }
    match max_age {
        Some(secs) => format!("max-age={secs}"),
        None => "no-store".to_string(),
    }
}
