//! Select the existing production permission profile without CLI overrides.
use permissions::permissions::{ExecutionPhase, parse_permissions};
use security::security_policy::{parse_deka_security_policy, policy_to_json};
use std::path::Path;

pub fn resolve(project: &Path) -> Result<String, String> {
    let manifest = project.join("deka.json");
    let document = if manifest.is_file() {
        let source = std::fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
        serde_json::from_str(&source).map_err(|e| format!("{}: {e}", manifest.display()))?
    } else {
        serde_json::json!({})
    };
    let parsed = parse_permissions(&document);
    if parsed.has_errors() {
        return Err(format!(
            "invalid native project permissions: {:?}",
            parsed.diagnostics
        ));
    }
    let policy = if let Some(permissions) = parsed.permissions {
        // APS 53: filesystem grants are relative to the process working directory.
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        permissions.resolve(ExecutionPhase::ProdRequest, &cwd)
    } else {
        let parsed = parse_deka_security_policy(&document);
        if parsed.has_errors() {
            return Err(format!(
                "invalid native project security policy: {:?}",
                parsed.diagnostics
            ));
        }
        parsed.policy
    };
    serde_json::to_string(&policy_to_json(&policy)).map_err(|e| e.to_string())
}
