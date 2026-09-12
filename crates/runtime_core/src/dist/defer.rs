//! `server:defer` island scanning and the RFD 24 §9.3 structural lints
//! (missing `slot="fallback"` is a build error; an all-deferred content
//! region is a warning).

use std::path::Path;

use super::source::strip_ds_comments;

/// Prop names written on a deferred-island tag: whitespace-separated
/// `name`/`name=value` tokens that are not component names or `client:` /
/// `server:` directives. (Was shared with client-island scanning; the
/// client-island pipeline is gone, `server:defer` scanning is the only
/// consumer left.)
fn island_prop_names(tag_src: &str) -> Vec<String> {
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredIsland {
    pub component: String,
    pub file: String,
    pub props: Vec<String>,
    pub cache: Option<String>,
    pub has_fallback: bool,
}

pub fn scan_server_defer(app_dir: &Path) -> Vec<DeferredIsland> {
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
            out.extend(defer_in_source(&src, path.to_string_lossy().as_ref()));
        }
    }
    out
}

fn defer_in_source(src: &str, file: &str) -> Vec<DeferredIsland> {
    let stripped = strip_ds_comments(src);
    defer_in_source_raw(&stripped, file)
}

fn defer_in_source_raw(src: &str, file: &str) -> Vec<DeferredIsland> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        let Some(rel) = rest.find("server:defer") else {
            break;
        };
        let at = i + rel;
        let prefix = &src[..at];
        let tag_start = prefix.rfind('<').unwrap_or(0);
        let tag_end = src[tag_start..]
            .find('>')
            .map(|rel| tag_start + rel + 1)
            .unwrap_or(at + "server:defer".len());
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
            let self_closing = tag_src.trim_end().ends_with("/>") || tag_src.contains("/>");
            let has_fallback = !self_closing && fallback_in_element(src, tag_end, &component);
            let cache = defer_cache_attr(tag_src);
            out.push(DeferredIsland {
                component,
                file: file.to_string(),
                props: island_prop_names(tag_src),
                cache,
                has_fallback,
            });
        }
        i = at + "server:defer".len();
    }
    out
}

fn fallback_in_element(src: &str, tag_end: usize, component: &str) -> bool {
    let rest = &src[tag_end.min(src.len())..];
    let close = format!("</{component}>");
    let window = rest.find(&close).map(|i| &rest[..i]).unwrap_or(rest);
    window.contains("slot=\"fallback\"")
        || window.contains("slot='fallback'")
        || window.contains("slot={\"fallback\"}")
}

fn defer_cache_attr(tag_src: &str) -> Option<String> {
    for raw in tag_src.split_whitespace() {
        if let Some(rest) = raw.strip_prefix("cache=") {
            let value = rest.trim_matches(|c| {
                c == '"' || c == '\'' || c == '{' || c == '}' || c == '>' || c == '/'
            });
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
/// Level of a §9.3 `server:defer` structural lint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferLintLevel {
    Error,
    Warning,
}

/// A §9.3 structural finding from scanning the app/ tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferLint {
    pub level: DeferLintLevel,
    pub file: String,
    pub message: String,
}

/// Structural checks over the app/ tree (RFD 24 §9.3 amendment,
/// dekaruntime/rfd#46):
/// - ERROR: a `server:defer` island has no `slot="fallback"` child at all.
///   The spec makes fallback required; absence must fail the build instead
///   of silently rendering an empty default.
/// - WARNING: every child of a route's content region is deferred, so the
///   no-JS render shows only loading indicators.
pub fn scan_defer_lints(app_dir: &Path) -> Vec<DeferLint> {
    let mut lints: Vec<DeferLint> = scan_server_defer(app_dir)
        .into_iter()
        .filter(|d| !d.has_fallback)
        .map(|d| DeferLint {
            level: DeferLintLevel::Error,
            file: d.file.clone(),
            message: format!(
                "server:defer requires a child with slot=\"fallback\" ({})",
                d.component
            ),
        })
        .collect();
    if !app_dir.is_dir() {
        return lints;
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
            let file = path.to_string_lossy().into_owned();
            let stripped = strip_ds_comments(&src);
            if returned_jsx_children_all_deferred(&stripped) {
                lints.push(DeferLint {
                    level: DeferLintLevel::Warning,
                    file: file.clone(),
                    message: format!(
                        "every child of the content region in {file} is deferred; the no-JS render shows only loading indicators"
                    ),
                });
            }
        }
    }
    lints
}

/// True when a `return <...>` block in `src` has children and every one of
/// them carries `server:defer`. Text or `{expression}` children count as
/// real content; whitespace between children is ignored.
fn returned_jsx_children_all_deferred(src: &str) -> bool {
    let mut from = 0;
    let mut saw_all_deferred = false;
    while let Some(rel) = src[from..].find("return <") {
        let at = from + rel + "return ".len();
        from = at + 1;
        let Some((body_start, body_end)) = jsx_element_body(src, at) else {
            continue;
        };
        if !jsx_children_all_deferred(&src[body_start..body_end]) {
            return false;
        }
        saw_all_deferred = true;
        from = body_end;
    }
    saw_all_deferred
}

/// Body range `(start, end)` of the JSX element whose `<` is at `start`.
/// Self-closed elements report an empty range.
fn jsx_element_body(src: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = src.as_bytes();
    if bytes.get(start) != Some(&b'<') || src[start..].starts_with("</") {
        return None;
    }
    let open_end = src[start..].find('>')? + start + 1;
    if src[start..open_end].trim_end().ends_with("/>") {
        return Some((open_end, open_end));
    }
    let mut depth = 1usize;
    let mut i = open_end;
    while i < bytes.len() {
        let rest = &src[i..];
        if rest.starts_with("</") {
            depth -= 1;
            if depth == 0 {
                return Some((open_end, i));
            }
            i += 2;
        } else if rest.starts_with('<') {
            let end = rest.find('>')? + i + 1;
            if !src[i..end].trim_end().ends_with("/>") {
                depth += 1;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    None
}

/// Index just past the JSX element whose `<` is at `start`.
fn skip_jsx_element(src: &str, start: usize) -> Option<usize> {
    jsx_element_body(src, start).and_then(|(body_start, body_end)| {
        if body_start == body_end {
            Some(body_start)
        } else {
            src[body_end..].find('>').map(|g| body_end + g + 1)
        }
    })
}

/// True when `body` has at least one child element and every direct child
/// element carries `server:defer` in its opening tag.
fn jsx_children_all_deferred(body: &str) -> bool {
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut saw_child = false;
    while i < bytes.len() {
        match bytes[i] {
            b'<' if body[i..].starts_with("</") => return saw_child,
            b'<' => {
                let Some(grel) = body[i..].find('>') else {
                    return false;
                };
                let open_end = i + grel + 1;
                let open_tag = &body[i..open_end];
                if !open_tag.contains("server:defer") {
                    return false;
                }
                saw_child = true;
                i = if open_tag.trim_end().ends_with("/>") {
                    open_end
                } else {
                    skip_jsx_element(body, i).unwrap_or(open_end)
                };
            }
            b if b.is_ascii_whitespace() => i += 1,
            _ => return false,
        }
    }
    saw_child
}

/// §9.3 build diagnostics: ERROR lints fail the build; WARNING lints print
/// to stderr. Matches the `Err(String)` style of the other scan findings.
pub(super) fn enforce_defer_lints(app_dir: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    for lint in scan_defer_lints(app_dir) {
        match lint.level {
            DeferLintLevel::Warning => eprintln!("warning: {}", lint.message),
            DeferLintLevel::Error => errors.push(lint.message),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}
pub fn defer_script_tag(has_defer: bool) -> String {
    if has_defer {
        "<script type=\"module\" src=\"/assets/islands-defer.js\"></script>".to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deka_{tag}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scan_server_defer_requires_fallback_slot() {
        let missing = defer_in_source(
            "export fn Page() {\n    return <Cart server:defer userId={id} />;\n}\n",
            "app/page.dsx",
        );
        assert_eq!(missing.len(), 1);
        assert!(!missing[0].has_fallback);
        let ok = defer_in_source(
            "export fn Page() {\n    return <Cart server:defer cache=\"60s\"><span slot=\"fallback\">.</span></Cart>;\n}\n",
            "app/page.dsx",
        );
        assert_eq!(ok.len(), 1);
        assert!(ok[0].has_fallback);
        assert_eq!(ok[0].component, "Cart");
        assert_eq!(ok[0].cache.as_deref(), Some("60s"));
    }

    #[test]
    fn defer_script_tag_is_exact() {
        assert_eq!(
            defer_script_tag(true),
            "<script type=\"module\" src=\"/assets/islands-defer.js\"></script>"
        );
        assert_eq!(defer_script_tag(false), "");
    }

    #[test]
    fn scan_skips_commented_defer_directives() {
        let src = "/* <Badge server:defer></Badge> */\nexport fn Page() { return <div /> }\n";
        assert!(defer_in_source(src, "app/page.dsx").is_empty());
    }

    #[test]
    fn defer_lints_error_without_fallback_and_pass_with_fallback() {
        let tmp = tmp_dir("defer_lint");
        std::fs::create_dir_all(tmp.join("app")).unwrap();
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <main><h1>Hi</h1></main>;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        assert!(
            lints.iter().all(|l| l.level != DeferLintLevel::Error),
            "static content must not error: {lints:?}"
        );
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <Cart server:defer />;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        let errors: Vec<_> = lints
            .iter()
            .filter(|l| l.level == DeferLintLevel::Error)
            .collect();
        assert_eq!(errors.len(), 1, "{lints:?}");
        assert!(
            errors[0].message.contains("slot=\"fallback\""),
            "{}",
            errors[0].message
        );
        assert!(errors[0].message.contains("Cart"), "{}", errors[0].message);
        // Negative case: a defer WITH fallback must not error.
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <Cart server:defer><span slot=\"fallback\">.</span></Cart>;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        assert!(
            lints.iter().all(|l| l.level != DeferLintLevel::Error),
            "defer with fallback must not error: {lints:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn defer_lints_warn_when_every_content_child_is_deferred() {
        let tmp = tmp_dir("defer_warn");
        std::fs::create_dir_all(tmp.join("app")).unwrap();
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <main><A server:defer><i slot=\"fallback\">a</i></A><B server:defer><i slot=\"fallback\">b</i></B></main>;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        let warnings: Vec<_> = lints
            .iter()
            .filter(|l| l.level == DeferLintLevel::Warning)
            .collect();
        assert_eq!(warnings.len(), 1, "{lints:?}");
        assert!(
            warnings[0]
                .message
                .contains("every child of the content region"),
            "{}",
            warnings[0].message
        );
        // Mixed content: no warning.
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <main><h1>Hi</h1><A server:defer><i slot=\"fallback\">a</i></A></main>;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        assert!(
            lints.iter().all(|l| l.level != DeferLintLevel::Warning),
            "mixed content must not warn: {lints:?}"
        );
        // Empty content region: no warning either.
        std::fs::write(
            tmp.join("app/page.dsx"),
            "export fn Page() {\n    return <main></main>;\n}\n",
        )
        .unwrap();
        let lints = scan_defer_lints(&tmp.join("app"));
        assert!(
            lints.iter().all(|l| l.level != DeferLintLevel::Warning),
            "empty content region must not warn: {lints:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
