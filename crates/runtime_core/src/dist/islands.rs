//! Client-island scanning (`client:load` / `client:idle` / `client:visible`)
//! and the script tag the app-router document injects for the shared
//! hydration bundle.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::source::{exports_fn_named, strip_ds_comments};

/// One island component that should hydrate in the browser.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClientIsland {
    /// Exported component name (`ThemeToggle`).
    pub name: String,
    /// Absolute path of the module that exports `name`.
    pub module: PathBuf,
    /// `load`, `idle`, or `visible`.
    pub directive: String,
}

/// Logical URL of the shared islands bundle. Content hashing rewrites this
/// to `/assets/islands.<hash>.js` at `deka build`.
pub const ISLANDS_SCRIPT_SRC: &str = "/assets/islands.js";

pub fn islands_script_tag() -> String {
    format!("<script type=\"module\" src=\"{ISLANDS_SCRIPT_SRC}\"></script>")
}

/// Walk `app/` and `src/` for uppercase JSX tags carrying `client:*`.
pub fn scan_client_islands(project_root: &Path) -> Vec<ClientIsland> {
    let mut found: BTreeSet<ClientIsland> = BTreeSet::new();
    for dir_name in ["app", "src"] {
        let dir = project_root.join(dir_name);
        if !dir.is_dir() {
            continue;
        }
        let mut stack = vec![dir];
        while let Some(dir) = stack.pop() {
            let Ok(reader) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in reader.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                if ext != "dsx" && ext != "ds" {
                    continue;
                }
                let Ok(src) = std::fs::read_to_string(&path) else {
                    continue;
                };
                found.extend(islands_in_source(&src, &path));
            }
        }
    }
    found.into_iter().collect()
}

fn islands_in_source(src: &str, file: &Path) -> Vec<ClientIsland> {
    let stripped = strip_ds_comments(src);
    let mut out = Vec::new();
    for (name, directive) in client_tags(&stripped) {
        let module = resolve_island_module(&stripped, file, &name).unwrap_or_else(|| file.to_path_buf());
        out.push(ClientIsland {
            name,
            module,
            directive,
        });
    }
    out
}

fn client_tags(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        let Some(rel) = rest.find("client:") else {
            break;
        };
        let at = i + rel;
        let directive_start = at + "client:".len();
        let directive = match src[directive_start..].split(|c: char| !c.is_ascii_alphabetic()).next() {
            Some("load") => "load",
            Some("idle") => "idle",
            Some("visible") => "visible",
            _ => {
                i = directive_start;
                continue;
            }
        };
        let prefix = &src[..at];
        let tag_start = prefix.rfind('<').unwrap_or(0);
        let component = src[tag_start..]
            .trim_start_matches('<')
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or("")
            .to_string();
        let is_component = component
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase());
        if is_component {
            out.push((component, directive.to_string()));
        }
        i = directive_start + directive.len();
    }
    out
}

fn resolve_island_module(src: &str, file: &Path, name: &str) -> Option<PathBuf> {
    let parent = file.parent().unwrap_or(file);
    for spec in import_specs_for(src, name) {
        let resolved = parent.join(spec);
        return Some(normalize_path(&resolved));
    }
    if exports_fn_named(src, name) {
        return Some(file.to_path_buf());
    }
    None
}

fn import_specs_for(src: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(at) = rest.find("import ") {
        let after = &rest[at + "import ".len()..];
        let Some(from_at) = after.find(" from ") else {
            rest = &after[1.min(after.len())..];
            continue;
        };
        let clause = after[..from_at].trim();
        let spec_src = after[from_at + " from ".len()..].trim_start();
        let spec = take_quoted(spec_src);
        rest = &after[from_at + 1..];
        if spec.is_empty() {
            continue;
        }
        if import_clause_binds(clause, name) {
            out.push(spec);
        }
    }
    out
}

fn import_clause_binds(clause: &str, name: &str) -> bool {
    let clause = clause.trim();
    if let Some(inner) = clause.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        for part in inner.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let local = part.split(" as ").nth(1).unwrap_or(part).trim();
            if local == name {
                return true;
            }
        }
        return false;
    }
    false
}

fn take_quoted(src: &str) -> String {
    let bytes = src.as_bytes();
    let Some(&quote) = bytes.first() else {
        return String::new();
    };
    if quote != b'"' && quote != b'\'' {
        return String::new();
    }
    let mut j = 1;
    while j < bytes.len() && bytes[j] != quote {
        if bytes[j] == b'\\' {
            j += 2;
            continue;
        }
        j += 1;
    }
    if j >= bytes.len() {
        return String::new();
    }
    src[1..j].to_string()
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

/// WYSIWYG client entry: import hydrateRoot + each island, then hydrate
/// every `<deka-island>` the SSR wrapper emitted.
pub fn generate_islands_entry_js(islands: &[ClientIsland], entry_dir: &Path) -> String {
    let mut out = String::from(
        "// Generated by deka. Hydrates compiled DSX islands via hydrateRoot.\n\
         import { hydrateRoot } from \"@js/react-dom/client\";\n\
         import { jsx } from \"@js/react/jsx-runtime\";\n",
    );
    let mut seen = BTreeSet::new();
    for island in islands {
        if !seen.insert(&island.name) {
            continue;
        }
        let rel = relative_specifier(entry_dir, &island.module);
        let spec = compiled_js_specifier(&rel);
        out.push_str(&format!(
            "import {{ {} }} from {};\n",
            island.name,
            json_str(&spec)
        ));
    }
    out.push_str("const registry = {\n");
    let mut seen = BTreeSet::new();
    for island in islands {
        if !seen.insert(&island.name) {
            continue;
        }
        out.push_str(&format!("  {0}: {0},\n", island.name));
    }
    out.push_str(
        "};\n\
         for (const node of document.querySelectorAll(\"[data-deka-island]\")) {\n\
           const name = node.getAttribute(\"data-deka-island\");\n\
           const Component = registry[name];\n\
           if (!Component) continue;\n\
           let props = {};\n\
           const raw = node.getAttribute(\"data-deka-props\");\n\
           if (raw) props = JSON.parse(raw);\n\
           hydrateRoot(node, jsx(Component, props));\n\
         }\n",
    );
    out
}

fn compiled_js_specifier(spec: &str) -> String {
    spec.strip_suffix(".dsx")
        .or_else(|| spec.strip_suffix(".ds"))
        .map(|stem| format!("{stem}.js"))
        .unwrap_or_else(|| spec.to_string())
}

fn relative_specifier(from_dir: &Path, to_file: &Path) -> String {
    let from = from_dir.components().collect::<Vec<_>>();
    let to = to_file.components().collect::<Vec<_>>();
    let mut i = 0;
    while i < from.len() && i < to.len() && from[i] == to[i] {
        i += 1;
    }
    let mut parts = Vec::new();
    for _ in i..from.len() {
        parts.push("..");
    }
    for component in to.iter().skip(i) {
        parts.push(component.as_os_str().to_str().unwrap_or(""));
    }
    let spec = if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    };
    let spec = spec.replace('\\', "/");
    if spec.starts_with('.') {
        spec
    } else {
        format!("./{spec}")
    }
}

fn json_str(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_client_load_and_resolves_import() {
        let src = "import { ThemeToggle } from \"./Theme.dsx\"\n\
                   export fn Layout() ReactNode {\n\
                     return <header><ThemeToggle client:load /></header>\n\
                   }\n";
        let file = PathBuf::from("/proj/src/ui/Shell.dsx");
        let islands = islands_in_source(src, &file);
        assert_eq!(islands.len(), 1);
        assert_eq!(islands[0].name, "ThemeToggle");
        assert_eq!(islands[0].directive, "load");
        assert_eq!(
            islands[0].module,
            PathBuf::from("/proj/src/ui/Theme.dsx")
        );
    }

    #[test]
    fn scan_same_file_export() {
        let src = "export fn Counter() ReactNode {\n  return <button>0</button>\n}\n\
                   export fn Page() ReactNode {\n  return <Counter client:idle />\n}\n";
        let file = PathBuf::from("/proj/app/page.dsx");
        let islands = islands_in_source(src, &file);
        assert_eq!(islands.len(), 1);
        assert_eq!(islands[0].name, "Counter");
        assert_eq!(islands[0].directive, "idle");
        assert_eq!(islands[0].module, file);
    }

    #[test]
    fn host_tags_are_not_islands() {
        let src = "export fn Page() ReactNode {\n  return <div client:load>x</div>\n}\n";
        let file = PathBuf::from("/proj/app/page.dsx");
        assert!(islands_in_source(src, &file).is_empty());
    }

    #[test]
    fn generated_entry_imports_compiled_js() {
        let islands = vec![ClientIsland {
            name: "ThemeToggle".into(),
            module: PathBuf::from("/proj/src/ui/Theme.dsx"),
            directive: "load".into(),
        }];
        let js = generate_islands_entry_js(&islands, Path::new("/proj/.cache/dekascript"));
        assert!(js.contains("from \"@js/react-dom/client\""));
        assert!(js.contains("from \"@js/react/jsx-runtime\""));
        assert!(js.contains("import { ThemeToggle } from \"../../src/ui/Theme.js\""));
        assert!(js.contains("hydrateRoot(node, jsx(Component, props))"));
        assert!(!js.contains(".dsx"));
    }
}
