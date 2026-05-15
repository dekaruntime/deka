use super::git::{git_resolve_commit, git_show_file, list_files_at_ref};
use super::types::{CapabilityReport, PublishPackageRequest};
use super::versioning::parse_semver;

pub(super) fn capability_report(
    repo_path: &std::path::Path,
    git_ref: &str,
    manifest: Option<&serde_json::Value>,
) -> Result<CapabilityReport, anyhow::Error> {
    let detected = detect_capabilities(repo_path, git_ref)?;
    let declared = declared_capabilities(manifest);
    let missing = detected
        .iter()
        .filter(|cap| !declared.iter().any(|d| d == *cap))
        .cloned()
        .collect::<Vec<_>>();

    Ok(CapabilityReport {
        detected,
        declared,
        missing,
    })
}

#[derive(Debug, Clone)]
pub(super) struct ParsedPackageName {
    scope: String,
    package: String,
}

#[derive(Debug, Clone)]
pub(super) struct ManifestIdentity {
    name: String,
    version: String,
    pub(super) raw: serde_json::Value,
}

pub(super) fn enforce_repo_identity(
    owner: &str,
    parsed_name: &ParsedPackageName,
    repo: &str,
) -> Result<(), anyhow::Error> {
    if parsed_name.scope != owner {
        anyhow::bail!(
            "publish identity mismatch: package scope `@{}` must match authenticated owner `@{}`. fix: use --name @{}{}{}",
            parsed_name.scope,
            owner,
            owner,
            "/",
            parsed_name.package
        );
    }
    if parsed_name.package != repo {
        anyhow::bail!(
            "publish identity mismatch: package `@{}/{}`
must publish from repo `{}` (received `{}`). fix: use --repo {}",
            parsed_name.scope,
            parsed_name.package,
            parsed_name.package,
            repo,
            parsed_name.package
        );
    }
    Ok(())
}

pub(super) fn enforce_release_tag_alignment(
    repo_path: &std::path::Path,
    git_ref: &str,
    version: &str,
) -> Result<(), anyhow::Error> {
    let expected_tag = format!("v{}", version);
    let tag_ref = format!("refs/tags/{}^{{commit}}", expected_tag);
    let tag_commit = git_resolve_commit(repo_path, &tag_ref)?;
    let Some(tag_commit) = tag_commit else {
        anyhow::bail!(
            "missing release tag `{}` for version {}. fix: git tag {} && git push origin {}",
            expected_tag,
            version,
            expected_tag,
            expected_tag
        );
    };

    let git_commit = git_resolve_commit(repo_path, git_ref)?;
    let Some(git_commit) = git_commit else {
        anyhow::bail!("Git ref does not resolve to a commit: {}", git_ref);
    };

    if git_commit != tag_commit {
        anyhow::bail!(
            "git ref/tag mismatch: ref `{}` -> {}, but `{}` -> {}. publish must target the tagged commit for version {}",
            git_ref,
            git_commit,
            expected_tag,
            tag_commit,
            version
        );
    }
    Ok(())
}

pub(super) fn resolved_manifest(
    repo_path: &std::path::Path,
    git_ref: &str,
    request_manifest: Option<&serde_json::Value>,
) -> Result<ManifestIdentity, anyhow::Error> {
    if let Some(raw) = request_manifest {
        if let Some(identity) = manifest_identity(raw.clone()) {
            return Ok(identity);
        }
    }

    let deka_raw = git_show_file(repo_path, git_ref, "deka.json")
        .map_err(|_| anyhow::anyhow!("missing or unreadable deka.json at `{}`", git_ref))?;
    let parsed = serde_json::from_str::<serde_json::Value>(&deka_raw)
        .map_err(|e| anyhow::anyhow!("invalid deka.json at `{}`: {}", git_ref, e))?;

    manifest_identity(parsed).ok_or_else(|| {
        anyhow::anyhow!(
            "deka.json missing `name` or `version` at `{}`. add both fields before publish",
            git_ref
        )
    })
}

fn manifest_identity(raw: serde_json::Value) -> Option<ManifestIdentity> {
    let name = raw.get("name")?.as_str()?.trim().to_string();
    let version = raw.get("version")?.as_str()?.trim().to_string();
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some(ManifestIdentity { name, version, raw })
}

pub(super) fn enforce_manifest_identity(
    manifest: &ManifestIdentity,
    req: &PublishPackageRequest,
) -> Result<(), anyhow::Error> {
    if manifest.name != req.name {
        anyhow::bail!(
            "manifest/package mismatch: deka.json name is `{}`, publish name is `{}`. fix: use --name {}",
            manifest.name,
            req.name,
            manifest.name
        );
    }
    if manifest.version != req.version {
        anyhow::bail!(
            "manifest/version mismatch: deka.json version is `{}`, publish version is `{}`. fix: use --pkg-version {}",
            manifest.version,
            req.version,
            manifest.version
        );
    }
    Ok(())
}

fn detect_capabilities(
    repo_path: &std::path::Path,
    git_ref: &str,
) -> Result<Vec<String>, anyhow::Error> {
    let files = list_files_at_ref(repo_path, git_ref)?;
    let mut detected = std::collections::BTreeSet::new();
    for file in files {
        if file.contains("/.git/") || file.contains("/node_modules/") {
            continue;
        }
        let source = git_show_file(repo_path, git_ref, &file)?;
        let lower = source.to_ascii_lowercase();
        if lower.contains("eval(")
            || lower.contains("new function(")
            || lower.contains("function(") && lower.contains("return await import(")
            || lower.contains("fetch(") && lower.contains("eval(")
        {
            detected.insert("dynamic".to_string());
        }
        if lower.contains("command::new(")
            || lower.contains("std::process::command")
            || lower.contains("shell_exec(")
            || lower.contains("proc_open(")
            || lower.contains("`")
        {
            detected.insert("run".to_string());
        }
    }
    Ok(detected.into_iter().collect())
}

pub(super) fn declared_capabilities(manifest: Option<&serde_json::Value>) -> Vec<String> {
    let Some(manifest) = manifest else {
        return Vec::new();
    };

    let allow = manifest.get("deka.security").and_then(|v| v.get("allow"));

    let mut declared = std::collections::BTreeSet::new();
    if allow
        .and_then(|v| v.get("dynamic"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        declared.insert("dynamic".to_string());
    }

    let run_declared = allow
        .and_then(|v| v.get("run"))
        .map(|run| match run {
            serde_json::Value::Bool(v) => *v,
            serde_json::Value::String(s) => !s.trim().is_empty(),
            serde_json::Value::Array(arr) => arr
                .iter()
                .any(|item| item.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false)),
            _ => false,
        })
        .unwrap_or(false);
    if run_declared {
        declared.insert("run".to_string());
    }

    declared.into_iter().collect()
}

pub(super) fn validate_package_name(name: &str) -> Result<ParsedPackageName, anyhow::Error> {
    if name.is_empty() || name.len() > 200 {
        anyhow::bail!("Invalid package name length");
    }

    if !name.starts_with('@') {
        anyhow::bail!("Package name must be scoped as @scope/name");
    }
    let mut parts = name.split('/');
    let scope_raw = parts.next().unwrap_or_default();
    let pkg = parts.next().unwrap_or_default();
    if parts.next().is_some() || scope_raw.len() <= 1 || pkg.is_empty() {
        anyhow::bail!("Package name must be in @scope/name format");
    }
    let scope = &scope_raw[1..];
    if !scope
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("Package scope contains invalid characters");
    }
    if !pkg
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("Package name contains invalid characters");
    }
    Ok(ParsedPackageName {
        scope: scope.to_string(),
        package: pkg.to_string(),
    })
}

pub(super) fn validate_version(version: &str) -> Result<(), anyhow::Error> {
    if version.is_empty() || version.len() > 64 {
        anyhow::bail!("Invalid version length");
    }
    parse_semver(version)?;
    Ok(())
}
