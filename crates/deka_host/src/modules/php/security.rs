use super::*;

fn security_policy_from_env() -> SecurityPolicy {
    let raw = match std::env::var("DEKA_SECURITY_POLICY") {
        Ok(v) => v,
        Err(_) => return SecurityPolicy::default(),
    };
    let json = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(v) => v,
        Err(_) => return SecurityPolicy::default(),
    };
    let parsed = parse_deka_security_policy(&json);
    if parsed.has_errors() {
        SecurityPolicy::default()
    } else {
        parsed.policy
    }
}

fn rule_allows(capability: &str, rule: &RuleList, target: Option<&str>) -> bool {
    match rule {
        RuleList::None => false,
        RuleList::All => true,
        RuleList::List(items) => match target {
            Some(target) => items
                .iter()
                .any(|item| match_rule_item(capability, item, target)),
            None => false,
        },
    }
}

fn rule_denies(capability: &str, rule: &RuleList, target: Option<&str>) -> bool {
    match rule {
        RuleList::None => false,
        RuleList::All => true,
        RuleList::List(items) => match target {
            Some(target) => items
                .iter()
                .any(|item| match_rule_item(capability, item, target)),
            None => false,
        },
    }
}

fn match_rule_item(capability: &str, rule_item: &str, target: &str) -> bool {
    if rule_item == "*" {
        return true;
    }
    if matches!(capability, "read" | "write" | "wasm") {
        return path_matches(rule_item, target);
    }
    // Net rules support DNS-label wildcards: `*.squareup.com` matches
    // `connect.squareup.com` and `api.v2.squareup.com`, but not the
    // bare `squareup.com`. This mirrors the pattern shipped by every
    // other outbound allowlist (deno `--allow-net`, curl ACLs, cert
    // SANs). The exact-match fallback below still handles plain hosts
    // and `host:port` pairs.
    if capability == "net" && rule_item.starts_with("*.") {
        let suffix = &rule_item[1..]; // ".squareup.com"
        let t = target.to_ascii_lowercase();
        if t.ends_with(suffix) && t.len() > suffix.len() {
            return true;
        }
        return false;
    }
    rule_item == target
}

/// Public helper for @deka/http (and future outbound modules) to run
/// the standard `net` capability gate against a host. We keep the host
/// string lowercase and without port — the allowlist match handles
/// exact hosts, DNS wildcards, and `*`.
pub fn enforce_net_public(host: &str) -> Result<(), String> {
    match enforce_net(Some(host)) {
        Ok(()) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn path_matches(rule_item: &str, target: &str) -> bool {
    let target_path = normalize_path(target);
    let rule_path = normalize_path(rule_item);
    target_path.starts_with(&rule_path)
}

/// Resolve a path for policy comparison, resolving symlinks even when the
/// path does not exist yet.
///
/// `std::fs::canonicalize` fails on a path that is not on disk, and the old
/// fallback kept the raw string. That made grants depend on whether the target
/// already existed: on macOS `/tmp` is a symlink to `/private/tmp`, so the rule
/// `/tmp` canonicalized to `/private/tmp` while a not-yet-created
/// `/tmp/new.txt` stayed as written and failed the prefix test. The effect was
/// that a `write` grant let you **overwrite** a file but never **create** one
/// (deka#420). Any symlinked directory reproduces it, not just macOS `/tmp`.
///
/// Resolving the nearest existing ancestor and re-appending the remainder keeps
/// symlink resolution — which is what makes the prefix test meaningful — while
/// giving a not-yet-created path the same answer it will have a moment later.
fn normalize_path(value: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(value);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    };

    if let Ok(canonical) = std::fs::canonicalize(&resolved) {
        return canonical;
    }

    // Walk up to the nearest ancestor that exists, canonicalize that, then
    // re-append the components we walked past.
    let mut trailing: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = resolved.as_path();
    loop {
        match cursor.parent() {
            Some(parent) => {
                if let Some(name) = cursor.file_name() {
                    trailing.push(name.to_os_string());
                } else {
                    // A `..` or `.` component; nothing sensible to re-append.
                    return resolved;
                }
                if let Ok(canonical) = std::fs::canonicalize(parent) {
                    let mut out = canonical;
                    for name in trailing.iter().rev() {
                        out.push(name);
                    }
                    return out;
                }
                cursor = parent;
            }
            None => return resolved,
        }
    }
}

thread_local! {
    static SECURITY_PRIVILEGED: Cell<bool> = Cell::new(false);
    static SECURITY_PRIVILEGED_LABEL: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn set_security_privileged(enabled: bool, label: Option<String>) {
    SECURITY_PRIVILEGED.with(|flag| flag.set(enabled));
    SECURITY_PRIVILEGED_LABEL.with(|slot| {
        let mut guard = slot.borrow_mut();
        *guard = label;
    });
    let context = SECURITY_PRIVILEGED_LABEL.with(|slot| slot.borrow().clone());
    let context = context.as_deref().unwrap_or("unknown");
    if enabled {
        stdio::debug(
            "security",
            &format!("privileged context enabled ({})", context),
        );
    } else {
        stdio::debug(
            "security",
            &format!("privileged context disabled ({})", context),
        );
    }
}

fn security_privileged_enabled() -> bool {
    SECURITY_PRIVILEGED.with(|flag| flag.get())
}

fn is_internal_security_target(target: &str) -> bool {
    let mut normalized = target.replace('\\', "/");
    if normalized.ends_with('/') {
        normalized = normalized.trim_end_matches('/').to_string();
    }
    if normalized == "deka.lock" || normalized.ends_with("/deka.lock") {
        return true;
    }
    if normalized == "php_modules/.cache"
        || normalized.starts_with("php_modules/.cache/")
        || normalized.contains("/php_modules/.cache/")
        || normalized.ends_with("/php_modules/.cache")
    {
        return true;
    }
    if normalized == ".cache"
        || normalized.starts_with(".cache/")
        || normalized.contains("/.cache/")
        || normalized.ends_with("/.cache")
    {
        return true;
    }
    false
}

#[cfg(test)]
mod security_rule_tests {
    use super::{
        RuleList, classify_security_origin, is_internal_security_target, is_runtime_safe_env_key,
        match_rule_item, normalize_rel_like, prompt_scope_key, rule_allows, rule_denies,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        dir.push(format!("deka-security-test-{}", stamp));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_allows_prefix_path() {
        let root = temp_dir();
        let subdir = root.join("src");
        fs::create_dir_all(&subdir).unwrap();
        let file = subdir.join("main.phpx");
        fs::write(&file, "ok").unwrap();
        let rule = RuleList::List(vec![root.to_string_lossy().to_string()]);
        let target = file.to_string_lossy().to_string();
        assert!(rule_allows("read", &rule, Some(&target)));
    }

    #[test]
    fn write_denies_prefix_path() {
        let root = temp_dir();
        let subdir = root.join("php_modules/.cache");
        fs::create_dir_all(&subdir).unwrap();
        let file = subdir.join("out.php");
        fs::write(&file, "x").unwrap();
        let rule = RuleList::List(vec![subdir.to_string_lossy().to_string()]);
        let target = file.to_string_lossy().to_string();
        assert!(rule_denies("write", &rule, Some(&target)));
    }

    #[test]
    fn non_path_capability_requires_exact_match() {
        assert!(match_rule_item("env", "DATABASE_URL", "DATABASE_URL"));
        assert!(!match_rule_item("env", "DATABASE_URL", "PATH"));
    }

    #[test]
    fn internal_security_targets_match_expected_paths() {
        assert!(is_internal_security_target("deka.lock"));
        assert!(is_internal_security_target("/tmp/project/deka.lock"));
        assert!(is_internal_security_target(
            "php_modules/.cache/phpx/foo.php"
        ));
        assert!(is_internal_security_target(
            "/tmp/project/php_modules/.cache"
        ));
        assert!(is_internal_security_target(".cache/phpx/foo.php"));
        assert!(is_internal_security_target("/tmp/project/.cache"));
        assert!(!is_internal_security_target("/tmp/project/app/index.phpx"));
    }

    #[test]
    fn runtime_safe_env_keys_detected() {
        assert!(is_runtime_safe_env_key("PORT"));
        assert!(is_runtime_safe_env_key("deka_security_policy"));
        assert!(!is_runtime_safe_env_key("DATABASE_URL"));
    }

    #[test]
    fn prompt_scope_key_collapses_env_runtime_safe() {
        assert_eq!(prompt_scope_key("env", Some("PORT")), "env::runtime-safe");
        assert_eq!(
            prompt_scope_key("env", Some("DATABASE_URL")),
            "env::DATABASE_URL"
        );
    }

    #[test]
    fn relative_deps_classifies_as_third_party() {
        assert_eq!(
            classify_security_origin("read", Some("./deps/evil.txt")),
            "third-party"
        );
    }

    #[test]
    fn relative_app_classifies_as_project_owned() {
        assert_eq!(
            classify_security_origin("read", Some("./app/main.phpx")),
            "project-owned"
        );
    }

    #[test]
    fn normalize_rel_like_strips_dot_prefixes() {
        assert_eq!(
            normalize_rel_like("./deps/../deps/file.txt"),
            "deps/../deps/file.txt"
        );
        assert_eq!(normalize_rel_like("././app/main.phpx"), "app/main.phpx");
    }

    #[test]
    fn prompt_scope_key_collapses_read_paths_to_directory_rule() {
        let key = prompt_scope_key("read", Some("./deps/evil.txt"));
        assert_eq!(key, "read::./deps");
    }
}

fn enforce_scope(
    capability: &str,
    allow_rule: &RuleList,
    deny_rule: &RuleList,
    target: Option<&str>,
) -> Result<(), deno_core::error::CoreError> {
    if !security_enforcement_enabled() {
        return Ok(());
    }
    if rule_denies(capability, deny_rule, target) {
        return Err(core_err(format!(
            "SECURITY_POLICY_DENY_PRECEDENCE: capability={} target={} denied by policy",
            capability,
            target.unwrap_or("*")
        )));
    }
    if !rule_allows(capability, allow_rule, target) {
        if prompt_enabled() && prompt_grant(capability, target)? {
            return Ok(());
        }
        let origin = classify_security_origin(capability, target);
        let mut message = format!(
            "SECURITY_CAPABILITY_DENIED: capability={} target={} origin={} not allowed (re-run with explicit allow flag or configure security)",
            capability,
            target.unwrap_or("*"),
            origin
        );
        if let Some(hint) = config_hint_for_request(capability, target) {
            message.push_str(" Hint: ");
            message.push_str(&hint);
        }
        return Err(core_err(message));
    }
    Ok(())
}

fn security_enforcement_enabled() -> bool {
    true
}

fn prompt_enabled() -> bool {
    if std::env::var("DEKA_SECURITY_NO_PROMPT")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return false;
    }
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

fn prompt_grants() -> &'static Mutex<HashSet<String>> {
    static PROMPT_GRANTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    PROMPT_GRANTS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn prompt_grant(
    capability: &str,
    target: Option<&str>,
) -> Result<bool, deno_core::error::CoreError> {
    let key = prompt_scope_key(capability, target);
    let origin = classify_security_origin(capability, target);
    let target_label = target.unwrap_or("*");
    let safe_env_scope = capability == "env"
        && target
            .map(|value| is_runtime_safe_env_key(value))
            .unwrap_or(false);
    {
        let grants = prompt_grants()
            .lock()
            .map_err(|_| core_err("security prompt lock poisoned"))?;
        if grants.contains(&key) {
            return Ok(true);
        }
    }

    if let Some(hint) = config_hint_for_request(capability, target) {
        eprintln!("[security] hint: {}", hint);
    }
    if safe_env_scope {
        eprintln!(
            "[security] note: '{}' is treated as runtime-safe env scope; one decision applies to this process",
            target_label
        );
    }
    let prompt = format!(
        "[security] allow {} on {} (origin={}, scope={}) for this process? [y/N]: ",
        capability, target_label, origin, key
    );
    eprint!("{}", prompt);
    let _ = std::io::stderr().flush();

    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(_) => {
            let accepted = matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
            if accepted {
                let mut grants = prompt_grants()
                    .lock()
                    .map_err(|_| core_err("security prompt lock poisoned"))?;
                grants.insert(key);
                if let Err(err) = update_deka_json_allow(capability, target) {
                    eprintln!("[security] note: failed to update deka.json: {}", err);
                } else {
                    eprintln!(
                        "[security] updated deka.json to allow {} on {}",
                        capability,
                        target.unwrap_or("*")
                    );
                }
            }
            Ok(accepted)
        }
        Err(err) => Err(core_err(format!("security prompt failed: {}", err))),
    }
}

fn prompt_scope_key(capability: &str, target: Option<&str>) -> String {
    let Some(target) = target else {
        return format!("{}::*", capability);
    };
    let target = target.trim();
    if target.is_empty() {
        return format!("{}::*", capability);
    }
    let project_kind = project_kind();
    match capability {
        "read" | "write" => {
            let items = rule_items_for_request(capability, target, project_kind);
            if let Some(item) = items.first() {
                return format!("{}::{}", capability, item);
            }
        }
        "env" => {
            if is_runtime_safe_env_key(target) {
                return "env::runtime-safe".to_string();
            }
        }
        _ => {}
    }
    format!("{}::{}", capability, target)
}

fn is_runtime_safe_env_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_uppercase();
    matches!(
        normalized.as_str(),
        "PORT" | "PWD" | "TMPDIR" | "TEMP" | "TMP" | "HOME" | "PATH" | "DEKA_MODULE_ROOT"
    ) || normalized.starts_with("DEKA_")
}

fn classify_security_origin(capability: &str, target: Option<&str>) -> &'static str {
    let Some(raw_target) = target else {
        return "unknown";
    };
    let target = raw_target.trim();
    if target.is_empty() {
        return "unknown";
    }

    if (capability == "read" || capability == "write") && is_internal_security_target(target) {
        return "runtime-internal";
    }

    let path = std::path::Path::new(target);
    let root = project_root();
    if let Some(root) = root {
        let rel = if path.is_absolute() {
            path.strip_prefix(&root).ok().map(|p| p.to_path_buf())
        } else {
            Some(path.to_path_buf())
        };
        if let Some(rel) = rel {
            let rel_str = normalize_rel_like(&rel.to_string_lossy().replace('\\', "/"));
            if rel_str.starts_with("deps/")
                || rel_str.starts_with("vendor/")
                || rel_str.starts_with("node_modules/")
            {
                return "third-party";
            }
            if rel_str.starts_with("app/")
                || rel_str.starts_with("src/")
                || rel_str.starts_with("public/")
                || rel_str.starts_with("php_modules/")
                || rel_str == "deka.json"
                || rel_str == "deka.lock"
            {
                return "project-owned";
            }
        }
    }

    if path.is_absolute() {
        return "third-party";
    }
    "unknown"
}

fn config_hint_for_request(capability: &str, target: Option<&str>) -> Option<String> {
    let target = target?.trim();
    if target.is_empty() {
        return None;
    }
    let project_kind = project_kind();
    let suggestion = match capability {
        "read" => suggest_read_rule(target, project_kind),
        "write" => suggest_write_rule(target, project_kind),
        "net" => Some(format!("security.allow.net = [\"{}\"]", target)),
        "env" => Some(format!("security.allow.env = [\"{}\"]", target)),
        "run" => Some(format!("security.allow.run = [\"{}\"]", target)),
        "db" => Some(format!("security.allow.db = [\"{}\"]", target)),
        "wasm" => Some(format!("security.allow.wasm = [\"{}\"]", target)),
        _ => None,
    }?;

    let note = if is_common_target(target, project_kind, capability) {
        " (common for this project type)"
    } else {
        ""
    };
    let patch = patch_for_suggestion(capability, &suggestion)?;
    let path = format!("$.security.allow.{}", capability);
    let risk = risk_note_for_request(capability, &suggestion);
    Some(format!(
        "deka.json path: {} rule: {}{} risk: {} patch:\n{}",
        path, suggestion, note, risk, patch
    ))
}

fn risk_note_for_request(capability: &str, suggestion: &str) -> &'static str {
    let broad = suggestion.contains("\"./\"")
        || suggestion.contains("\"*\"")
        || suggestion.contains("\"./app\"")
        || suggestion.contains("\"./php_modules\"");
    match capability {
        "read" => {
            if broad {
                "broader read grants expose more local files; prefer the narrowest folder."
            } else {
                "read grants reveal file contents at allowed paths only."
            }
        }
        "write" => {
            if broad {
                "broader write grants permit file mutation; keep writable paths scoped."
            } else {
                "write grants can modify files in allowed paths; avoid source roots when possible."
            }
        }
        "env" => "env grants expose process secrets; allow specific keys only.",
        "net" => "net grants allow outbound/inbound network access to allowed targets.",
        "run" => "run grants allow subprocess execution; treat binaries as privileged.",
        "db" => "db grants allow database side effects on allowed drivers.",
        "wasm" => "wasm grants permit loading executable wasm modules.",
        _ => "this grant expands runtime capabilities; scope tightly.",
    }
}

fn suggest_read_rule(target: &str, project_kind: ProjectKind) -> Option<String> {
    let root = project_root()?;
    let path = std::path::Path::new(target);
    let rel = path.strip_prefix(&root).ok().unwrap_or(path);
    let rel_str = normalize_rel_like(&rel.to_string_lossy());
    if rel_str.starts_with("deps/") {
        return Some("security.allow.read = [\"./deps\"]".to_string());
    }
    if rel_str.starts_with("node_modules/") {
        return Some("security.allow.read = [\"./node_modules\"]".to_string());
    }
    if rel_str.starts_with("php_modules/.cache") {
        return Some("security.allow.read = [\"./php_modules/.cache\"]".to_string());
    }
    if rel_str.starts_with("php_modules/") {
        return Some("security.allow.read = [\"./php_modules\"]".to_string());
    }
    if rel_str.starts_with("src/") {
        return Some("security.allow.read = [\"./src\"]".to_string());
    }
    if rel_str == "deka.lock" {
        return Some("security.allow.read = [\"./deka.lock\"]".to_string());
    }
    if let Some(prefix) = rel.components().next().and_then(|c| c.as_os_str().to_str()) {
        return Some(format!("security.allow.read = [\"./{}\"]", prefix));
    }
    default_example("read", project_kind)
}

fn suggest_write_rule(target: &str, project_kind: ProjectKind) -> Option<String> {
    let root = project_root()?;
    let path = std::path::Path::new(target);
    let rel = path.strip_prefix(&root).ok().unwrap_or(path);
    let rel_str = normalize_rel_like(&rel.to_string_lossy());
    if rel_str.starts_with("php_modules/.cache") {
        return Some("security.allow.write = [\"./php_modules/.cache\"]".to_string());
    }
    if rel_str.starts_with("dist/") {
        return Some("security.allow.write = [\"./dist\"]".to_string());
    }
    if rel_str.starts_with("build/") {
        return Some("security.allow.write = [\"./build\"]".to_string());
    }
    if rel_str.starts_with(".cache/") {
        return Some("security.allow.write = [\"./.cache\"]".to_string());
    }
    if rel_str == "deka.lock" {
        return Some("security.allow.write = [\"./deka.lock\"]".to_string());
    }
    if let Some(prefix) = rel.components().next().and_then(|c| c.as_os_str().to_str()) {
        return Some(format!("security.allow.write = [\"./{}\"]", prefix));
    }
    default_example("write", project_kind)
}

fn project_root() -> Option<std::path::PathBuf> {
    if let Ok(root) = std::env::var("DEKA_MODULE_ROOT") {
        if !root.trim().is_empty() {
            return Some(std::path::PathBuf::from(root));
        }
    }
    if let Ok(handler) = std::env::var("HANDLER_PATH") {
        let path = std::path::PathBuf::from(handler);
        if path.is_file() {
            if let Some(parent) = path.parent() {
                return Some(parent.to_path_buf());
            }
        } else if let Some(parent) = path.parent() {
            return Some(parent.to_path_buf());
        }
    }
    std::env::current_dir().ok()
}

#[derive(Copy, Clone)]
pub(super) enum ProjectKind {
    Php,
    Js,
    Other,
}

fn project_kind() -> ProjectKind {
    if std::env::var("DEKA_MODULE_ROOT").is_ok() {
        return ProjectKind::Php;
    }
    if let Ok(handler) = std::env::var("HANDLER_PATH") {
        if handler.ends_with(".phpx") || handler.ends_with(".php") {
            return ProjectKind::Php;
        }
        if handler.ends_with(".ts")
            || handler.ends_with(".tsx")
            || handler.ends_with(".js")
            || handler.ends_with(".jsx")
        {
            return ProjectKind::Js;
        }
    }
    ProjectKind::Other
}

fn default_example(capability: &str, project_kind: ProjectKind) -> Option<String> {
    let example = match (project_kind, capability) {
        (ProjectKind::Php, "read") => "security.allow.read = [\"./php_modules\"]",
        (ProjectKind::Php, "write") => "security.allow.write = [\"./php_modules/.cache\"]",
        (ProjectKind::Js, "read") => "security.allow.read = [\"./src\"]",
        (ProjectKind::Js, "write") => "security.allow.write = [\"./.cache\"]",
        _ => return None,
    };
    Some(example.to_string())
}

fn is_common_target(target: &str, project_kind: ProjectKind, capability: &str) -> bool {
    let target = target.replace('\\', "/");
    match (project_kind, capability) {
        (ProjectKind::Php, "read") => {
            target.contains("/php_modules/") || target.ends_with("/deka.lock")
        }
        (ProjectKind::Php, "write") => {
            target.contains("/php_modules/.cache/") || target.ends_with("/deka.lock")
        }
        (ProjectKind::Js, "read") => {
            target.contains("/src/")
                || target.contains("/deps/")
                || target.contains("/node_modules/")
                || target.ends_with(".ts")
                || target.ends_with(".js")
        }
        (ProjectKind::Js, "write") => {
            target.contains("/.cache/")
                || target.ends_with(".cache")
                || target.contains("/dist/")
                || target.contains("/build/")
        }
        _ => false,
    }
}

fn patch_for_suggestion(capability: &str, suggestion: &str) -> Option<String> {
    let list_start = suggestion.find('[')?;
    let list_end = suggestion.rfind(']')?;
    let items = suggestion.get(list_start + 1..list_end)?.trim();
    let patch = format!(
        "{{\n  \"security\": {{\n    \"allow\": {{\n      \"{}\": [{}]\n    }}\n  }}\n}}",
        capability, items
    );
    Some(patch)
}

fn update_deka_json_allow(capability: &str, target: Option<&str>) -> Result<(), String> {
    let target = target
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| default_allow_target_for_capability(capability))
        .ok_or("missing target")?;
    if target.is_empty() {
        return Err("empty target".to_string());
    }
    let project_kind = project_kind();
    let root = project_root().ok_or("failed to determine project root")?;
    let path = root.join("deka.json");
    let mut doc = if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        serde_json::from_str::<Value>(&raw)
            .map_err(|err| format!("invalid JSON in {}: {}", path.display(), err))?
    } else {
        Value::Object(Map::new())
    };

    let root_obj = doc
        .as_object_mut()
        .ok_or("deka.json root must be an object")?;
    let security = root_obj
        .entry("security")
        .or_insert_with(|| Value::Object(Map::new()));
    let security_obj = security
        .as_object_mut()
        .ok_or("security must be an object")?;
    let allow = security_obj
        .entry("allow")
        .or_insert_with(|| Value::Object(Map::new()));
    let allow_obj = allow
        .as_object_mut()
        .ok_or("security.allow must be an object")?;

    let items = rule_items_for_request(capability, target, project_kind);
    if items.is_empty() {
        return Err("no allow items to apply".to_string());
    }

    let entry = allow_obj.entry(capability).or_insert(Value::Bool(false));
    if entry.as_bool() == Some(true) {
        return Ok(());
    }

    let mut list = Vec::new();
    match entry {
        Value::Bool(_) => {}
        Value::String(existing) => {
            list.push(existing.clone());
        }
        Value::Array(existing) => {
            for value in existing.iter() {
                if let Some(item) = value.as_str() {
                    if !item.trim().is_empty() {
                        list.push(item.to_string());
                    }
                }
            }
        }
        _ => {
            return Err(format!(
                "security.allow.{} must be boolean, string, or array",
                capability
            ));
        }
    }

    for item in items {
        if !list.iter().any(|existing| existing == &item) {
            list.push(item);
        }
    }

    *entry = Value::Array(list.into_iter().map(Value::String).collect());

    let payload = serde_json::to_string_pretty(&doc)
        .map_err(|err| format!("failed to serialize deka.json: {}", err))?;
    std::fs::write(&path, payload)
        .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    Ok(())
}

pub(super) fn default_allow_target_for_capability(capability: &str) -> Option<&'static str> {
    match capability {
        // Scope-less capability requests should persist as wildcard allows.
        "env" | "net" | "run" | "db" | "wasm" => Some("*"),
        _ => None,
    }
}

fn rule_items_for_request(
    capability: &str,
    target: &str,
    project_kind: ProjectKind,
) -> Vec<String> {
    match capability {
        "read" => rule_items_for_path(target, project_kind, true),
        "write" => rule_items_for_path(target, project_kind, false),
        "net" | "env" | "run" | "db" | "wasm" => vec![target.trim().to_string()],
        _ => Vec::new(),
    }
}

fn rule_items_for_path(target: &str, _project_kind: ProjectKind, is_read: bool) -> Vec<String> {
    let root = project_root();
    let path = std::path::Path::new(target);
    let rel = root
        .as_ref()
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    let rel_str = normalize_rel_like(&rel.to_string_lossy().replace('\\', "/"));

    let item = if rel_str.starts_with("php_modules/.cache") {
        "./php_modules/.cache".to_string()
    } else if rel_str.starts_with("php_modules/") {
        "./php_modules".to_string()
    } else if rel_str.starts_with("src/") {
        "./src".to_string()
    } else if rel_str.starts_with("deps/") {
        "./deps".to_string()
    } else if rel_str.starts_with("node_modules/") {
        "./node_modules".to_string()
    } else if !is_read && rel_str.starts_with("dist/") {
        "./dist".to_string()
    } else if !is_read && rel_str.starts_with("build/") {
        "./build".to_string()
    } else if rel_str == "deka.lock" {
        "./deka.lock".to_string()
    } else if let Some(prefix) = rel.components().next().and_then(|c| c.as_os_str().to_str()) {
        format!("./{}", prefix)
    } else {
        target.to_string()
    };

    vec![item]
}

fn normalize_rel_like(input: &str) -> String {
    let normalized = input.replace('\\', "/");
    let mut out = normalized.trim().to_string();
    while out.starts_with("./") {
        out = out[2..].to_string();
    }
    out
}

pub(super) fn enforce_read(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    if security_privileged_enabled() {
        match target {
            None => return Ok(()),
            Some(target) if is_internal_security_target(target) => return Ok(()),
            _ => {}
        }
    }
    enforce_scope("read", &policy.allow.read, &policy.deny.read, target)
}

pub(super) fn enforce_write(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    if security_privileged_enabled() {
        match target {
            None => return Ok(()),
            Some(target) if is_internal_security_target(target) => return Ok(()),
            _ => {}
        }
    }
    enforce_scope("write", &policy.allow.write, &policy.deny.write, target)
}

pub(super) fn enforce_net(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    enforce_scope("net", &policy.allow.net, &policy.deny.net, target)
}

pub(super) fn enforce_env(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    enforce_scope("env", &policy.allow.env, &policy.deny.env, target)
}

pub(super) fn enforce_db(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    enforce_scope("db", &policy.allow.db, &policy.deny.db, target)
}

pub(super) fn enforce_wasm(target: Option<&str>) -> Result<(), deno_core::error::CoreError> {
    let policy = security_policy_from_env();
    enforce_scope("wasm", &policy.allow.wasm, &policy.deny.wasm, target)
}


#[cfg(test)]
mod path_normalization_tests {
    use super::path_matches;

    /// deka#420: a `write` grant used to permit overwriting an existing file
    /// and refuse to create a new one, because only the existing path could be
    /// canonicalized through the symlink.
    #[test]
    fn a_grant_covers_a_path_that_does_not_exist_yet() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().to_str().expect("utf-8 temp dir");

        let existing = dir.path().join("already-there.txt");
        std::fs::write(&existing, b"x").expect("seed file");
        assert!(
            path_matches(root, existing.to_str().expect("utf-8")),
            "an existing file under the grant must match"
        );

        let missing = dir.path().join("not-yet.txt");
        assert!(
            path_matches(root, missing.to_str().expect("utf-8")),
            "a file that does not exist yet must match the same grant"
        );

        let nested = dir.path().join("deep").join("nested").join("new.txt");
        assert!(
            path_matches(root, nested.to_str().expect("utf-8")),
            "several missing components must still resolve to the grant"
        );
    }

    #[test]
    fn a_grant_does_not_leak_to_a_sibling_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let granted = dir.path().join("granted");
        let other = dir.path().join("other");
        std::fs::create_dir_all(&granted).expect("granted dir");
        std::fs::create_dir_all(&other).expect("other dir");

        assert!(!path_matches(
            granted.to_str().expect("utf-8"),
            other.join("new.txt").to_str().expect("utf-8")
        ));
        // Prefix-of-a-name, not a path component.
        let adjacent = dir.path().join("granted-extra");
        std::fs::create_dir_all(&adjacent).expect("adjacent dir");
        assert!(!path_matches(
            granted.to_str().expect("utf-8"),
            adjacent.join("new.txt").to_str().expect("utf-8")
        ));
    }
}
