//! Compile the shared islands hydration bundle: dsc-emitted island modules
//! plus a WYSIWYG `hydrateRoot` entry, with production React inlined.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use runtime_core::dist::{ClientIsland, generate_islands_entry_js, scan_client_islands};

use crate::dsc_compile;
use crate::js_builtins;

/// Build the shared islands bundle into `out_path` (typically
/// `<cache>/assets/islands.js` or `dist/client/assets/islands.js`).
/// Returns `Ok(false)` when the project has no client islands.
pub fn emit_islands_bundle(
    project_root: &Path,
    dsc: &Path,
    out_path: &Path,
) -> Result<bool, String> {
    let islands = scan_client_islands(project_root);
    if islands.is_empty() {
        return Ok(false);
    }
    let js = bundle_islands(project_root, dsc, &islands, out_path)?;
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    std::fs::write(out_path, js.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", out_path.display()))?;
    Ok(true)
}

pub fn bundle_islands(
    project_root: &Path,
    dsc: &Path,
    islands: &[ClientIsland],
    out_path: &Path,
) -> Result<String, String> {
    let mut modules: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for island in islands {
        if !seen.insert(island.module.clone()) {
            continue;
        }
        let graph = dsc_compile::compile_graph_with_dsc(project_root, &island.module, dsc)
            .map_err(|err| {
                format!(
                    "failed to compile island {}: {err}",
                    island.module.display()
                )
            })?;
        for (path, js) in graph {
            modules.entry(path).or_insert(js);
        }
    }
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let entry_dir = out_path
        .parent()
        .map(|dir| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()))
        .unwrap_or_else(|| out_path.to_path_buf());
    let islands: Vec<ClientIsland> = islands
        .iter()
        .map(|island| ClientIsland {
            module: std::fs::canonicalize(&island.module).unwrap_or_else(|_| island.module.clone()),
            name: island.name.clone(),
            directive: island.directive.clone(),
        })
        .collect();
    let entry_js = generate_islands_entry_js(&islands, &entry_dir);
    let entry_path = entry_dir.join("islands.js");
    modules.insert(entry_path.clone(), entry_js);
    let bundled = concat_esm_graph(&entry_path, &modules)?;
    js_builtins::inline_into(&bundled)
}

fn concat_esm_graph(
    entry: &Path,
    modules: &BTreeMap<PathBuf, String>,
) -> Result<String, String> {
    let mut order = Vec::new();
    let mut visiting = BTreeSet::new();
    let mut seen = BTreeSet::new();
    visit(entry, modules, &mut visiting, &mut seen, &mut order)?;

    let mut out = String::from(
        "// deka: shared islands bundle (compiled DSX + production React)\n\
         const __deka_island_mods = Object.create(null);\n\
         function __deka_island_export(id, exports) {\n\
           __deka_island_mods[id] = exports;\n\
         }\n\
         function __deka_island_import(id) {\n\
           const m = __deka_island_mods[id];\n\
           if (!m) throw new Error(\"island module missing: \" + id);\n\
           return m;\n\
         }\n",
    );
    for path in &order {
        let js = modules.get(path).ok_or_else(|| {
            format!("island graph missing {}", path.display())
        })?;
        let id = module_id(path);
        let is_entry = path == entry;
        out.push_str(&rewrite_module(js, path, modules, &id, is_entry)?);
    }
    Ok(out)
}

fn visit(
    path: &Path,
    modules: &BTreeMap<PathBuf, String>,
    visiting: &mut BTreeSet<PathBuf>,
    seen: &mut BTreeSet<PathBuf>,
    order: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if !seen.insert(path.to_path_buf()) {
        return Ok(());
    }
    if !visiting.insert(path.to_path_buf()) {
        return Err(format!("cycle in island graph at {}", path.display()));
    }
    let js = match modules.get(path) {
        Some(js) => js,
        None => {
            if let Some(alt) = lookup_source_key(path, modules) {
                return visit(&alt, modules, visiting, seen, order);
            }
            visiting.remove(path);
            return Err(format!(
                "island graph does not contain {}",
                path.display()
            ));
        }
    };
    for spec in relative_imports(js) {
        let resolved = resolve_relative(path, &spec);
        let key = lookup_source_key(&resolved, modules).unwrap_or(resolved);
        visit(&key, modules, visiting, seen, order)?;
    }
    visiting.remove(path);
    order.push(path.to_path_buf());
    Ok(())
}

fn lookup_source_key(js_path: &Path, modules: &BTreeMap<PathBuf, String>) -> Option<PathBuf> {
    if modules.contains_key(js_path) {
        return Some(js_path.to_path_buf());
    }
    let text = js_path.to_string_lossy();
    if let Some(stem) = text.strip_suffix(".js") {
        for ext in [".dsx", ".ds"] {
            let candidate = PathBuf::from(format!("{stem}{ext}"));
            if modules.contains_key(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn resolve_relative(importer: &Path, spec: &str) -> PathBuf {
    let parent = importer.parent().unwrap_or(importer);
    normalize_path(&parent.join(spec))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn module_id(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn relative_imports(js: &str) -> Vec<String> {
    js_builtins_import_specs(js)
        .into_iter()
        .filter(|spec| spec.starts_with("./") || spec.starts_with("../"))
        .collect()
}

fn js_builtins_import_specs(js: &str) -> Vec<String> {
    deka_modules::ds_imports::paths(js)
}

fn rewrite_module(
    js: &str,
    path: &Path,
    modules: &BTreeMap<PathBuf, String>,
    id: &str,
    is_entry: bool,
) -> Result<String, String> {
    let (body, specs) = strip_relative_imports(js, path, modules)?;
    let (body, exported) = strip_exports(&body);
    let mut out = String::from("{\n");
    for (spec, names) in specs {
        out.push_str(&format!(
            "const {{ {} }} = __deka_island_import({});\n",
            names.join(", "),
            json_str(&spec)
        ));
    }
    out.push_str(&body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if is_entry {
        out.push_str("}\n");
        return Ok(out);
    }
    out.push_str(&format!(
        "__deka_island_export({}, {{ {} }});\n}}\n",
        json_str(id),
        exported.join(", ")
    ));
    Ok(out)
}

fn strip_relative_imports(
    js: &str,
    path: &Path,
    modules: &BTreeMap<PathBuf, String>,
) -> Result<(String, Vec<(String, Vec<String>)>), String> {
    let mut specs = Vec::new();
    let mut out = String::with_capacity(js.len());
    let mut rest = js;
    while let Some((prefix, _decl, spec, names, suffix)) = next_import(rest) {
        out.push_str(prefix);
        if spec.starts_with("./") || spec.starts_with("../") {
            let resolved = resolve_relative(path, &spec);
            let key = lookup_source_key(&resolved, modules).unwrap_or(resolved);
            specs.push((module_id(&key), names));
        } else {
            out.push_str(&rebuild_import(&spec, &names));
        }
        rest = suffix;
    }
    out.push_str(rest);
    Ok((out, specs))
}

struct ImportedName {
    imported: String,
    local: String,
}

fn rebuild_import(spec: &str, names: &[String]) -> String {
    if names.is_empty() {
        return format!("import {};\n", json_str(spec));
    }
    format!("import {{ {} }} from {};\n", names.join(", "), json_str(spec))
}

fn next_import(source: &str) -> Option<(&str, &str, String, Vec<String>, &str)> {
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if is_word_at(bytes, i, b"import") {
            let start = i;
            i += 6;
            i = skip_ws(bytes, i);
            if let Some((names, after_clause)) = take_import_clause(bytes, i) {
                i = skip_ws(bytes, after_clause);
                if is_word_at(bytes, i, b"from") {
                    i = skip_ws(bytes, i + 4);
                    if let Some((spec, after_spec)) = take_quoted(bytes, i) {
                        let mut end = skip_ws(bytes, after_spec);
                        if bytes.get(end) == Some(&b';') {
                            end += 1;
                        }
                        if bytes.get(end) == Some(&b'\n') {
                            end += 1;
                        }
                        let locals = names
                            .into_iter()
                            .map(|n| {
                                if n.imported == n.local {
                                    n.local
                                } else {
                                    format!("{}: {}", n.imported, n.local)
                                }
                            })
                            .collect();
                        return Some((
                            &source[..start],
                            &source[start..end],
                            spec,
                            locals,
                            &source[end..],
                        ));
                    }
                }
            }
            i = start + 1;
            continue;
        }
        i += 1;
    }
    None
}

fn take_import_clause(bytes: &[u8], mut i: usize) -> Option<(Vec<ImportedName>, usize)> {
    i = skip_ws(bytes, i);
    if bytes.get(i) == Some(&b'"') || bytes.get(i) == Some(&b'\'') {
        return Some((Vec::new(), i));
    }
    if bytes.get(i) == Some(&b'{') {
        let close = bytes[i + 1..].iter().position(|&b| b == b'}')? + i + 1;
        let inner = std::str::from_utf8(&bytes[i + 1..close]).ok()?;
        let mut names = Vec::new();
        for part in inner.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            names.push(parse_as_name(part));
        }
        return Some((names, close + 1));
    }
    None
}

fn parse_as_name(part: &str) -> ImportedName {
    if let Some((imported, local)) = part.split_once(" as ") {
        ImportedName {
            imported: imported.trim().to_string(),
            local: local.trim().to_string(),
        }
    } else {
        ImportedName {
            imported: part.to_string(),
            local: part.to_string(),
        }
    }
}

fn take_quoted(bytes: &[u8], i: usize) -> Option<(String, usize)> {
    let quote = *bytes.get(i)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut j = i + 1;
    while j < bytes.len() && bytes[j] != quote {
        if bytes[j] == b'\\' {
            j += 2;
            continue;
        }
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let spec = std::str::from_utf8(&bytes[i + 1..j]).ok()?.to_string();
    Some((spec, j + 1))
}

fn is_word_at(bytes: &[u8], i: usize, word: &[u8]) -> bool {
    let end = i + word.len();
    if end > bytes.len() || &bytes[i..end] != word {
        return false;
    }
    let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
    let after_ok = end == bytes.len() || !is_ident_byte(bytes[end]);
    before_ok && after_ok
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn strip_exports(js: &str) -> (String, Vec<String>) {
    let mut exported = Vec::new();
    let mut out = String::with_capacity(js.len());
    let mut rest = js;
    while let Some(at) = rest.find("export ") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "export ".len()..];
        if let Some(name) = after.strip_prefix("function ").and_then(ident_at_start) {
            exported.push(name);
            out.push_str("function ");
            rest = after.strip_prefix("function ").unwrap_or(after);
            continue;
        }
        if let Some(name) = after.strip_prefix("async function ").and_then(ident_at_start) {
            exported.push(name);
            out.push_str("async function ");
            rest = after.strip_prefix("async function ").unwrap_or(after);
            continue;
        }
        if let Some(rest_kw) = after.strip_prefix("const ") {
            if let Some(name) = ident_at_start(rest_kw) {
                exported.push(name);
            }
            out.push_str("const ");
            rest = rest_kw;
            continue;
        }
        if after.starts_with('{') {
            if let Some(close) = after.find('}') {
                let inner = &after[1..close];
                for part in inner.split(',') {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    let exported_name = part.split(" as ").nth(1).unwrap_or(part).trim();
                    exported.push(exported_name.to_string());
                }
                rest = after[close + 1..].trim_start_matches(|c| c == ';' || c == '\n');
                if rest.starts_with('\n') {
                    rest = &rest[1..];
                }
                continue;
            }
        }
        out.push_str("export ");
        rest = after;
    }
    out.push_str(rest);
    (out, exported)
}

fn ident_at_start(s: &str) -> Option<String> {
    let mut end = 0;
    for (i, ch) in s.char_indices() {
        if i == 0 {
            if !(ch.is_ascii_alphabetic() || ch == '_' || ch == '$') {
                return None;
            }
        } else if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$') {
            end = i;
            break;
        }
        end = i + ch.len_utf8();
    }
    if end == 0 {
        None
    } else {
        Some(s[..end].to_string())
    }
}

fn json_str(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concat_inlines_relative_and_keeps_js_builtins() {
        let mut modules = BTreeMap::new();
        let counter = PathBuf::from("/proj/src/ui/Counter.dsx");
        let entry = PathBuf::from("/proj/.cache/dekascript/islands.js");
        modules.insert(
            counter.clone(),
            "import { jsx } from \"@js/react/jsx-runtime\";\n\
             import { useState } from \"@js/react\";\n\
             export function Counter(props) {\n\
               return jsx(\"button\", { children: props.start });\n\
             }\n"
            .to_string(),
        );
        modules.insert(
            entry.clone(),
            "import { hydrateRoot } from \"@js/react-dom/client\";\n\
             import { jsx } from \"@js/react/jsx-runtime\";\n\
             import { Counter } from \"../../src/ui/Counter.js\";\n\
             hydrateRoot(document.body, jsx(Counter, { start: 1 }));\n"
                .to_string(),
        );
        let bundled = concat_esm_graph(&entry, &modules).expect("concat");
        assert!(bundled.contains("function Counter(props)"));
        assert!(bundled.contains("from \"@js/react-dom/client\""));
        assert!(bundled.contains("from \"@js/react/jsx-runtime\""));
        assert!(!bundled.contains("from \"../../src/ui/Counter.js\""));
        assert!(bundled.contains("hydrateRoot(document.body"));
    }
}
