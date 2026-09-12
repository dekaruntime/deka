use serve::config::{ServeConfig, ServeMode, StaticServeConfig};
use std::path::{Path, PathBuf};

pub fn handler_input_with<Get>(positionals: &[String], env_get: &Get) -> (String, Vec<String>)
where
    Get: Fn(&str) -> Option<String>,
{
    let handler = positionals
        .first()
        .cloned()
        .or_else(|| env_get("HANDLER_PATH"))
        .unwrap_or_else(|| ".".to_string());
    let extra_args = if positionals.len() > 1 {
        positionals[1..].to_vec()
    } else {
        Vec::new()
    };
    (handler, extra_args)
}

pub fn normalize_handler_path(path: &str) -> String {
    normalize_handler_path_with(path, &|| std::env::current_dir().ok(), &|p| {
        p.canonicalize().ok()
    })
}

pub fn normalize_handler_path_with<Cwd, Canonicalize>(
    path: &str,
    cwd_get: &Cwd,
    canonicalize: &Canonicalize,
) -> String
where
    Cwd: Fn() -> Option<std::path::PathBuf>,
    Canonicalize: Fn(&Path) -> Option<std::path::PathBuf>,
{
    let path = Path::new(path);
    if path.is_absolute() {
        return canonicalize(path)
            .unwrap_or_else(|| path.to_path_buf())
            .to_string_lossy()
            .to_string();
    }
    let cwd = match cwd_get() {
        Some(dir) => dir,
        None => return path.to_string_lossy().to_string(),
    };
    let joined = cwd.join(path);
    match canonicalize(&joined) {
        Some(canon) => canon.to_string_lossy().to_string(),
        None => joined.to_string_lossy().to_string(),
    }
}

pub fn is_deka_entry(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".ds") || lower.ends_with(".dsx")
}

pub fn is_js_entry(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".js")
}

pub fn is_html_entry(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".html")
}

#[derive(Debug, Clone)]
pub struct ResolvedHandler {
    pub path: PathBuf,
    pub directory: PathBuf,
    pub mode: ServeMode,
    pub config: ServeConfig,
}

pub fn resolve_handler_path(path: &str) -> Result<ResolvedHandler, String> {
    let path = Path::new(path);
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let cwd = std::env::current_dir().map_err(|e| format!("Failed to get cwd: {}", e))?;
        cwd.join(path)
    };

    let abs_path = if abs_path.exists() {
        abs_path.canonicalize().unwrap_or(abs_path)
    } else {
        abs_path
    };

    let is_dir = abs_path.is_dir();
    let (handler_dir, serve_config) = if is_dir {
        let config = ServeConfig::load(&abs_path);
        (abs_path.clone(), config)
    } else if let Some(parent) = abs_path.parent() {
        let config = ServeConfig::load(parent);
        (parent.to_path_buf(), config)
    } else {
        (PathBuf::from("."), ServeConfig::default())
    };

    if !is_dir {
        let mode = serve_config
            .mode
            .clone()
            .unwrap_or_else(|| detect_mode(&abs_path));
        return Ok(ResolvedHandler {
            path: abs_path,
            directory: handler_dir,
            mode,
            config: serve_config,
        });
    }

    if let Some(ref entry) = serve_config.entry {
        let entry_path = if Path::new(entry).is_absolute() {
            PathBuf::from(entry)
        } else {
            handler_dir.join(entry)
        };

        if !entry_path.exists() {
            return Err(format!("Entry file not found: {}", entry_path.display()));
        }

        let mode = serve_config
            .mode
            .clone()
            .unwrap_or_else(|| detect_mode(&entry_path));
        return Ok(ResolvedHandler {
            path: entry_path,
            directory: handler_dir,
            mode,
            config: serve_config,
        });
    }

    // Convention: if an app/ folder exists, default to PHP app routing mode.
    let app_dir = abs_path.join("app");
    if app_dir.is_dir() {
        return Ok(ResolvedHandler {
            path: abs_path.clone(),
            directory: handler_dir,
            mode: serve_config.mode.clone().unwrap_or(ServeMode::Php),
            config: serve_config,
        });
    }

    let index_files = [
        "index.ds",
        "index.dsx",
        "index.js",
        "index.mjs",
        "index.php",
        "index.phpx",
        "index.html",
        "main.php",
        "main.phpx",
        "handler.php",
        "handler.phpx",
    ];

    for index_file in &index_files {
        let index_path = abs_path.join(index_file);
        if index_path.exists() {
            let mode = serve_config
                .mode
                .clone()
                .unwrap_or_else(|| detect_mode(&index_path));
            return Ok(ResolvedHandler {
                path: index_path,
                directory: handler_dir,
                mode,
                config: serve_config,
            });
        }
    }

    Ok(ResolvedHandler {
        path: abs_path,
        directory: handler_dir,
        mode: serve_config.mode.clone().unwrap_or(ServeMode::Static),
        config: serve_config,
    })
}

fn detect_mode(path: &Path) -> ServeMode {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext.to_ascii_lowercase().as_str() {
            "php" | "phpx" | "ds" | "dsx" => ServeMode::Php,
            "js" | "mjs" | "cjs" => ServeMode::Js,
            "html" | "htm" => ServeMode::Static,
            _ => ServeMode::Static,
        }
    } else {
        ServeMode::Static
    }
}

/// Handler and serving configuration captured before command dispatch.
#[derive(Debug, Clone)]
pub struct HandlerSnapshot {
    pub input: String,
    pub resolved: ResolvedHandler,
    pub static_config: StaticServeConfig,
    pub serve_config_path: Option<PathBuf>,
}

impl HandlerSnapshot {
    pub fn from_positionals(positionals: &[String]) -> Result<Self, String> {
        let (input, _) = handler_input_with(positionals, &|key| std::env::var(key).ok());

        let resolved = resolve_handler_path(&input)?;
        let static_config = StaticServeConfig::load(&resolved.directory);
        let serve_config_path = resolved.directory.join("serve.json");

        Ok(Self {
            input,
            resolved,
            static_config,
            serve_config_path: serve_config_path.exists().then_some(serve_config_path),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn handler_input_prefers_first_positional() {
        let env = HashMap::<String, String>::from([("HANDLER_PATH".into(), "env.ds".into())]);
        let env_get = |k: &str| env.get(k).cloned();
        let (handler, extra) = handler_input_with(&["main.ds".into(), "a".into()], &env_get);
        assert_eq!(handler, "main.ds");
        assert_eq!(extra, vec!["a".to_string()]);
    }

    #[test]
    fn handler_input_uses_env_then_default() {
        let env = HashMap::<String, String>::from([("HANDLER_PATH".into(), "env.ds".into())]);
        let env_get = |k: &str| env.get(k).cloned();
        let (handler, extra) = handler_input_with(&[], &env_get);
        assert_eq!(handler, "env.ds");
        assert!(extra.is_empty());

        let none_get = |_k: &str| None;
        let (handler2, extra2) = handler_input_with(&[], &none_get);
        assert_eq!(handler2, ".");
        assert!(extra2.is_empty());
    }

    #[test]
    fn dekascript_entry_detection_rejects_phpx_and_html() {
        assert!(is_deka_entry("index.DS"));
        assert!(is_deka_entry("page.dsx"));
        assert!(!is_deka_entry("index.DekaScript"));
        assert!(!is_deka_entry("index.html"));
        assert!(!is_deka_entry("handler.js"));
        assert!(is_js_entry("handler.js"));
        assert!(is_js_entry("handler.JS"));
        assert!(!is_js_entry("handler.ds"));
        assert!(is_html_entry("index.html"));
        assert!(is_html_entry("index.HTML"));
    }

    #[test]
    fn normalize_handler_path_with_uses_injected_closures() {
        let cwd = || Some(std::path::PathBuf::from("/tmp/project"));
        let canonicalize = |_path: &std::path::Path| None;
        let path = normalize_handler_path_with("main.ds", &cwd, &canonicalize);
        assert_eq!(path, "/tmp/project/main.ds");
    }

    #[test]
    fn normalize_handler_path_with_canonicalizes_existing_absolute_paths() {
        let cwd = || Some(std::path::PathBuf::from("/tmp/project"));
        let canonicalize = |path: &std::path::Path| {
            assert_eq!(path, std::path::Path::new("/tmp/project/entry.ds"));
            Some(std::path::PathBuf::from("/tmp/project/legacy.phpx"))
        };
        let path = normalize_handler_path_with("/tmp/project/entry.ds", &cwd, &canonicalize);
        assert_eq!(path, "/tmp/project/legacy.phpx");
    }

    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{}_{}", prefix, nonce));
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn explicit_file_path_overrides_serve_entry() {
        let dir = temp_dir("deka_handler_override");
        let explicit = dir.join("simple.phpx");
        let configured = dir.join("main.phpx");
        fs::write(&explicit, "<?php echo 'simple';").expect("write explicit");
        fs::write(&configured, "<?php echo 'main';").expect("write configured");
        fs::write(dir.join("serve.json"), r#"{"entry":"main.phpx"}"#).expect("write config");

        let resolved = resolve_handler_path(explicit.to_str().expect("path")).expect("resolve");
        let resolved_canon = resolved.path.canonicalize().expect("resolved canonicalize");
        let explicit_canon = explicit.canonicalize().expect("explicit canonicalize");
        assert_eq!(resolved_canon, explicit_canon);
    }

    #[test]
    fn directory_with_app_respects_serve_entry() {
        let dir = temp_dir("deka_handler_entry");
        let configured = dir.join("main.phpx");
        fs::write(&configured, "<?php echo 'main';").expect("write configured");
        let app_dir = dir.join("app");
        fs::create_dir_all(&app_dir).expect("mkdir app");
        fs::write(app_dir.join("page.phpx"), "<?php echo 'page';").expect("write page");
        fs::write(dir.join("serve.json"), r#"{"entry":"main.phpx"}"#).expect("write config");

        let resolved = resolve_handler_path(dir.to_str().expect("path")).expect("resolve");
        let resolved_canon = resolved.path.canonicalize().expect("resolved canonicalize");
        let configured_canon = configured.canonicalize().expect("configured canonicalize");
        assert_eq!(resolved_canon, configured_canon);
    }

    #[test]
    fn app_directory_defaults_to_php_mode() {
        let dir = temp_dir("deka_handler_app_router");
        let app_dir = dir.join("app");
        fs::create_dir_all(&app_dir).expect("mkdir app");
        fs::write(app_dir.join("page.phpx"), "<?php echo 'ok';").expect("write page");

        let resolved = resolve_handler_path(dir.to_str().expect("path")).expect("resolve");
        assert!(resolved.path.is_dir());
        assert!(matches!(resolved.mode, ServeMode::Php));
    }

    #[test]
    fn javascript_file_is_js_mode() {
        let dir = temp_dir("deka_handler_js_worker");
        let file = dir.join("handler.js");
        fs::write(
            &file,
            "export default { async fetch() { return new Response(\"ok\"); } }\n",
        )
        .expect("write js");

        let resolved = resolve_handler_path(file.to_str().expect("path")).expect("resolve");
        assert_eq!(
            resolved.path.canonicalize().expect("resolved"),
            file.canonicalize().expect("file")
        );
        assert!(matches!(resolved.mode, ServeMode::Js));
    }
}
