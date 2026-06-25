use anyhow::{Context as AnyhowContext, Result, bail};
use core::{CommandSpec, Context, FlagSpec, ParamSpec, Registry};
use serde_json::Value;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use stdio;

const COMMAND: CommandSpec = CommandSpec {
    name: "release",
    category: "package",
    summary: "bump, tag, push, and publish a package release",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_param(ParamSpec {
        name: "--name",
        description: "package name (@scope/name). default: deka.json name",
    });
    registry.add_param(ParamSpec {
        name: "--repo",
        description: "source git repo name in linkhash. default: package basename",
    });
    registry.add_param(ParamSpec {
        name: "--version",
        description: "target version (deprecated alias; use --pkg-version)",
    });
    registry.add_param(ParamSpec {
        name: "--pkg-version",
        description: "target version",
    });
    registry.add_param(ParamSpec {
        name: "--bump",
        description: "version bump strategy: patch|minor|major (default: patch)",
    });
    registry.add_param(ParamSpec {
        name: "--token",
        description: "PAT token passed through to publish",
    });
    registry.add_param(ParamSpec {
        name: "--registry-url",
        description: "registry URL passed through to publish",
    });
    registry.add_param(ParamSpec {
        name: "--description",
        description: "description passed through to publish",
    });
    registry.add_flag(FlagSpec {
        name: "--no-push",
        aliases: &[],
        description: "skip pushing commit and tag to origin",
    });
    registry.add_flag(FlagSpec {
        name: "--yes",
        aliases: &["-y"],
        description: "auto-accept prompts",
    });
}

pub fn cmd(context: &Context) {
    match run_release(context) {
        Ok(()) => {}
        Err(err) => {
            stdio::error("release", &err.to_string());
            std::process::exit(1);
        }
    }
}

fn run_release(context: &Context) -> Result<()> {
    let auto_yes =
        context.args.flags.contains_key("--yes") || context.args.flags.contains_key("-y");
    let no_push = context.args.flags.contains_key("--no-push");
    let manifest_path = std::env::current_dir()
        .context("failed to read current directory")?
        .join("deka.json");

    let mut manifest = load_manifest(&manifest_path)?;
    let current_name = manifest
        .json
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .context("deka.json is missing required string field `name`")?
        .to_string();
    let current_version = manifest
        .json
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .context("deka.json is missing required string field `version`")?
        .to_string();

    let name = context
        .args
        .params
        .get("--name")
        .cloned()
        .unwrap_or_else(|| current_name.clone());
    validate_scoped_package_name(&name)?;
    let repo = context
        .args
        .params
        .get("--repo")
        .cloned()
        .unwrap_or_else(|| {
            package_basename_from_scoped(&name).unwrap_or_else(|_| "package".to_string())
        });

    let target_version = if let Some(explicit) = context
        .args
        .params
        .get("--pkg-version")
        .or_else(|| context.args.params.get("--version"))
    {
        explicit.trim().to_string()
    } else {
        let bump = context
            .args
            .params
            .get("--bump")
            .map(|v| v.to_ascii_lowercase())
            .unwrap_or_else(|| "patch".to_string());
        bump_version(&current_version, &bump)?
    };

    ensure_semver_gt(&target_version, &current_version)?;
    let release_tag = format!("v{}", target_version);

    let mut plan = vec![
        format!("name: {}", name),
        format!("repo: {}", repo),
        format!("version: {} -> {}", current_version, target_version),
        format!("tag: {}", release_tag),
        format!("push: {}", if no_push { "no" } else { "yes" }),
    ];

    let changed_name = current_name != name;
    if changed_name {
        plan.push(format!(
            "update deka.json name: {} -> {}",
            current_name, name
        ));
    }

    stdio::log("release", "planned operations:");
    for item in &plan {
        stdio::log("release", &format!("  - {}", item));
    }

    if !auto_yes && !prompt_yes_no("Proceed with release? [Y/n]: ", true).unwrap_or(false) {
        bail!("release canceled");
    }

    manifest.json["name"] = Value::String(name.clone());
    manifest.json["version"] = Value::String(target_version.clone());
    write_manifest(&manifest_path, &manifest.json)?;

    run_git(["add", "deka.json"])?;
    run_git([
        "commit",
        "-m",
        &format!("release: {}@{}", name, target_version),
    ])?;

    if git_ref_exists(&format!("refs/tags/{}", release_tag))? {
        bail!(
            "release tag `{}` already exists. choose a higher version or delete the existing tag",
            release_tag
        );
    }
    run_git(["tag", &release_tag])?;

    if !no_push {
        run_git(["push", "origin", "HEAD"])?;
        run_git(["push", "origin", &release_tag])?;
    }

    run_publish(
        context,
        &name,
        &repo,
        &target_version,
        &release_tag,
        auto_yes,
    )?;

    stdio::log("release", &format!("released {}@{}", name, target_version));
    Ok(())
}

struct LocalManifest {
    json: Value,
}

fn load_manifest(path: &std::path::Path) -> Result<LocalManifest> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let json = serde_json::from_str::<Value>(&raw)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    Ok(LocalManifest { json })
}

fn write_manifest(path: &std::path::Path, json: &Value) -> Result<()> {
    let body = serde_json::to_string_pretty(json).context("failed to format deka.json")?;
    std::fs::write(path, format!("{}\n", body))
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn bump_version(current: &str, bump: &str) -> Result<String> {
    let (mut major, mut minor, mut patch) = parse_semver_triplet(current)?;
    match bump {
        "patch" => patch += 1,
        "minor" => {
            minor += 1;
            patch = 0;
        }
        "major" => {
            major += 1;
            minor = 0;
            patch = 0;
        }
        other => bail!("invalid --bump `{}` (expected patch|minor|major)", other),
    }
    Ok(format!("{}.{}.{}", major, minor, patch))
}

fn parse_semver_triplet(raw: &str) -> Result<(u64, u64, u64)> {
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.len() != 3 {
        bail!("version `{}` must be in major.minor.patch format", raw);
    }
    let major = parts[0]
        .parse::<u64>()
        .with_context(|| format!("invalid major in `{}`", raw))?;
    let minor = parts[1]
        .parse::<u64>()
        .with_context(|| format!("invalid minor in `{}`", raw))?;
    let patch = parts[2]
        .parse::<u64>()
        .with_context(|| format!("invalid patch in `{}`", raw))?;
    Ok((major, minor, patch))
}

fn ensure_semver_gt(next: &str, current: &str) -> Result<()> {
    let a = parse_semver_triplet(next)?;
    let b = parse_semver_triplet(current)?;
    if a <= b {
        bail!(
            "target version `{}` must be greater than current `{}`",
            next,
            current
        );
    }
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

fn package_basename_from_scoped(name: &str) -> Result<String> {
    validate_scoped_package_name(name)?;
    let mut parts = name.split('/');
    let _scope = parts.next();
    let pkg = parts.next().unwrap_or("");
    Ok(pkg.to_string())
}

fn run_git<const N: usize>(args: [&str; N]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .status()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !status.success() {
        bail!("git command failed: git {}", args.join(" "));
    }
    Ok(())
}

fn git_ref_exists(reference: &str) -> Result<bool> {
    let status = Command::new("git")
        .args(["rev-parse", "--verify", reference])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to verify git ref {}", reference))?;
    Ok(status.success())
}

fn run_publish(
    context: &Context,
    name: &str,
    repo: &str,
    version: &str,
    git_ref: &str,
    auto_yes: bool,
) -> Result<()> {
    let exe = std::env::current_exe().context("failed to resolve current deka executable")?;
    let mut cmd = Command::new(exe);
    cmd.arg("publish")
        .arg("--name")
        .arg(name)
        .arg("--repo")
        .arg(repo)
        .arg("--pkg-version")
        .arg(version)
        .arg("--git-ref")
        .arg(git_ref)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if auto_yes {
        cmd.arg("--yes");
    }
    if let Some(registry) = context.args.params.get("--registry-url") {
        cmd.arg("--registry-url").arg(registry);
    }
    if let Some(token) = context.args.params.get("--token") {
        cmd.arg("--token").arg(token);
    }
    if let Some(description) = context.args.params.get("--description") {
        cmd.arg("--description").arg(description);
    }

    let status = cmd.status().context("failed to run deka publish")?;
    if !status.success() {
        bail!("release publish step failed");
    }
    Ok(())
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
