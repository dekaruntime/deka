use deno_core::{JsRuntime, ModuleCodeString, RuntimeOptions, serde_v8};
use serde_json::Value;
use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

const MARKER: &str = "__deka_utility_css";
const DEFAULT_PREFLIGHT: bool = true;

const UTILITY_CSS_BUNDLE: &str = include_str!("../../../assets/utility-css/bundle.js");
const UTILITY_CSS_REGISTRY: &str = include_str!("../../../assets/utility-css/registry.json");

#[derive(Clone, Copy, Debug)]
pub struct UtilityCssConfig {
    pub enabled: bool,
    pub preflight: bool,
}

/// Inject utility CSS into an HTML response.
///
/// The actual scanning/generation is performed by the shared TypeScript
/// implementation bundled at build time and executed inside a `deno_core`
/// isolate. This keeps the browser tour and the server runtime on a single
/// implementation and a single JSON registry.
pub fn inject_utility_css(html: &str) -> String {
    let config = load_config();
    if !config.enabled {
        return html.to_string();
    }
    inject_utility_css_with_config(html, config)
}

/// Generate a stylesheet for the given utility class names. Preflight is
/// included only when requested. Empty class lists yield empty CSS unless
/// preflight is on.
pub fn utility_css_for_classes(classes: &[String], preflight: bool) -> String {
    if classes.is_empty() && !preflight {
        return String::new();
    }
    let html = format!(
        "<html><head></head><body><div class=\"{}\"></div></body></html>",
        classes.join(" ")
    );
    let out = inject_utility_css_with_config(
        &html,
        UtilityCssConfig {
            enabled: true,
            preflight,
        },
    );
    extract_style_text(&out)
}

fn extract_style_text(html: &str) -> String {
    let Some(start_tag) = html.find("<style") else {
        return String::new();
    };
    let rest = &html[start_tag..];
    let Some(inner_start) = rest.find('>') else {
        return String::new();
    };
    let inner = &rest[inner_start + 1..];
    let Some(end) = inner.find("</style>") else {
        return inner.to_string();
    };
    inner[..end].to_string()
}

pub fn inject_utility_css_with_config(html: &str, config: UtilityCssConfig) -> String {
    if !config.enabled {
        return html.to_string();
    }
    if html.contains(MARKER) {
        return html.to_string();
    }

    let options = serde_json::json!({
        "enabled": true,
        "includePreflight": config.preflight,
    });

    match run_js_generator(html, options.to_string()) {
        Ok(out) => out,
        Err(err) => {
            tracing::warn!("utility-css generation failed: {}", err);
            html.to_string()
        }
    }
}

thread_local! {
    static JS_RUNTIME: RefCell<Option<JsRuntime>> = const { RefCell::new(None) };
}

fn with_runtime<F, R>(f: F) -> R
where
    F: FnOnce(&mut JsRuntime) -> R,
{
    JS_RUNTIME.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.is_none() {
            let mut runtime = JsRuntime::new(RuntimeOptions::default());
            // Load the bundled generator once per thread.
            let _ = runtime.execute_script(
                "utility-css-bundle.js",
                ModuleCodeString::from(UTILITY_CSS_BUNDLE.to_string()),
            );
            *opt = Some(runtime);
        }
        f(opt.as_mut().unwrap())
    })
}

fn run_js_generator(html: &str, options_json: String) -> Result<String, String> {
    with_runtime(|runtime| {
        let escaped_html = serde_json::to_string(html).map_err(|e| e.to_string())?;
        let escaped_registry = serde_json::to_string(UTILITY_CSS_REGISTRY).map_err(|e| e.to_string())?;
        let script = format!(
            "globalThis.__dekaGenerateUtilityCss({}, {}, {})",
            escaped_html, escaped_registry, options_json
        );

        let value = runtime
            .execute_script("utility-css-run.js", ModuleCodeString::from(script))
            .map_err(|e| e.to_string())?;

        deno_core::scope!(scope, runtime);
        let local = deno_core::v8::Local::new(scope, &value);
        serde_v8::from_v8::<String>(scope, local).map_err(|e| e.to_string())
    })
}

/// Collect class names from HTML. Kept in Rust for cheap tests/validation.
#[cfg(test)]
pub fn collect_classes(html: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
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

fn load_config() -> UtilityCssConfig {
    let path = config_path();
    if !path.exists() {
        return UtilityCssConfig {
            enabled: true,
            preflight: DEFAULT_PREFLIGHT,
        };
    }
    let contents = fs::read_to_string(&path).unwrap_or_default();
    parse_config(&contents)
}

fn config_path() -> PathBuf {
    std::env::var("DEKA_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("deka.css.json")
}

fn parse_config(contents: &str) -> UtilityCssConfig {
    let json: Value = serde_json::from_str(contents).unwrap_or(Value::Null);
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

#[cfg(test)]
mod tests {
    use super::{collect_classes, inject_utility_css, inject_utility_css_with_config, UtilityCssConfig};

    #[test]
    fn utility_css_for_classes_emits_rules() {
        let css = super::utility_css_for_classes(&["p-4".to_string(), "bg-white".to_string()], false);
        assert!(css.contains(".p-4{padding:1rem;}"), "{css}");
        assert!(css.contains(".bg-white{background-color:#ffffff;}"), "{css}");
        assert!(!css.contains("box-sizing:border-box"), "{css}");
    }

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
