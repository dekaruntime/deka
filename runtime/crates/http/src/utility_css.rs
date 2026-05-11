use serde_json::Value;
use std::collections::{BTreeSet, HashMap, hash_map::DefaultHasher};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

const MARKER: &str = "__deka_utility_css";
const DEFAULT_PREFLIGHT: bool = true;

#[derive(Clone, Copy, Debug)]
struct UtilityCssConfig {
    enabled: bool,
    preflight: bool,
}

pub fn inject_utility_css(html: &str) -> String {
    let config = load_config();
    if !config.enabled {
        return html.to_string();
    }
    inject_utility_css_with_config(html, config)
}

fn inject_utility_css_with_config(html: &str, config: UtilityCssConfig) -> String {
    if !config.enabled {
        return html.to_string();
    }
    if html.contains(MARKER) {
        return html.to_string();
    }
    let classes = collect_classes(html);
    if classes.is_empty() {
        return html.to_string();
    }
    let css = generate_css(&classes);
    if css.is_empty() {
        return html.to_string();
    }
    let preflight = if config.preflight {
        preflight_css()
    } else {
        ""
    };
    let style = format!("<style id=\"{}\">{}{}</style>", MARKER, preflight, css);
    if let Some(idx) = html.rfind("</head>") {
        let mut out = String::with_capacity(html.len() + style.len());
        out.push_str(&html[..idx]);
        out.push_str(&style);
        out.push_str(&html[idx..]);
        write_cache(&classes, preflight, &css);
        return out;
    }
    let mut out = String::with_capacity(html.len() + style.len());
    out.push_str(&style);
    out.push_str(html);
    write_cache(&classes, preflight, &css);
    out
}

fn collect_classes(html: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = html.as_bytes();
    let mut i = 0usize;
    while i + 6 < bytes.len() {
        if !bytes[i..].starts_with(b"class=") {
            i += 1;
            continue;
        }
        i += 6;
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        let chunk = &html[start..i.min(bytes.len())];
        for token in chunk.split_whitespace() {
            if !token.is_empty() {
                out.insert(token.to_string());
            }
        }
        i += 1;
    }
    out
}

fn generate_css(classes: &BTreeSet<String>) -> String {
    let mut rules = Vec::new();
    for class in classes {
        if let Some(rule) = class_to_rule(class) {
            rules.push(rule);
        }
    }
    rules.join("")
}

fn class_to_rule(class: &str) -> Option<String> {
    let mut parts: Vec<&str> = class.split(':').collect();
    if parts.is_empty() {
        return None;
    }
    let base = parts.pop()?.to_string();
    let mut selector = format!(".{}", escape_selector(class));
    let mut media = None;
    for variant in &parts {
        match *variant {
            "hover" => selector.push_str(":hover"),
            "focus" => selector.push_str(":focus"),
            "active" => selector.push_str(":active"),
            "dark" => selector = format!(".dark {}", selector),
            "sm" => media = Some("(min-width: 640px)"),
            "md" => media = Some("(min-width: 768px)"),
            "lg" => media = Some("(min-width: 1024px)"),
            "xl" => media = Some("(min-width: 1280px)"),
            _ => {}
        }
    }
    if let Some(token) = base.strip_prefix("space-y-") {
        let v = spacing_value(token)?;
        let rule = format!(
            "{} > :not([hidden]) ~ :not([hidden]){{--tw-space-y-reverse:0;margin-top:calc({} * calc(1 - var(--tw-space-y-reverse)));margin-bottom:calc({} * var(--tw-space-y-reverse));}}",
            selector, v, v
        );
        if let Some(query) = media {
            return Some(format!("@media {}{{{}}}", query, rule));
        }
        return Some(rule);
    }
    let decl = utility_decl(&base)?;
    let rule = format!("{}{{{}}}", selector, decl);
    if let Some(query) = media {
        Some(format!("@media {}{{{}}}", query, rule))
    } else {
        Some(rule)
    }
}

fn utility_decl(base: &str) -> Option<String> {
    let spacing = spacing_scale();
    let colors = color_scale();
    if let Some(value) = colors.get(base).copied() {
        return Some(value.to_string());
    }
    if let Some(value) = spacing.get(base).copied() {
        return Some(value.to_string());
    }
    match base {
        "block" => Some("display:block;".to_string()),
        "inline-block" => Some("display:inline-block;".to_string()),
        "flex" => Some("display:flex;".to_string()),
        "grid" => Some("display:grid;".to_string()),
        "hidden" => Some("display:none;".to_string()),
        "items-center" => Some("align-items:center;".to_string()),
        "items-start" => Some("align-items:flex-start;".to_string()),
        "items-end" => Some("align-items:flex-end;".to_string()),
        "justify-center" => Some("justify-content:center;".to_string()),
        "justify-between" => Some("justify-content:space-between;".to_string()),
        "justify-start" => Some("justify-content:flex-start;".to_string()),
        "justify-end" => Some("justify-content:flex-end;".to_string()),
        "flex-wrap" => Some("flex-wrap:wrap;".to_string()),
        "w-full" => Some("width:100%;".to_string()),
        "w-9" => Some("width:2.25rem;".to_string()),
        "h-full" => Some("height:100%;".to_string()),
        "h-9" => Some("height:2.25rem;".to_string()),
        "h-16" => Some("height:4rem;".to_string()),
        "min-h-screen" => Some("min-height:100vh;".to_string()),
        "mx-auto" => Some("margin-left:auto;margin-right:auto;".to_string()),
        "uppercase" => Some("text-transform:uppercase;".to_string()),
        "font-semibold" => Some("font-weight:600;".to_string()),
        "font-bold" => Some("font-weight:700;".to_string()),
        "font-mono" => Some("font-family:ui-monospace,SFMono-Regular,Menlo,monospace;".to_string()),
        "text-xs" => Some("font-size:0.75rem;line-height:1rem;".to_string()),
        "text-sm" => Some("font-size:0.875rem;line-height:1.25rem;".to_string()),
        "text-base" => Some("font-size:1rem;line-height:1.5rem;".to_string()),
        "text-lg" => Some("font-size:1.125rem;line-height:1.75rem;".to_string()),
        "text-xl" => Some("font-size:1.25rem;line-height:1.75rem;".to_string()),
        "text-2xl" => Some("font-size:1.5rem;line-height:2rem;".to_string()),
        "text-4xl" => Some("font-size:2.25rem;line-height:2.5rem;".to_string()),
        "tracking-wide" => Some("letter-spacing:0.025em;".to_string()),
        "list-decimal" => Some("list-style-type:decimal;".to_string()),
        "list-disc" => Some("list-style-type:disc;".to_string()),
        "rounded" => Some("border-radius:0.25rem;".to_string()),
        "rounded-md" => Some("border-radius:0.375rem;".to_string()),
        "rounded-lg" => Some("border-radius:0.5rem;".to_string()),
        "rounded-xl" => Some("border-radius:0.75rem;".to_string()),
        "rounded-2xl" => Some("border-radius:1rem;".to_string()),
        "shadow-sm" => Some("box-shadow:0 1px 2px 0 rgba(0,0,0,0.05);".to_string()),
        "shadow-md" => Some("box-shadow:0 4px 6px -1px rgba(0,0,0,0.1),0 2px 4px -2px rgba(0,0,0,0.1);".to_string()),
        "border" => Some("border-width:1px;border-style:solid;".to_string()),
        "border-b" => Some("border-bottom-width:1px;border-bottom-style:solid;".to_string()),
        "border-t" => Some("border-top-width:1px;border-top-style:solid;".to_string()),
        "whitespace-pre-wrap" => Some("white-space:pre-wrap;".to_string()),
        "transition-shadow" => Some("transition-property:box-shadow;transition-duration:150ms;transition-timing-function:cubic-bezier(0.4,0,0.2,1);".to_string()),
        _ if base.starts_with("gap-") => spacing_value(base.strip_prefix("gap-")?).map(|v| format!("gap:{};", v)),
        _ if base.starts_with("max-w-") => max_width(base),
        _ if base.starts_with("grid-cols-") => grid_cols(base),
        _ => None,
    }
}

fn spacing_value(token: &str) -> Option<&'static str> {
    match token {
        "0" => Some("0rem"),
        "1" => Some("0.25rem"),
        "2" => Some("0.5rem"),
        "3" => Some("0.75rem"),
        "4" => Some("1rem"),
        "5" => Some("1.25rem"),
        "6" => Some("1.5rem"),
        "8" => Some("2rem"),
        "10" => Some("2.5rem"),
        "12" => Some("3rem"),
        _ => None,
    }
}

fn spacing_scale() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("p-0", "padding:0rem;"),
        ("p-2", "padding:0.5rem;"),
        ("p-3", "padding:0.75rem;"),
        ("p-4", "padding:1rem;"),
        ("p-5", "padding:1.25rem;"),
        ("p-6", "padding:1.5rem;"),
        ("p-8", "padding:2rem;"),
        ("p-10", "padding:2.5rem;"),
        ("px-3", "padding-left:0.75rem;padding-right:0.75rem;"),
        ("px-4", "padding-left:1rem;padding-right:1rem;"),
        ("px-5", "padding-left:1.25rem;padding-right:1.25rem;"),
        ("px-6", "padding-left:1.5rem;padding-right:1.5rem;"),
        ("py-1", "padding-top:0.25rem;padding-bottom:0.25rem;"),
        ("py-2", "padding-top:0.5rem;padding-bottom:0.5rem;"),
        ("py-3", "padding-top:0.75rem;padding-bottom:0.75rem;"),
        ("py-6", "padding-top:1.5rem;padding-bottom:1.5rem;"),
        ("py-10", "padding-top:2.5rem;padding-bottom:2.5rem;"),
        ("pt-2", "padding-top:0.5rem;"),
        ("pl-5", "padding-left:1.25rem;"),
        ("mt-1", "margin-top:0.25rem;"),
        ("mt-2", "margin-top:0.5rem;"),
        ("mt-3", "margin-top:0.75rem;"),
        ("mt-4", "margin-top:1rem;"),
        ("mt-5", "margin-top:1.25rem;"),
        ("mt-6", "margin-top:1.5rem;"),
        ("mt-8", "margin-top:2rem;"),
        ("mt-10", "margin-top:2.5rem;"),
        ("mt-12", "margin-top:3rem;"),
        ("mb-2", "margin-bottom:0.5rem;"),
        ("mx-auto", "margin-left:auto;margin-right:auto;"),
    ])
}

fn color_scale() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("bg-white", "background-color:#ffffff;"),
        ("bg-gray-50", "background-color:#f9fafb;"),
        ("bg-gray-900", "background-color:#111827;"),
        ("bg-blue-50", "background-color:#eff6ff;"),
        ("bg-blue-600", "background-color:#2563eb;"),
        ("bg-blue-700", "background-color:#1d4ed8;"),
        ("bg-amber-50", "background-color:#fffbeb;"),
        ("bg-emerald-50", "background-color:#ecfdf5;"),
        ("bg-emerald-600", "background-color:#059669;"),
        ("bg-emerald-700", "background-color:#047857;"),
        ("bg-indigo-600", "background-color:#4f46e5;"),
        ("bg-indigo-700", "background-color:#4338ca;"),
        ("text-white", "color:#ffffff;"),
        ("text-gray-100", "color:#f3f4f6;"),
        ("text-gray-400", "color:#9ca3af;"),
        ("text-gray-500", "color:#6b7280;"),
        ("text-gray-600", "color:#4b5563;"),
        ("text-gray-700", "color:#374151;"),
        ("text-gray-800", "color:#1f2937;"),
        ("text-gray-900", "color:#111827;"),
        ("text-blue-600", "color:#2563eb;"),
        ("text-blue-700", "color:#1d4ed8;"),
        ("text-blue-800", "color:#1e40af;"),
        ("text-blue-900", "color:#1e3a8a;"),
        ("text-amber-700", "color:#b45309;"),
        ("text-amber-800", "color:#92400e;"),
        ("text-emerald-700", "color:#047857;"),
        ("border-gray-300", "border-color:#d1d5db;"),
        ("border-gray-200", "border-color:#e5e7eb;"),
        ("border-amber-200", "border-color:#fde68a;"),
        ("border-blue-100", "border-color:#dbeafe;"),
        ("border-blue-200", "border-color:#bfdbfe;"),
        ("border-emerald-200", "border-color:#a7f3d0;"),
    ])
}

fn max_width(base: &str) -> Option<String> {
    match base {
        "max-w-6xl" => Some("max-width:72rem;".to_string()),
        _ => None,
    }
}

fn grid_cols(base: &str) -> Option<String> {
    if let Some(n) = base.strip_prefix("grid-cols-") {
        if let Ok(num) = n.parse::<u8>() {
            if num > 0 {
                return Some(format!(
                    "grid-template-columns:repeat({},minmax(0,1fr));",
                    num
                ));
            }
        }
        if n.starts_with('[') && n.ends_with(']') {
            let inner = &n[1..n.len().saturating_sub(1)];
            let value = inner.replace('_', " ");
            if !value.is_empty() {
                return Some(format!("grid-template-columns:{};", value));
            }
        }
    }
    None
}

fn escape_selector(class: &str) -> String {
    let mut out = String::with_capacity(class.len() + 8);
    for ch in class.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('\\');
            out.push(ch);
        }
    }
    out
}

fn write_cache(classes: &BTreeSet<String>, preflight: &str, css: &str) {
    let Some(root) = find_project_root() else {
        return;
    };
    let cache_dir = root.join(".cache").join("utility-css");
    if fs::create_dir_all(&cache_dir).is_err() {
        return;
    }
    let mut hasher = DefaultHasher::new();
    classes.hash(&mut hasher);
    preflight.hash(&mut hasher);
    let hash = hasher.finish();
    let path = cache_dir.join(format!("{:016x}.css", hash));
    let mut out = String::with_capacity(preflight.len() + css.len());
    out.push_str(preflight);
    out.push_str(css);
    let _ = fs::write(path, out);
}

fn find_project_root() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("PHPX_MODULE_ROOT") {
        let path = PathBuf::from(root);
        if path.join("deka.lock").exists() {
            return Some(path);
        }
    }
    let handler = std::env::var("HANDLER_PATH").ok()?;
    let mut current = Path::new(&handler).to_path_buf();
    if current.is_file() {
        current.pop();
    }
    for ancestor in current.ancestors() {
        if ancestor.join("deka.lock").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn load_config() -> UtilityCssConfig {
    let Some(root) = find_project_root() else {
        return UtilityCssConfig {
            enabled: true,
            preflight: DEFAULT_PREFLIGHT,
        };
    };
    let path = root.join("deka.css.json");
    let Ok(raw) = fs::read_to_string(path) else {
        return UtilityCssConfig {
            enabled: true,
            preflight: DEFAULT_PREFLIGHT,
        };
    };
    let Ok(json) = serde_json::from_str::<Value>(&raw) else {
        return UtilityCssConfig {
            enabled: true,
            preflight: DEFAULT_PREFLIGHT,
        };
    };
    let enabled = json
        .get("utility")
        .and_then(|v| v.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let preflight = json
        .get("utility")
        .and_then(|v| v.get("preflight"))
        .and_then(Value::as_bool)
        .unwrap_or(DEFAULT_PREFLIGHT);
    UtilityCssConfig { enabled, preflight }
}

fn preflight_css() -> &'static str {
    "*,*::before,*::after{box-sizing:border-box;border:0 solid #e5e7eb;}html,body{margin:0;padding:0;}html{line-height:1.5;-webkit-text-size-adjust:100%;font-family:ui-sans-serif,system-ui,sans-serif;}body{line-height:inherit;}h1,h2,h3,h4,h5,h6,p,figure,blockquote,dl,dd{margin:0;}ul,ol{margin:0;padding:0;list-style:none;}a{color:inherit;text-decoration:inherit;}img,svg,video,canvas{display:block;max-width:100%;height:auto;}button,input,select,textarea{font:inherit;color:inherit;background-color:transparent;}button,[type='button'],[type='reset'],[type='submit']{appearance:button;cursor:pointer;}"
}

#[cfg(test)]
mod tests {
    use super::{
        UtilityCssConfig, collect_classes, inject_utility_css, inject_utility_css_with_config,
    };

    #[test]
    fn injects_style_for_basic_classes() {
        let html = "<html><head></head><body><div class=\"bg-white text-gray-900 p-4\"></div></body></html>";
        let out = inject_utility_css(html);
        assert!(out.contains("__deka_utility_css"));
        assert!(out.contains(".bg-white{background-color:#ffffff;}"));
        assert!(out.contains(".text-gray-900{color:#111827;}"));
        assert!(out.contains(".p-4{padding:1rem;}"));
    }

    #[test]
    fn supports_variants() {
        let html = "<html><head></head><body><a class=\"hover:text-blue-600 md:grid-cols-3\"></a></body></html>";
        let out = inject_utility_css(html);
        assert!(out.contains(".hover\\:text-blue-600:hover{color:#2563eb;}"));
        assert!(out.contains("@media (min-width: 768px){.md\\:grid-cols-3{grid-template-columns:repeat(3,minmax(0,1fr));}}"));
    }

    #[test]
    fn class_scanner_handles_quotes() {
        let html = "<div class='a b c'></div><span class=\"d e\"></span>";
        let classes = collect_classes(html);
        assert!(classes.contains("a"));
        assert!(classes.contains("e"));
    }

    #[test]
    fn preflight_is_optional() {
        let html = "<html><head></head><body><div class=\"p-4\"></div></body></html>";
        let no_preflight = inject_utility_css_with_config(
            html,
            UtilityCssConfig {
                enabled: true,
                preflight: false,
            },
        );
        assert!(!no_preflight.contains("box-sizing:border-box"));
        let with_preflight = inject_utility_css_with_config(
            html,
            UtilityCssConfig {
                enabled: true,
                preflight: true,
            },
        );
        assert!(with_preflight.contains("box-sizing:border-box"));
    }

    #[test]
    fn disabled_config_skips_injection() {
        let html = "<html><head></head><body><div class=\"p-4\"></div></body></html>";
        let out = inject_utility_css_with_config(
            html,
            UtilityCssConfig {
                enabled: false,
                preflight: true,
            },
        );
        assert_eq!(out, html);
    }
}
