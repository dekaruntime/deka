use core::Context;
use core::ServeMode;
use runtime_core::security_policy::{
    RuleList, SecurityCliOverrides, merge_policy_with_cli_manifest_net_env,
    parse_deka_security_policy, policy_to_json,
};
use std::path::Path;

pub struct ResolvedSecurityPolicy {
    pub policy_json: String,
    pub prompt_enabled: bool,
    pub summary: String,
    pub warnings: Vec<String>,
}

pub fn resolve_security_policy(context: &Context) -> Result<ResolvedSecurityPolicy, String> {
    resolve_security_policy_for_root(
        &context.handler.resolved.directory,
        &context.args.flags,
        ProjectKind::from_mode(&context.handler.resolved.mode),
        context.args.flags.contains_key("--dev"),
    )
}

pub fn resolve_security_policy_for_root(
    root: &Path,
    flags: &std::collections::HashMap<String, bool>,
    project_kind: ProjectKind,
    dev: bool,
) -> Result<ResolvedSecurityPolicy, String> {
    let deka_json_path = root.join("deka.json");
    let document = if deka_json_path.is_file() {
        let raw = std::fs::read_to_string(&deka_json_path)
            .map_err(|err| format!("failed to read {}: {}", deka_json_path.display(), err))?;
        serde_json::from_str::<serde_json::Value>(&raw)
            .map_err(|err| format!("invalid JSON in {}: {}", deka_json_path.display(), err))?
    } else {
        serde_json::json!({})
    };

    let parsed = parse_deka_security_policy(&document);
    if parsed.has_errors() {
        let mut lines = Vec::new();
        for diag in parsed.diagnostics {
            if matches!(
                diag.level,
                runtime_core::security_policy::PolicyDiagnosticLevel::Error
            ) {
                lines.push(format!("{} at {}: {}", diag.code, diag.path, diag.message));
            }
        }
        return Err(format!("invalid security policy:\n{}", lines.join("\n")));
    }

    let warnings = parsed
        .diagnostics
        .iter()
        .filter(|diag| {
            matches!(
                diag.level,
                runtime_core::security_policy::PolicyDiagnosticLevel::Warning
            )
        })
        .map(|diag| format_warning(diag, project_kind))
        .collect::<Vec<_>>();

    let overrides = SecurityCliOverrides::from_flags(flags);
    let mut policy = parsed.policy;
    if dev {
        apply_dev_defaults(&mut policy, root);
    }
    let merged = merge_policy_with_cli_manifest_net_env(policy, &overrides);
    let policy_json = serde_json::to_string(&policy_to_json(&merged))
        .map_err(|err| format!("failed to serialize security policy: {}", err))?;
    let summary = format!(
        "default-deny; allow(run={}, dynamic={}, wasm={}) deny(run={}, dynamic={}, net={}) prompt={}",
        summarize_rule(&merged.allow.run),
        merged.allow.dynamic,
        summarize_rule(&merged.allow.wasm),
        summarize_rule(&merged.deny.run),
        merged.deny.dynamic,
        summarize_rule(&merged.deny.net),
        merged.prompt
    );

    Ok(ResolvedSecurityPolicy {
        policy_json,
        prompt_enabled: merged.prompt,
        summary,
        warnings,
    })
}

pub fn install_platform_security_for_root(
    root: &Path,
    flags: &std::collections::HashMap<String, bool>,
) -> Result<(), String> {
    let resolved_security = resolve_security_policy_for_root(root, flags, ProjectKind::Php, false)?;
    for warning in resolved_security.warnings {
        stdio::log("security", &format!("warning: {}", warning));
    }
    stdio::log("security", &resolved_security.summary);
    unsafe {
        std::env::set_var("DEKA_SECURITY_POLICY", &resolved_security.policy_json);
        std::env::set_var(
            "DEKA_SECURITY_NO_PROMPT",
            if resolved_security.prompt_enabled {
                "0"
            } else {
                "1"
            },
        );
    }
    Ok(())
}

fn apply_dev_defaults(
    policy: &mut runtime_core::security_policy::SecurityPolicy,
    root: &std::path::Path,
) {
    if matches!(policy.allow.read, RuleList::None) {
        policy.allow.read = RuleList::List(vec![root.to_string_lossy().to_string()]);
    }
    if matches!(policy.allow.write, RuleList::None) {
        let cache_dirs = vec![root.join(".cache"), root.join("php_modules").join(".cache")];
        let entries = cache_dirs
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();
        policy.allow.write = RuleList::List(entries);
    }
    if matches!(policy.allow.wasm, RuleList::None) {
        policy.allow.wasm = RuleList::All;
    }
}

fn summarize_rule(rule: &RuleList) -> String {
    match rule {
        RuleList::None => "none".to_string(),
        RuleList::All => "all".to_string(),
        RuleList::List(items) => {
            if items.len() <= 3 {
                items.join(",")
            } else {
                format!("{} items", items.len())
            }
        }
    }
}

#[derive(Copy, Clone)]
pub enum ProjectKind {
    Php,
    Js,
    Other,
}

impl ProjectKind {
    fn from_mode(mode: &ServeMode) -> Self {
        match mode {
            ServeMode::Php => Self::Php,
            ServeMode::Js => Self::Js,
            ServeMode::Static => Self::Other,
        }
    }
}

fn format_warning(
    diag: &runtime_core::security_policy::PolicyDiagnostic,
    project_kind: ProjectKind,
) -> String {
    let mut message = format!("{} at {}: {}", diag.code, diag.path, diag.message);
    if matches!(
        diag.code,
        "SECURITY_POLICY_BROAD_ALLOW" | "SECURITY_POLICY_WEAK_ALLOW"
    ) {
        if let Some(example) = example_for_warning(diag.path.as_str(), project_kind) {
            message.push(' ');
            message.push_str(&example);
        }
        if let Some(patch) = example_patch_for_warning(diag.path.as_str(), project_kind) {
            message.push_str("\n[security] patch:\n");
            message.push_str(&patch);
        }
    }
    message
}

fn example_for_warning(path: &str, project_kind: ProjectKind) -> Option<String> {
    let capability = if path.ends_with(".read") {
        "read"
    } else if path.ends_with(".write") {
        "write"
    } else if path.ends_with(".net") {
        "net"
    } else if path.ends_with(".env") {
        "env"
    } else if path.ends_with(".run") {
        "run"
    } else if path.ends_with(".db") {
        "db"
    } else if path.ends_with(".wasm") {
        "wasm"
    } else {
        return None;
    };

    let example = match (project_kind, capability) {
        (ProjectKind::Php, "read") => "security.allow.read = [\"./php_modules\"]",
        (ProjectKind::Php, "write") => "security.allow.write = [\"./php_modules/.cache\"]",
        (ProjectKind::Php, "wasm") => "security.allow.wasm = [\"module.wasm\"]",
        (ProjectKind::Php, "net") => "security.allow.net = [\"localhost:5432\"]",
        (ProjectKind::Php, "env") => "security.allow.env = [\"DATABASE_URL\"]",
        (ProjectKind::Php, "run") => "security.allow.run = [\"git\"]",
        (ProjectKind::Php, "db") => "security.allow.db = [\"postgres\"]",
        (ProjectKind::Js, "read") => "security.allow.read = [\"./src\"]",
        (ProjectKind::Js, "write") => "security.allow.write = [\"./.cache\"]",
        (ProjectKind::Js, "wasm") => "security.allow.wasm = [\"module.wasm\"]",
        (ProjectKind::Js, "net") => "security.allow.net = [\"localhost:3000\"]",
        (ProjectKind::Js, "env") => "security.allow.env = [\"API_KEY\"]",
        (ProjectKind::Js, "run") => "security.allow.run = [\"git\"]",
        (ProjectKind::Js, "db") => "security.allow.db = [\"postgres\"]",
        _ => return None,
    };

    let label = match project_kind {
        ProjectKind::Php => "Example (phpx):",
        ProjectKind::Js => "Example (js):",
        ProjectKind::Other => "Example:",
    };
    Some(format!("{} {}", label, example))
}

fn example_patch_for_warning(path: &str, project_kind: ProjectKind) -> Option<String> {
    let capability = if path.ends_with(".read") {
        "read"
    } else if path.ends_with(".write") {
        "write"
    } else if path.ends_with(".net") {
        "net"
    } else if path.ends_with(".env") {
        "env"
    } else if path.ends_with(".run") {
        "run"
    } else if path.ends_with(".db") {
        "db"
    } else if path.ends_with(".wasm") {
        "wasm"
    } else {
        return None;
    };

    let entries = match (project_kind, capability) {
        (ProjectKind::Php, "read") => vec!["./php_modules"],
        (ProjectKind::Php, "write") => vec!["./php_modules/.cache"],
        (ProjectKind::Php, "wasm") => vec!["module.wasm"],
        (ProjectKind::Php, "net") => vec!["localhost:5432"],
        (ProjectKind::Php, "env") => vec!["DATABASE_URL"],
        (ProjectKind::Php, "run") => vec!["git"],
        (ProjectKind::Php, "db") => vec!["postgres"],
        (ProjectKind::Js, "read") => vec!["./src"],
        (ProjectKind::Js, "write") => vec!["./.cache"],
        (ProjectKind::Js, "wasm") => vec!["module.wasm"],
        (ProjectKind::Js, "net") => vec!["localhost:3000"],
        (ProjectKind::Js, "env") => vec!["API_KEY"],
        (ProjectKind::Js, "run") => vec!["git"],
        (ProjectKind::Js, "db") => vec!["postgres"],
        _ => return None,
    };

    let items = entries
        .into_iter()
        .map(|item| format!("\"{}\"", item))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "{{\n  \"security\": {{\n    \"allow\": {{\n      \"{}\": [{}]\n    }}\n  }}\n}}",
        capability, items
    ))
}
