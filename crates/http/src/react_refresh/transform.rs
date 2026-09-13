//! React Fast Refresh registration for emitted dev modules.
//!
//! Matches Vite plugin-react's `$RefreshReg$` / `$RefreshSig$` contract:
//! exported PascalCase functions are components, and a ReactNode/Component
//! return type (DS type info) is an additional positive signal. Hook order is
//! captured as a signature so a JSX-only edit preserves state while a hook
//! change resets that component.

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub hook_signature: String,
}

pub fn detect_components(source: &str, emitted_js: &str) -> Vec<Component> {
    let mut found: Vec<Component> = Vec::new();
    push_typed_ds_components(source, &mut found);
    push_pascal_exports(emitted_js, &mut found);
    push_pascal_exports(source, &mut found);
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found.dedup_by(|a, b| a.name == b.name);
    found
}

pub fn wrap_module(js: &str, module_id: &str, components: &[Component]) -> String {
    let (imports, body) = split_leading_imports(js);
    let mut out = String::with_capacity(js.len() + 800 + components.len() * 160);
    if !imports
        .lines()
        .any(|line| line.contains("/_deka/react/refresh-runtime.js"))
    {
        out.push_str("import * as __dekaRefresh from \"/_deka/react/refresh-runtime.js\";\n");
    }
    out.push_str(&imports);
    if !imports.is_empty() && !imports.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("const __dekaPrevReg = globalThis.$RefreshReg$;\n");
    out.push_str("const __dekaPrevSig = globalThis.$RefreshSig$;\n");
    out.push_str("globalThis.$RefreshReg$ = (type, id) => {\n");
    let _ = write!(
        out,
        "  __dekaRefresh.register(type, {module_id:?} + \" \" + id);\n"
    );
    out.push_str("};\n");
    out.push_str("globalThis.$RefreshSig$ = __dekaRefresh.createSignatureFunctionForTransform;\n");
    out.push_str(&body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    for component in components {
        let _ = write!(
            out,
            "if (typeof {name} === \"function\") {{\n  $RefreshReg$({name}, {name:?});\n",
            name = component.name
        );
        if !component.hook_signature.is_empty() {
            let _ = write!(
                out,
                "  __dekaRefresh.setSignature({name}, {sig:?});\n",
                name = component.name,
                sig = component.hook_signature
            );
        }
        out.push_str("}\n");
    }
    out.push_str("globalThis.$RefreshReg$ = __dekaPrevReg;\n");
    out.push_str("globalThis.$RefreshSig$ = __dekaPrevSig;\n");
    if components.is_empty() {
        out.push_str("export const __dekaRefreshBoundary = false;\n");
    } else {
        out.push_str("export const __dekaRefreshBoundary = true;\n");
    }
    out
}

pub fn is_refreshable_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".dsx")
        || lower.ends_with(".ds")
        || lower.ends_with(".js")
        || lower.ends_with(".mjs")
        || lower.ends_with(".jsx")
}

fn push_typed_ds_components(source: &str, found: &mut Vec<Component>) {
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(after_kw) = match_fn_keyword(&source[i..]) {
            let start = i;
            let name_start = i + after_kw;
            let rest = &source[name_start..];
            let name_len = ident_len(rest);
            if name_len == 0 {
                i += 1;
                continue;
            }
            let name = &rest[..name_len];
            if !is_pascal_case(name) {
                i = name_start + name_len;
                continue;
            }
            let after_name = &rest[name_len..];
            let Some(paren) = after_name.find('(') else {
                i = name_start + name_len;
                continue;
            };
            let after_params = skip_balanced(&after_name[paren..], '(', ')');
            let return_ty = return_type_before_brace(after_params);
            let typed = return_ty.is_some_and(is_component_return_type);
            let exported = source[..start].trim_end().ends_with("export")
                || source[..start].trim_end().ends_with("export default");
            if typed || (exported && is_pascal_case(name)) {
                let body = function_body(after_params);
                found.push(Component {
                    name: name.to_string(),
                    hook_signature: hook_signature(body),
                });
            }
            i = name_start + name_len;
            continue;
        }
        i += 1;
    }
}

fn push_pascal_exports(js: &str, found: &mut Vec<Component>) {
    for (idx, _) in js.match_indices("export ") {
        let after = js[idx + "export ".len()..].trim_start();
        let after = after
            .strip_prefix("default ")
            .map(str::trim_start)
            .unwrap_or(after);
        let after = if let Some(rest) = after.strip_prefix("async ") {
            rest.trim_start()
        } else {
            after
        };
        let name = if let Some(rest) = after.strip_prefix("function ") {
            take_ident(rest.trim_start())
        } else if let Some(rest) = after.strip_prefix("fn ") {
            take_ident(rest.trim_start())
        } else if let Some(rest) = after
            .strip_prefix("const ")
            .or_else(|| after.strip_prefix("let "))
        {
            take_ident(rest.trim_start())
        } else {
            None
        };
        let Some(name) = name else {
            continue;
        };
        if !is_pascal_case(name) {
            continue;
        }
        if found.iter().any(|c| c.name == name) {
            continue;
        }
        let body_from = &js[idx..];
        found.push(Component {
            name: name.to_string(),
            hook_signature: hook_signature(function_body(body_from)),
        });
    }
}

fn match_fn_keyword(s: &str) -> Option<usize> {
    if s.starts_with("function") {
        let rest = &s["function".len()..];
        if rest.starts_with(|c: char| c.is_whitespace()) {
            return Some("function".len() + rest.find(|c: char| !c.is_whitespace()).unwrap_or(0));
        }
    }
    if s.starts_with("fn") {
        let rest = &s["fn".len()..];
        if rest.starts_with(|c: char| c.is_whitespace()) {
            return Some("fn".len() + rest.find(|c: char| !c.is_whitespace()).unwrap_or(0));
        }
    }
    None
}

fn ident_len(s: &str) -> usize {
    s.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .map(|c| c.len_utf8())
        .sum()
}

fn take_ident(s: &str) -> Option<&str> {
    let n = ident_len(s);
    if n == 0 {
        None
    } else {
        Some(&s[..n])
    }
}

fn is_pascal_case(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
}

fn is_component_return_type(ty: &str) -> bool {
    let ty = ty.trim();
    ty == "ReactNode"
        || ty == "Component"
        || ty.starts_with("Component<")
        || ty == "ReactElement"
        || ty.starts_with("ReactElement<")
}

fn return_type_before_brace(s: &str) -> Option<&str> {
    let s = s.trim_start();
    let s = s.strip_prefix(':').map(str::trim_start).unwrap_or(s);
    let brace = s.find('{')?;
    let ty = s[..brace].trim();
    if ty.is_empty() {
        None
    } else {
        Some(ty)
    }
}

fn skip_balanced<'a>(s: &'a str, open: char, close: char) -> &'a str {
    let mut depth = 0i32;
    for (idx, ch) in s.char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return &s[idx + ch.len_utf8()..];
            }
        }
    }
    s
}

fn function_body(s: &str) -> &str {
    let Some(start) = s.find('{') else {
        return "";
    };
    let rest = &s[start..];
    let mut depth = 0i32;
    for (idx, ch) in rest.char_indices() {
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                return &rest[..idx + 1];
            }
        }
    }
    rest
}

fn hook_signature(body: &str) -> String {
    let mut hooks = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'u'
            && body[i..].starts_with("use")
            && body[i + 3..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
        {
            let name_len = ident_len(&body[i..]);
            let name = &body[i..i + name_len];
            let after = body[i + name_len..].trim_start();
            if after.starts_with('(') {
                hooks.push(name.to_string());
            }
            i += name_len;
            continue;
        }
        i += 1;
    }
    hooks.join("\n")
}

fn split_leading_imports(js: &str) -> (String, String) {
    let mut rest = js;
    let mut imports = String::new();
    loop {
        let trimmed_len = rest.len() - rest.trim_start().len();
        if trimmed_len > 0 {
            imports.push_str(&rest[..trimmed_len]);
            rest = &rest[trimmed_len..];
        }
        if rest.starts_with("//") {
            let end = rest.find('\n').map(|i| i + 1).unwrap_or(rest.len());
            imports.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        if rest.starts_with("/*") {
            let end = rest.find("*/").map(|i| i + 2).unwrap_or(rest.len());
            imports.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        if rest.starts_with("\"use strict\"") || rest.starts_with("'use strict'") {
            let end = rest.find('\n').map(|i| i + 1).unwrap_or(rest.len());
            imports.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        if rest.starts_with("import ") || rest.starts_with("import{") {
            let end = import_statement_end(rest);
            imports.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        break;
    }
    (imports, rest.to_string())
}

fn import_statement_end(s: &str) -> usize {
    let mut in_str = None;
    for (idx, ch) in s.char_indices() {
        if let Some(q) = in_str {
            if ch == q {
                in_str = None;
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            in_str = Some(ch);
            continue;
        }
        if ch == ';' {
            return idx + 1;
        }
        if ch == '\n' && s[..idx].contains(" from ") {
            return idx + 1;
        }
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_react_node_export_is_a_component() {
        let ds = r#"
interface Props { title: string }
export fn Card(props: Props) ReactNode {
  return <p>{props.title}</p>;
}
"#;
        let js = r#"
import { jsxDEV } from "@js/react/jsx-dev-runtime";
export function Card(props) {
  return jsxDEV("p", {"children": props.title}, undefined, false, {fileName: "Card.dsx", lineNumber: 3, columnNumber: 10}, this);
}
"#;
        let components = detect_components(ds, js);
        assert_eq!(components.len(), 1, "{components:?}");
        assert_eq!(components[0].name, "Card");
    }

    #[test]
    fn pascal_export_without_type_is_still_registered() {
        let js = "export function Counter() {\n  const [n, setN] = useState(0);\n  return n;\n}\n";
        let components = detect_components("", js);
        assert_eq!(components[0].name, "Counter");
        assert_eq!(components[0].hook_signature, "useState");
    }

    #[test]
    fn camel_case_helper_is_not_a_component() {
        let js = "export function formatCount(n) { return String(n); }\n";
        assert!(detect_components("", js).is_empty());
    }

    #[test]
    fn wrap_injects_refresh_registration_and_boundary() {
        let js = "import { jsxDEV } from \"@js/react/jsx-dev-runtime\";\nexport function Card() {\n  return jsxDEV(\"p\", {\"children\": \"hi\"}, undefined, false, {fileName: \"Card.dsx\", lineNumber: 1, columnNumber: 1}, this);\n}\n";
        let components = detect_components("", js);
        let wrapped = wrap_module(js, "Card.dsx", &components);
        assert!(
            wrapped.contains("import * as __dekaRefresh from \"/_deka/react/refresh-runtime.js\";")
        );
        assert!(wrapped.contains("import { jsxDEV } from \"@js/react/jsx-dev-runtime\";"));
        assert!(wrapped.contains("$RefreshReg$(Card, \"Card\")"));
        assert!(wrapped.contains("export const __dekaRefreshBoundary = true;"));
        assert!(wrapped.contains("Card.dsx"));
    }

    #[test]
    fn empty_module_is_not_a_refresh_boundary() {
        let js = "export const answer = 42;\n";
        let wrapped = wrap_module(js, "math.js", &[]);
        assert!(wrapped.contains("__dekaRefreshBoundary = false"));
        assert!(!wrapped.contains("$RefreshReg$("));
    }

    #[test]
    fn jsx_only_edit_keeps_hook_signature() {
        let before = "export function Counter() { const [n, setN] = useState(0); return n; }";
        let after = "export function Counter() { const [n, setN] = useState(0); return n + 1; }";
        let a = detect_components("", before);
        let b = detect_components("", after);
        assert_eq!(a[0].hook_signature, b[0].hook_signature);
        assert_eq!(a[0].hook_signature, "useState");
    }

    #[test]
    fn adding_a_hook_changes_the_signature() {
        let before = "export function Counter() { const [n, setN] = useState(0); return n; }";
        let after = "export function Counter() { const [n, setN] = useState(0); useEffect(() => {}, [n]); return n; }";
        let a = detect_components("", before);
        let b = detect_components("", after);
        assert_ne!(a[0].hook_signature, b[0].hook_signature);
        assert_eq!(b[0].hook_signature, "useState\nuseEffect");
    }
}
