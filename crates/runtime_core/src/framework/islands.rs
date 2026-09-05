//! Client-island scanning: finds `client:load`/`client:idle`/`client:visible`
//! directives in `.ds`/`.dsx` sources and derives the per-directive script
//! tags the document must load.

use std::path::Path;

use super::source::strip_ds_comments;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIsland {
    pub component: String,
    pub directive: String,
    pub file: String,
    pub props: Vec<String>,
}

pub fn scan_client_islands(app_dir: &Path) -> Vec<ClientIsland> {
    let mut out = Vec::new();
    if !app_dir.is_dir() {
        return out;
    }
    let mut stack = vec![app_dir.to_path_buf()];
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
            out.extend(islands_in_source(&src, path.to_string_lossy().as_ref()));
        }
    }
    out
}

fn islands_in_source(src: &str, file: &str) -> Vec<ClientIsland> {
    let stripped = strip_ds_comments(src);
    islands_in_source_raw(&stripped, file)
}

fn islands_in_source_raw(src: &str, file: &str) -> Vec<ClientIsland> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        let Some(rel) = ["client:load", "client:idle", "client:visible"]
            .iter()
            .filter_map(|needle| rest.find(needle).map(|at| (at, *needle)))
            .min_by_key(|(at, _)| *at)
        else {
            break;
        };
        let at = i + rel.0;
        let directive = rel.1.rsplit(':').next().unwrap_or("load").to_string();
        let prefix = &src[..at];
        let tag_start = prefix.rfind('<').unwrap_or(0);
        let tag_end = src[tag_start..]
            .find('>')
            .map(|rel| tag_start + rel + 1)
            .unwrap_or(at + rel.1.len());
        let tag_src = &src[tag_start..tag_end];
        let component = tag_src
            .trim_start_matches('<')
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or("")
            .to_string();
        let is_component = component
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false);
        if is_component {
            let props = island_prop_names(tag_src);
            out.push(ClientIsland {
                component,
                directive,
                file: file.to_string(),
                props,
            });
        }
        i = at + rel.1.len();
    }
    out
}

pub(super) fn island_prop_names(tag_src: &str) -> Vec<String> {
    let mut names = Vec::new();
    for raw in tag_src.split_whitespace() {
        let name = raw.split('=').next().unwrap_or("").trim();
        if name.is_empty()
            || name.starts_with('<')
            || name.starts_with("client:")
            || name.starts_with("server:")
        {
            continue;
        }
        if name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            names.push(name.to_string());
        }
    }
    names
}
pub fn island_script_tags(islands: &[ClientIsland]) -> String {
    let mut seen = std::collections::BTreeSet::new();
    let mut tags = String::new();
    for directive in ["load", "idle", "visible"] {
        if islands.iter().any(|i| i.directive == directive) && seen.insert(directive) {
            tags.push_str(&format!(
                "<script type=\"module\" src=\"/assets/islands-{directive}.js\"></script>"
            ));
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_client_islands_finds_directive_and_props() {
        let src = "export fn Page() {\n    return <Cart client:load userId={id} />;\n}\n";
        let found = islands_in_source(src, "app/page.dsx");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].component, "Cart");
        assert_eq!(found[0].directive, "load");
        assert_eq!(found[0].props, vec!["userId".to_string()]);
        // Exact tag, not `contains("islands-load.js")`: one module script per
        // directive, and no others.
        assert_eq!(
            island_script_tags(&found),
            "<script type=\"module\" src=\"/assets/islands-load.js\"></script>"
        );
    }

    #[test]
    fn scan_client_islands_ignores_server_defer() {
        let src = "export fn Page() {\n    return <Cart server:defer userId={id} />;\n}\n";
        let found = islands_in_source(src, "app/page.dsx");
        assert!(found.is_empty());
        assert_eq!(island_script_tags(&found), "");
    }

    #[test]
    fn scan_skips_commented_island_directives() {
        let src = "// <Cart client:load />\nexport fn Page() { return <div /> }\n";
        assert!(islands_in_source(src, "app/page.dsx").is_empty());
    }

    #[test]
    fn island_script_tags_empty_without_islands() {
        assert_eq!(island_script_tags(&[]), "");
    }
}
