use std::path::Path;

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
    path.to_ascii_lowercase().ends_with(".ds")
}

pub fn is_html_entry(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".html")
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
        assert!(!is_deka_entry("index.PHPX"));
        assert!(!is_deka_entry("index.html"));
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
}
