use anyhow::{bail, Context as AnyhowContext, Result};
use core::{CommandSpec, Context, FlagSpec, ParamSpec, Registry};
use deka_modules::modules::is_modules_dir_name;
use serde_json::json;
use std::io::{self, Write};
use std::process::Command;
use stdio;

use crate::cli::auth_store;

const COMMAND: CommandSpec = CommandSpec {
    owner: "",
    name: "publish",
    category: "package",
    summary: "publish a DekaScript package release to Linkhash",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_param(ParamSpec {
        name: "--name",
        description: "package name (@scope/name). default: from deka.json",
    });
    registry.add_param(ParamSpec {
        name: "--version",
        description: "package version (deprecated alias; use --pkg-version)",
    });
    registry.add_param(ParamSpec {
        name: "--pkg-version",
        description: "package version. default: from deka.json",
    });
    registry.add_param(ParamSpec {
        name: "--repo",
        description:
            "owner-qualified backing repository (owner/name). Must match deka.json repository",
    });
    registry.add_param(ParamSpec {
        name: "--git-ref",
        description: "git ref to publish (default: HEAD)",
    });
    registry.add_param(ParamSpec {
        name: "--token",
        description: "PAT token (fallback: auth profile)",
    });
    registry.add_param(ParamSpec {
        name: "--registry-url",
        description: "registry base URL (default: auth profile or http://localhost:9418)",
    });
    registry.add_param(ParamSpec {
        name: "--registry",
        description: "alias for --registry-url",
    });
    registry.add_param(ParamSpec {
        name: "--description",
        description: "package description. default: from deka.json",
    });
    registry.add_flag(FlagSpec {
        name: "--yes",
        aliases: &["-y"],
        description: "auto-apply publish guard-rail fixes",
    });
    registry.add_flag(FlagSpec {
        name: "--dry-run",
        aliases: &[],
        description: "run preflight check only; do not actually publish",
    });
}

pub fn cmd(context: &Context) {
    match build_request(context) {
        Ok(request) => {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            if let Err(err) = runtime.block_on(run_publish(request)) {
                stdio::error("publish", &err.to_string());
            }
        }
        Err(err) => stdio::error("publish", &err.to_string()),
    }
}

struct PublishRequest {
    endpoint: String,
    token: String,
    payload: serde_json::Value,
    dry_run: bool,
}

/// Resolve the publish token and registry URL from explicit channels only:
/// token = `--token` → auth profile → error; registry = `--registry-url` /
/// `--registry` → auth profile → `http://localhost:9418` (deka#801: no
/// environment fallbacks).
pub fn resolve_registry_auth(
    params: &std::collections::HashMap<String, String>,
    profile: Option<&crate::cli::auth_store::AuthProfile>,
) -> anyhow::Result<(String, String)> {
    let token = params
        .get("--token")
        .cloned()
        .or_else(|| profile.map(|p| p.token.clone()))
        .context("missing --token (or run `deka login`)")?;

    let registry = params
        .get("--registry-url")
        .or_else(|| params.get("--registry"))
        .cloned()
        .or_else(|| profile.map(|p| p.registry_url.clone()))
        .unwrap_or_else(|| "http://localhost:9418".to_string());

    Ok((token, registry))
}

fn build_request(context: &Context) -> Result<PublishRequest> {
    let params = &context.args.params;
    let profile = auth_store::load().ok().flatten();
    let local_manifest = load_local_deka_manifest()?;
    let dry_run = context.args.flags.contains_key("--dry-run");

    // Name: --name flag, or deka.json name
    let mut name = if let Some(n) = params.get("--name").cloned() {
        n
    } else {
        let manifest = &local_manifest;
        manifest.name.clone()
    };
    validate_scoped_package_name(&name)?;

    // Version: --pkg-version or --version flag, or deka.json version
    let mut version = if let Some(v) = params
        .get("--pkg-version")
        .or_else(|| params.get("--version"))
        .cloned()
    {
        v
    } else {
        let manifest = &local_manifest;
        manifest.version.clone()
    };

    // Repository identity is deliberately not inferred from package scope. A
    // package scope and a Git owner are separate authorities.
    let manifest_repo = local_manifest.repository.as_deref().context(
        "deka.json must declare an owner-qualified \"repository\" (for example \"deka/tcp\")",
    )?;
    validate_owner_qualified_repo(manifest_repo)?;
    let repo = if let Some(explicit_repo) = params.get("--repo") {
        validate_owner_qualified_repo(explicit_repo)?;
        if explicit_repo != manifest_repo {
            bail!(
                "--repo `{}` does not match deka.json repository `{}`; update the manifest instead of rewriting package identity during publish",
                explicit_repo,
                manifest_repo
            );
        }
        explicit_repo.clone()
    } else {
        manifest_repo.to_string()
    };

    let mut git_ref = params
        .get("--git-ref")
        .cloned()
        .unwrap_or_else(|| "HEAD".to_string());

    let (token, registry) = resolve_registry_auth(params, profile.as_ref())?;

    // Description: --description flag, or deka.json description
    let description = params
        .get("--description")
        .cloned()
        .or_else(|| local_manifest.description.clone())
        .unwrap_or_else(|| "Published with deka publish".to_string());

    let auto_apply =
        context.args.flags.contains_key("--yes") || context.args.flags.contains_key("-y");
    let mut planned_fixes: Vec<String> = Vec::new();

    // Align with deka.json if present and flags were explicitly provided
    {
        let manifest = &local_manifest;
        if params.contains_key("--name") && manifest.name != name {
            planned_fixes.push(format!(
                "align --name from `{}` to deka.json name `{}`",
                name, manifest.name
            ));
            name = manifest.name.clone();
        }
        if (params.contains_key("--pkg-version") || params.contains_key("--version"))
            && manifest.version != version
        {
            planned_fixes.push(format!(
                "align --pkg-version from `{}` to deka.json version `{}`",
                version, manifest.version
            ));
            version = manifest.version.clone();
        }
    }

    let expected_tag = format!("v{}", version);
    if git_ref != expected_tag {
        planned_fixes.push(format!(
            "align --git-ref from `{}` to `{}`",
            git_ref, expected_tag
        ));
        git_ref = expected_tag.clone();
    }

    if !planned_fixes.is_empty() {
        stdio::warn_simple("publish guard rails detected fixable mismatches:");
        for fix in &planned_fixes {
            stdio::warn_simple(&format!("  - {}", fix));
        }

        let apply = if auto_apply || dry_run {
            true
        } else {
            prompt_yes_no("Apply these fixes now? [Y/n]: ", true).unwrap_or(false)
        };

        if !apply {
            stdio::warn_simple(
                "continuing without auto-fixes; publish may be rejected by registry guard rails",
            );
        }
    }

    validate_scoped_package_name(&name)?;

    // Linkhash publishes a git tree. A vendored php_modules directory in that
    // tree would be copied by consumers and recreate nested dependency trees.
    // Releases contain source and the dependencies declared by deka.json only.
    reject_publish_tree_php_modules("HEAD")?;
    // macOS AppleDouble sidecar files (`._<name>`) must never ship: they poison
    // every consumer's integrity walk (dekaruntime/deka#587).
    reject_publish_tree_appledouble("HEAD")?;

    let peeled_commit = prepare_release_tag(&expected_tag, &version, dry_run)?;

    let endpoint = format!("{}/api/packages/publish", registry.trim_end_matches('/'));
    let mut payload = json!({
        "name": name,
        "version": version,
        "repo": repo,
        "git_ref": peeled_commit,
        "description": description,
    });
    payload["manifest"] = local_manifest.raw;

    Ok(PublishRequest {
        endpoint,
        token,
        payload,
        dry_run,
    })
}

fn reject_publish_tree_php_modules(git_ref: &str) -> Result<()> {
    reject_publish_tree_php_modules_at(None, git_ref)
}

fn reject_publish_tree_appledouble(git_ref: &str) -> Result<()> {
    reject_publish_tree_appledouble_at(None, git_ref)
}

/// Reject the publish tree if it contains a macOS AppleDouble sidecar file
/// (`._<name>` sibling). These are filesystem metadata, not source; a release
/// that ships them breaks integrity computation for every consumer on every OS
/// (dekaruntime/deka#587).
fn reject_publish_tree_appledouble_at(
    repo: Option<&std::path::Path>,
    git_ref: &str,
) -> Result<()> {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.current_dir(repo);
    }
    let output = command
        .args(["ls-tree", "-r", "-z", git_ref])
        .output()
        .with_context(|| format!("failed to inspect publish tree {}", git_ref))?;
    if !output.status.success() {
        bail!("failed to inspect publish tree {}", git_ref);
    }
    if let Some(path) = appledouble_path(&output.stdout) {
        bail!(
            "publish rejected: git tree contains `{}`, a macOS AppleDouble (`._*`) file. Remove it from the source tree (it is filesystem metadata, not source) and commit the cleanup before publishing",
            path
        );
    }
    Ok(())
}

fn appledouble_path(entries: &[u8]) -> Option<String> {
    entries.split(|byte| *byte == 0).find_map(|entry| {
        let tab = entry.iter().position(|byte| *byte == b'\t')?;
        let path = &entry[tab + 1..];
        let path = std::str::from_utf8(path).ok()?;
        if path.split('/').any(|segment| segment.starts_with("._")) {
            Some(path.to_string())
        } else {
            None
        }
    })
}

/// Validate the actual Git artifact, not a working-tree approximation.  Git
/// records symlinks as blobs, so `ls-tree -d` is insufficient here.
fn reject_publish_tree_php_modules_at(repo: Option<&std::path::Path>, git_ref: &str) -> Result<()> {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.current_dir(repo);
    }
    let output = command
        .args(["ls-tree", "-r", "-z", git_ref])
        .output()
        .with_context(|| format!("failed to inspect publish tree {}", git_ref))?;
    if !output.status.success() {
        bail!("failed to inspect publish tree {}", git_ref);
    }
    if let Some(path) = vendored_php_modules_path(&output.stdout) {
        bail!(
            "publish rejected: git tree contains `{}`. Remove ds_modules/ (or php_modules/) and declare dependencies in deka.json; releases never ship vendored dependencies",
            path
        );
    }
    Ok(())
}

fn vendored_php_modules_path(entries: &[u8]) -> Option<String> {
    entries.split(|byte| *byte == 0).find_map(|entry| {
        let tab = entry.iter().position(|byte| *byte == b'\t')?;
        let path = &entry[tab + 1..];
        let path = std::str::from_utf8(path).ok()?;
        // Artifact names must be portable to case-insensitive filesystems.
        // Also reject every symlink artifact: a link can turn an otherwise
        // harmless path into a php_modules directory after extraction.
        let mode = entry.split(|byte| *byte == b' ').next()?;
        let is_symlink = mode == b"120000";
        if is_symlink
            || path
                .split('/')
                .any(|segment| {
                    is_modules_dir_name(segment) || segment.eq_ignore_ascii_case("php_modules")
                })
        {
            Some(path.to_string())
        } else {
            None
        }
    })
}

async fn run_publish(request: PublishRequest) -> Result<()> {
    let requested_name = request
        .payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let requested_version = request
        .payload
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let client = reqwest::Client::new();
    let preflight_endpoint = request
        .endpoint
        .replace("/api/packages/publish", "/api/packages/preflight");
    let preflight_response = client
        .post(&preflight_endpoint)
        .bearer_auth(&request.token)
        .json(&request.payload)
        .send()
        .await
        .with_context(|| format!("failed to send preflight request to {}", preflight_endpoint))?;

    let preflight_status = preflight_response.status();
    let preflight_payload: serde_json::Value = preflight_response
        .json()
        .await
        .context("failed to parse preflight response json")?;
    if !preflight_status.is_success() {
        let err_msg = preflight_payload
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| preflight_payload.to_string());
        bail!(
            "publish preflight failed ({}): {}",
            preflight_status,
            err_msg
        );
    }

    if let Some(preflight) = preflight_payload.get("preflight") {
        let required = preflight
            .get("required_bump")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let minimum = preflight
            .get("minimum_allowed_version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        stdio::log(
            "publish",
            &format!("preflight: required bump={}, minimum={}", required, minimum),
        );
        if let Some(reasons) = preflight.get("reasons").and_then(|v| v.as_array()) {
            for reason in reasons.iter().filter_map(|v| v.as_str()) {
                stdio::log("publish", &format!("reason: {}", reason));
            }
        }
        if let Some(caps) = preflight.get("capabilities") {
            if let Some(detected) = caps.get("detected").and_then(|v| v.as_array()) {
                for cap in detected.iter().filter_map(|v| v.as_str()) {
                    stdio::log("publish", &format!("capability detected: {}", cap));
                }
            }
            if let Some(missing) = caps.get("missing").and_then(|v| v.as_array()) {
                for cap in missing.iter().filter_map(|v| v.as_str()) {
                    stdio::warn_simple(&format!(
                        "missing capability declaration in manifest: {}",
                        cap
                    ));
                }
            }
        }
        if let Some(issues) = preflight.get("issues").and_then(|v| v.as_array()) {
            for issue in issues {
                let code = issue
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("API_ISSUE");
                let symbol = issue
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<unknown>");
                let message = issue
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("api issue");
                let old_source = issue
                    .get("old_source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                let new_source = issue
                    .get("new_source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-");
                stdio::warn_simple(&format!(
                    "{} {}: {} (old: {}, new: {})",
                    code, symbol, message, old_source, new_source
                ));
            }
        }
        let allowed = preflight
            .get("allowed")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !allowed {
            bail!(
                "publish blocked by semver gate: requested version is below required minimum {}",
                minimum
            );
        }
    }

    // If dry-run, stop after preflight
    if request.dry_run {
        stdio::log(
            "publish",
            &format!(
                "dry-run complete for {}@{} (no changes made)",
                requested_name, requested_version
            ),
        );
        return Ok(());
    }

    let response = client
        .post(&request.endpoint)
        .bearer_auth(&request.token)
        .json(&request.payload)
        .send()
        .await
        .with_context(|| format!("failed to send publish request to {}", request.endpoint))?;

    let status = response.status();
    let payload: serde_json::Value = response
        .json()
        .await
        .context("failed to parse publish response json")?;

    if !status.is_success() {
        let err_msg = payload
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| payload.to_string());
        bail!("publish failed ({}): {}", status, err_msg);
    }

    // Registry may return the release object directly or nested under "release".
    let release = payload
        .get("release")
        .cloned()
        .unwrap_or_else(|| payload.clone());

    let package = release
        .get("package_name")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let version = release
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");

    if !requested_name.is_empty() && package != requested_name {
        bail!(
            "publish response mismatch: requested package `{}`, got `{}`",
            requested_name,
            package
        );
    }
    if !requested_version.is_empty() && version != requested_version {
        bail!(
            "publish response mismatch: requested version `{}`, got `{}`",
            requested_version,
            version
        );
    }

    stdio::log("publish", &format!("Published {}@{}", package, version));
    Ok(())
}

fn validate_scoped_package_name(name: &str) -> Result<()> {
    if !name.starts_with('@') {
        bail!("package name must be scoped as @scope/name");
    }

    let mut parts = name.split('/');
    let scope = parts.next().unwrap_or("");
    let pkg = parts.next().unwrap_or("");
    if parts.next().is_some() || scope.len() <= 1 || pkg.is_empty() {
        bail!("package name must be in @scope/name format");
    }

    if !scope[1..]
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("scope contains invalid characters");
    }

    if !pkg
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("package name contains invalid characters");
    }

    Ok(())
}

fn validate_owner_qualified_repo(repository: &str) -> Result<()> {
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    if parts.next().is_some() || owner.is_empty() || name.is_empty() {
        bail!(
            "repository must be owner-qualified as owner/name (for example deka/tcp), got `{}`",
            repository
        );
    }
    for segment in [owner, name] {
        if !segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
        {
            bail!("repository contains invalid characters: `{}`", repository);
        }
    }
    Ok(())
}

#[derive(Clone)]
struct LocalManifest {
    name: String,
    version: String,
    description: Option<String>,
    repository: Option<String>,
    raw: serde_json::Value,
}

fn load_local_deka_manifest() -> Result<LocalManifest> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let raw = std::fs::read_to_string(cwd.join("deka.json"))
        .context("deka publish requires a deka.json manifest")?;
    let json =
        serde_json::from_str::<serde_json::Value>(&raw).context("deka.json is not valid JSON")?;
    if json.get("deka.security").is_some() {
        bail!(
            r#"legacy manifest key `deka.security` is not supported; replace `deka.security.allow.*` with `security.allow.*` (JSON: rename top-level key "deka.security" to "security")"#
        );
    }
    let name = json
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("deka.json must contain a non-empty \"name\"")?
        .to_string();
    let version = json
        .get("version")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("deka.json must contain a non-empty \"version\"")?
        .to_string();
    let description = json
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string());
    let repository = json
        .get("repository")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string());
    Ok(LocalManifest {
        name,
        version,
        description,
        repository,
        raw: json,
    })
}

fn local_git_ref_exists(reference: &str) -> bool {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--verify")
        .arg(reference)
        .output();
    match output {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

fn prepare_release_tag(tag: &str, version: &str, dry_run: bool) -> Result<String> {
    ensure_clean_pushed_default_head()?;
    let head = git_output(&["rev-parse", "HEAD^{commit}"])?;

    if local_git_ref_exists(&format!("refs/tags/{}^{{commit}}", tag)) {
        let existing_commit = git_output(&["rev-parse", &format!("refs/tags/{tag}^{{commit}}")])?;
        if existing_commit != head {
            bail!(
                "release tag `{}` points at {} instead of current HEAD {}; choose a new version rather than rewriting an immutable release tag",
                tag,
                existing_commit,
                head
            );
        }
        let tag_type = git_output(&["cat-file", "-t", &format!("refs/tags/{tag}")])?;
        if tag_type != "tag" {
            bail!(
                "release tag `{}` must be annotated; delete the local lightweight tag and retry before it is published",
                tag
            );
        }
    } else if dry_run {
        bail!(
            "dry-run cannot preflight an unpublished release tag `{}`; create and push it with `deka publish --yes` first",
            tag
        );
    } else {
        let status = Command::new("git")
            .args(["tag", "-a", tag, "-m", &format!("Release {version}"), &head])
            .status()
            .with_context(|| format!("failed to create annotated release tag `{tag}`"))?;
        if !status.success() {
            bail!("failed to create annotated release tag `{tag}`");
        }
        stdio::log("publish", &format!("created annotated tag {tag}"));
    }

    if dry_run {
        return Ok(head);
    }

    let status = Command::new("git")
        .args([
            "push",
            "origin",
            &format!("refs/tags/{tag}:refs/tags/{tag}"),
        ])
        .status()
        .with_context(|| format!("failed to push release tag `{tag}` to origin"))?;
    if !status.success() {
        bail!(
            "failed to push release tag `{}`; source commits were not pushed. Push the tag explicitly with `git push origin refs/tags/{0}:refs/tags/{0}` and retry",
            tag
        );
    }
    stdio::log("publish", &format!("pushed release tag {tag}"));
    Ok(head)
}

fn ensure_clean_pushed_default_head() -> Result<()> {
    let status = git_output(&["status", "--porcelain"])?;
    if !status.is_empty() {
        bail!("publish requires a clean working tree; commit, stash, or remove local changes before releasing");
    }
    let head = git_output(&["rev-parse", "HEAD^{commit}"])?;
    let default_branch = default_branch_from_origin()?;
    let remote_head = git_output(&[
        "ls-remote",
        "origin",
        &format!("refs/heads/{default_branch}"),
    ])?;
    let remote_commit = remote_head
        .split_whitespace()
        .next()
        .context("origin default branch has no pushed commit")?;
    if head != remote_commit {
        bail!(
            "publish requires HEAD ({}) to equal pushed origin/{} ({}); push source commits to the default branch before releasing",
            head,
            default_branch,
            remote_commit
        );
    }
    Ok(())
}

fn default_branch_from_origin() -> Result<String> {
    let output = git_output(&["ls-remote", "--symref", "origin", "HEAD"])?;
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() == Some("ref:") {
            if let (Some(reference), Some("HEAD")) = (fields.next(), fields.next()) {
                if let Some(branch) = reference.strip_prefix("refs/heads/") {
                    return Ok(branch.to_string());
                }
            }
        }
    }
    bail!("could not determine origin default branch from `git ls-remote --symref origin HEAD`");
}

fn git_output(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to run `git {}`", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn prompt_yes_no(prompt: &str, default_yes: bool) -> Option<bool> {
    print!("{}", prompt);
    let _ = io::stdout().flush();
    let mut buf = String::new();
    if io::stdin().read_line(&mut buf).is_err() {
        return None;
    }
    let trimmed = buf.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Some(default_yes);
    }
    if trimmed == "y" || trimmed == "yes" {
        return Some(true);
    }
    if trimmed == "n" || trimmed == "no" {
        return Some(false);
    }
    Some(default_yes)
}

#[cfg(test)]
mod tests {
    use super::{reject_publish_tree_appledouble_at, reject_publish_tree_php_modules_at};
    use deka_modules::modules::MODULES_DIR;
    use std::{fs, process::Command};

    #[test]
    fn publish_artifact_rejects_appledouble_entries() {
        let temp = tempfile::tempdir().expect("temp repo");
        let repo = temp.path();
        for args in [
            vec!["init"],
            vec!["config", "user.email", "test@tana.gg"],
            vec!["config", "user.name", "test"],
        ] {
            assert!(Command::new("git")
                .current_dir(repo)
                .args(&args)
                .status()
                .expect("git")
                .success());
        }
        fs::write(repo.join("index.phpx"), "export const x = 1;").expect("module");
        fs::write(repo.join("._index.phpx"), b"\x00\x05\x16\x07\x00\x02\x00\x00")
            .expect("top-level AppleDouble");
        fs::create_dir_all(repo.join("lib")).expect("lib dir");
        fs::write(repo.join("lib/._helper.phpx"), b"\x00\x05\x16\x07\x00\x02\x00\x00")
            .expect("nested AppleDouble");
        for args in [vec!["add", "."], vec!["commit", "-m", "appledouble"]] {
            assert!(Command::new("git")
                .current_dir(repo)
                .args(&args)
                .status()
                .expect("git")
                .success());
        }

        // A tree containing AppleDouble entries must be refused, with the
        // offending file named and the fix pointed at the source tree.
        let err = reject_publish_tree_appledouble_at(Some(repo), "HEAD").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("publish rejected"), "{msg}");
        assert!(msg.contains("._index.phpx"), "{msg}");
        assert!(msg.contains("source tree"), "{msg}");

        // ... and after removing the AppleDouble files, the same ref passes.
        fs::remove_file(repo.join("._index.phpx")).expect("remove top-level");
        fs::remove_file(repo.join("lib/._helper.phpx")).expect("remove nested");
        assert!(Command::new("git")
            .current_dir(repo)
            .args(["add", "-A"])
            .status()
            .expect("add")
            .success());
        assert!(Command::new("git")
            .current_dir(repo)
            .args(["commit", "-m", "clean"])
            .status()
            .expect("commit")
            .success());
        reject_publish_tree_appledouble_at(Some(repo), "HEAD")
            .expect("clean tree must publish");
    }

    #[test]
    fn publish_artifact_rejects_case_variant_and_symlinked_php_modules() {
        let temp = tempfile::tempdir().expect("temp repo");
        let repo = temp.path();
        assert!(Command::new("git")
            .current_dir(repo)
            .arg("init")
            .status()
            .expect("git")
            .success());
        assert!(Command::new("git")
            .current_dir(repo)
            .args(["config", "user.email", "test@tana.gg"])
            .status()
            .expect("git")
            .success());
        assert!(Command::new("git")
            .current_dir(repo)
            .args(["config", "user.name", "test"])
            .status()
            .expect("git")
            .success());
        fs::create_dir_all(repo.join("Php_Modules")).expect("case directory");
        fs::write(repo.join("Php_Modules/module.phpx"), "export const x = 1;").expect("module");
        assert!(Command::new("git")
            .current_dir(repo)
            .args(["add", "."])
            .status()
            .expect("add")
            .success());
        assert!(Command::new("git")
            .current_dir(repo)
            .args(["commit", "-m", "case variant"])
            .status()
            .expect("commit")
            .success());
        let err = reject_publish_tree_php_modules_at(Some(repo), "HEAD").unwrap_err();
        assert!(err.to_string().contains("Php_Modules"), "{err}");

        fs::remove_dir_all(repo.join("Php_Modules")).expect("remove case directory");
        #[cfg(unix)]
        std::os::unix::fs::symlink("outside", repo.join(MODULES_DIR)).expect("symlink");
        #[cfg(not(unix))]
        panic!("publish artifact symlink test requires unix");
        assert!(Command::new("git")
            .current_dir(repo)
            .args(["add", "-A"])
            .status()
            .expect("add")
            .success());
        assert!(Command::new("git")
            .current_dir(repo)
            .args(["commit", "-m", "symlink"])
            .status()
            .expect("commit")
            .success());
        let err = reject_publish_tree_php_modules_at(Some(repo), "HEAD").unwrap_err();
        assert!(err.to_string().contains("php_modules"), "{err}");
    }
}
