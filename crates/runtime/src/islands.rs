//! Compile `client:*` island modules into `/assets/islands-{directive}.js`.
//!
//! Never copies `ui/server`. Chunks register islands then call `hydrate()`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use runtime_core::framework::ClientIsland;

pub fn find_app_router_root(start: &Path) -> Option<PathBuf> {
    let mut cur = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        if runtime_core::framework::is_app_router_project(&cur) {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

pub fn write_island_client_assets_for_project(project_root: &Path) -> Result<(), String> {
    if !runtime_core::framework::is_app_router_project(project_root) {
        return Ok(());
    }
    let app_dir = project_root.join("app");
    let islands = runtime_core::framework::scan_client_islands(&app_dir);
    let deferred = runtime_core::framework::scan_server_defer(&app_dir);
    if islands.is_empty() && deferred.is_empty() {
        return Ok(());
    }
    write_island_client_assets(
        &project_root.join(".cache").join("dekascript").join("assets"),
        &islands,
    )?;
    if !deferred.is_empty() {
        write_defer_client_assets(
            &project_root.join(".cache").join("dekascript").join("assets"),
        )?;
    }
    Ok(())
}

pub fn write_island_client_assets(
    assets_dir: &Path,
    islands: &[ClientIsland],
) -> Result<(), String> {
    let ui_dir = assets_dir.join("ui");
    fs::create_dir_all(&ui_dir)
        .map_err(|err| format!("failed to create {}: {err}", ui_dir.display()))?;
    for (name, source) in [
        ("jsx.js", deka_ui::JSX),
        ("reactive.js", deka_ui::REACTIVE),
        ("client.js", deka_ui::CLIENT),
    ] {
        fs::write(ui_dir.join(name), source.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", ui_dir.join(name).display()))?;
    }
    fs::write(
        ui_dir.join("server-stub.js"),
        b"export function renderToString() { throw new Error(\"ui/server is not available in the browser\"); }\n",
    )
    .map_err(|err| {
        format!(
            "failed to write {}: {err}",
            ui_dir.join("server-stub.js").display()
        )
    })?;

    for directive in ["load", "idle", "visible"] {
        let group: Vec<&ClientIsland> = islands
            .iter()
            .filter(|island| island.directive == directive)
            .collect();
        if group.is_empty() {
            continue;
        }
        let mut imports = String::new();
        let mut registers = String::new();
        let mut seen_files = BTreeSet::new();
        let mut idx = 0usize;
        for island in &group {
            if !seen_files.insert(island.file.clone()) {
                continue;
            }
            let source = fs::read_to_string(&island.file).map_err(|err| {
                format!("failed to read {}: {err}", island.file)
            })?;
            let mut js = compile_js(&source, &island.file)?;
            js = rewrite_ui_imports(&js);
            let mod_name = format!("island-{directive}-{idx}.js");
            idx += 1;
            let names: Vec<&str> = group
                .iter()
                .filter(|item| item.file == island.file)
                .map(|item| item.component.as_str())
                .collect();
            let mut unique = Vec::new();
            for name in names {
                if !unique.contains(&name) {
                    unique.push(name);
                }
            }
            let spec_list = unique.join(", ");
            let mut missing = Vec::new();
            let mut to_export = Vec::new();
            for name in &unique {
                if js_exports_ident(&js, name) {
                    continue;
                }
                if js_has_ident_binding(&js, name) {
                    to_export.push(*name);
                } else {
                    missing.push(*name);
                }
            }
            if !missing.is_empty() {
                return Err(format!(
                    "client island {} is not exported from {}",
                    missing.join(", "),
                    island.file
                ));
            }
            if !to_export.is_empty() {
                js.push_str(&format!("\nexport {{ {} }};\n", to_export.join(", ")));
            }
            fs::write(assets_dir.join(&mod_name), js.as_bytes()).map_err(|err| {
                format!("failed to write {}: {err}", assets_dir.join(&mod_name).display())
            })?;
            imports.push_str(&format!(
                "import {{ {spec_list} }} from \"./{mod_name}\";\n"
            ));
            for name in unique {
                registers.push_str(&format!(
                    "registerIsland({name_json}, {name});\n",
                    name_json = serde_json::to_string(name).unwrap_or_else(|_| format!("\"{name}\"")),
                    name = name
                ));
            }
        }
        let entry = format!(
            "{imports}import {{ hydrate, registerIsland }} from \"./ui/client.js\";\n{registers}hydrate();\n"
        );
        let dest = assets_dir.join(format!("islands-{directive}.js"));
        fs::write(&dest, entry.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", dest.display()))?;
    }
    Ok(())
}

fn compile_js(source: &str, path: &str) -> Result<String, String> {
    match deka_compile::compile_to_js(source, path) {
        Ok(result) => Ok(result.js),
        Err(diagnostics) => Err(diagnostics
            .iter()
            .map(deka_compile::format_diagnostic)
            .collect::<Vec<_>>()
            .join("\n")),
    }
}

pub fn write_defer_client_assets(assets_dir: &Path) -> Result<(), String> {
    let ui_dir = assets_dir.join("ui");
    fs::create_dir_all(&ui_dir)
        .map_err(|err| format!("failed to create {}: {err}", ui_dir.display()))?;
    for (name, source) in [
        ("jsx.js", deka_ui::JSX),
        ("reactive.js", deka_ui::REACTIVE),
        ("client.js", deka_ui::CLIENT),
    ] {
        fs::write(ui_dir.join(name), source.as_bytes())
            .map_err(|err| format!("failed to write {}: {err}", ui_dir.join(name).display()))?;
    }
    let entry = "import { hydrate } from \"./ui/client.js\";\nhydrate();\n";
    let dest = assets_dir.join("islands-defer.js");
    fs::write(&dest, entry.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", dest.display()))?;
    Ok(())
}

fn ident_boundary_after(src: &str, _name: &str) -> bool {
    match src.chars().next() {
        None => true,
        Some(c) => !c.is_ascii_alphanumeric() && c != '_',
    }
}

fn contains_prefixed_ident(src: &str, prefix: &str, name: &str) -> bool {
    let needle = format!("{prefix}{name}");
    let mut rest = src;
    while let Some(at) = rest.find(&needle) {
        let before_ok = at == 0
            || rest[..at]
                .chars()
                .next_back()
                .map(|c| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(true);
        let after = &rest[at + needle.len()..];
        if before_ok && ident_boundary_after(after, name) {
            return true;
        }
        rest = &rest[at + 1..];
    }
    false
}

fn js_exports_ident(js: &str, name: &str) -> bool {
    contains_prefixed_ident(js, "export function ", name)
        || contains_prefixed_ident(js, "export async function ", name)
        || contains_prefixed_ident(js, "export const ", name)
        || contains_prefixed_ident(js, "export let ", name)
        || contains_prefixed_ident(js, "export var ", name)
        || js_export_list_contains(js, name)
}

fn js_has_ident_binding(js: &str, name: &str) -> bool {
    js_exports_ident(js, name)
        || contains_prefixed_ident(js, "function ", name)
        || contains_prefixed_ident(js, "async function ", name)
        || contains_prefixed_ident(js, "const ", name)
        || contains_prefixed_ident(js, "let ", name)
        || contains_prefixed_ident(js, "var ", name)
}

fn js_export_list_contains(js: &str, name: &str) -> bool {
    let mut rest = js;
    while let Some(at) = rest.find("export {") {
        let after = &rest[at + "export {".len()..];
        let Some(end) = after.find('}') else {
            break;
        };
        for part in after[..end].split(',') {
            let ident = part.trim().split_whitespace().next().unwrap_or("");
            if ident == name {
                return true;
            }
        }
        rest = &after[end.saturating_add(1)..];
    }
    false
}

fn rewrite_ui_imports(js: &str) -> String {
    js.replace("from \"ui/jsx\"", "from \"./ui/jsx.js\"")
        .replace("from \"ui/reactive\"", "from \"./ui/reactive.js\"")
        .replace("from \"ui/client\"", "from \"./ui/client.js\"")
        .replace("from \"ui/form\"", "from \"./ui/form.js\"")
        .replace("from \"ui/suspense\"", "from \"./ui/suspense.js\"")
        .replace("from \"ui/server\"", "from \"./ui/server-stub.js\"")
}
