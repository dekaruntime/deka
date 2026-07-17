use crate::{lock, payload::InstallPayload, spec::parse_package_spec};
use anyhow::{Context, Result, anyhow, bail};
use linkhash_client::LinkhashClient;
use modules_php::integrity::compute_package_integrity;
use runtime_core::module_spec::canonical_php_package_spec;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

mod bundled_stdlib {
    include!(concat!(env!("OUT_DIR"), "/stdlib_snapshot.rs"));
}

const BUNDLED_STDLIB_VERSION: &str = "0.1.0";

pub async fn run_install(payload: InstallPayload) -> Result<()> {
    if payload.rehash {
        rehash_php_packages(&payload).await?;
        return Ok(());
    }

    let specs = payload.specs.clone();
    let quiet = payload.quiet;
    tokio::task::spawn_blocking(move || run_php_install(specs, quiet))
        .await
        .context("install task failed")?
}

fn run_php_install(specs: Vec<String>, quiet: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    run_php_install_in(
        specs,
        quiet,
        &cwd,
        &linkhash_registry_url(),
        linkhash_token().as_deref(),
    )
}

fn run_php_install_in(
    specs: Vec<String>,
    quiet: bool,
    cwd: &Path,
    registry: &str,
    token: Option<&str>,
) -> Result<()> {
    let specs = if specs.is_empty() {
        collect_project_install_specs(cwd)?
    } else {
        specs
    };

    if specs.is_empty() {
        bail!("no PHP packages declared in deka.json or deka.lock");
    }

    let client = LinkhashClient::new(registry, token);
    let lock_path = cwd.join(lock::LOCKFILE_NAME);
    let existing_lock = lock::read_lockfile_at(&lock_path);
    let start = Instant::now();
    let mut pending = VecDeque::new();
    let mut requested = BTreeMap::new();
    for spec in specs {
        enqueue_package_spec(&mut pending, &mut requested, &spec, "root")?;
    }
    let mut installed = BTreeMap::new();

    // A release is source plus its declared dependencies, never a recursive
    // vendor tree. Resolve each declared dependency here so every package is a
    // sibling under the consumer's php_modules directory.
    while let Some(name) = pending.pop_front() {
        let requirements = requested.get(&name).expect("queued package requirement");
        let locked = locked_package(&existing_lock, &name)?;
        let version = select_version(&client, &name, requirements, locked.as_ref())?;
        let destination = php_modules_path_for_in(cwd, &name)?;
        let staging = install_staging_path(&destination)?;
        cleanup_install_staging(&staging);
        let install_source = match install_from_linkhash(&client, &name, &version, &staging) {
            Ok(source) => source,
            Err(err) if is_deka_package(&name) => {
                cleanup_install_staging(&staging);
                if !quiet {
                    eprintln!(
                        "[install] LinkHash unavailable for {} ({}); using bundled stdlib snapshot",
                        name, err
                    );
                }
                install_from_bundled_stdlib(&name, locked.as_ref(), &staging)?
            }
            Err(err) => {
                cleanup_install_staging(&staging);
                return Err(err);
            }
        };

        if let Err(err) = reject_vendored_php_modules(&staging, &name) {
            cleanup_install_staging(&staging);
            return Err(err);
        }

        let package_integrity = compute_package_integrity(&staging)
            .map_err(|err| anyhow!("failed to compute package integrity for {}: {}", name, err))?;

        if let Some(locked) = &locked {
            if let Err(err) =
                verify_locked_integrity(&name, locked, &install_source, &package_integrity)
            {
                cleanup_install_staging(&staging);
                return Err(err);
            }
            // The package still needs to contribute its declared dependencies
            // to the flat graph, even when its bytes are already lock-verified.
        }

        let dependencies = package_dependencies(&staging, &name)?;
        let dependency_names = dependencies
            .iter()
            .map(|dependency| {
                let (raw_name, _) = parse_package_spec(dependency);
                normalize_php_spec(&raw_name)
            })
            .collect::<Result<Vec<_>>>()?;
        let metadata = json!({
            "repo": install_source.repo,
            "gitRef": install_source.git_ref,
            "source": install_source.source,
            "dependencies": dependency_names,
            "moduleGraph": {
                "algo": "sha256",
                "hash": package_integrity.module_graph,
            },
            "fsGraph": {
                "algo": "sha256",
                "hash": package_integrity.fs_graph,
            },
        });
        replace_installed_package(&staging, &destination)?;
        installed.insert(
            name.clone(),
            (
                format!("{}@{}", name, install_source.version),
                format!("linkhash:{}", name),
                metadata,
                String::new(),
            ),
        );
        for dependency in dependencies {
            enqueue_package_spec(&mut pending, &mut requested, &dependency, &name)?;
        }
    }

    // The lockfile represents exactly the resolved transitive graph. This
    // also removes stale dependencies that are no longer declared by a release.
    lock::write_lockfile_at(
        &lock_path,
        &lock::DekaLock {
            lockfile_version: existing_lock.lockfile_version,
            packages: installed.clone(),
        },
    )?;

    let duration = Instant::now().duration_since(start);
    emit_summary(installed.len(), duration.as_millis() as u64, quiet)?;
    Ok(())
}

fn enqueue_package_spec(
    pending: &mut VecDeque<String>,
    requested: &mut BTreeMap<String, Vec<VersionRequirement>>,
    spec: &str,
    requested_by: &str,
) -> Result<()> {
    let (raw_name, version) = parse_package_spec(spec.trim());
    let name = normalize_php_spec(&raw_name)?;
    let requirement = VersionRequirement {
        range: version.unwrap_or_else(|| "latest".to_string()),
        requested_by: requested_by.to_string(),
    };
    let requirements = requested.entry(name.clone()).or_default();
    if !requirements.iter().any(|existing| existing == &requirement) {
        requirements.push(requirement);
        // Revisit a package when a newly discovered constraint may select a
        // higher common version or expose a conflict.
        pending.push_back(name);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionRequirement {
    range: String,
    requested_by: String,
}

fn select_version(
    client: &LinkhashClient,
    name: &str,
    requirements: &[VersionRequirement],
    locked: Option<&LockedPackage>,
) -> Result<String> {
    if let Some(locked) = locked {
        if requirements
            .iter()
            .all(|requirement| version_satisfies(&locked.version, &requirement.range))
        {
            return Ok(locked.version.clone());
        }
    }
    let mut versions = match client.list_versions(name) {
        Ok(versions) => versions,
        // Built-in packages remain installable offline.  They have one
        // bundled version, so this does not weaken multi-version resolution.
        Err(_) if is_deka_package(name) => return Ok(BUNDLED_STDLIB_VERSION.to_string()),
        Err(err) => return Err(err),
    };
    versions.sort_by(|left, right| {
        semver::Version::parse(left)
            .ok()
            .cmp(&semver::Version::parse(right).ok())
    });
    let selected = versions.into_iter().rev().find(|version| {
        semver::Version::parse(version).is_ok()
            && requirements
                .iter()
                .all(|requirement| version_satisfies(version, &requirement.range))
    });
    selected.ok_or_else(|| version_conflict(name, requirements))
}

fn version_satisfies(version: &str, range: &str) -> bool {
    if range.trim().is_empty() || matches!(range.trim(), "latest" | "*") {
        return true;
    }
    let Ok(version) = semver::Version::parse(version) else {
        return false;
    };
    let range = normalize_version_range(range);
    semver::VersionReq::parse(&range).is_ok_and(|requirement| requirement.matches(&version))
}

fn normalize_version_range(range: &str) -> String {
    let range = range.trim();
    let (prefix, bare) = range
        .strip_prefix('^')
        .map_or(("", range), |value| ("^", value));
    let components = bare.split('.').count();
    if bare
        .chars()
        .all(|character| character.is_ascii_digit() || character == '.')
        && components < 3
    {
        format!("{prefix}{bare}{}", ".0".repeat(3 - components))
    } else {
        range.to_string()
    }
}

fn version_conflict(name: &str, requirements: &[VersionRequirement]) -> anyhow::Error {
    let left = requirements
        .first()
        .expect("version conflict has requirement");
    let right = requirements
        .iter()
        .skip(1)
        .find(|requirement| requirement.range != left.range)
        .unwrap_or(left);
    anyhow!(
        "version conflict: {} required as {} by {} and {} by {}",
        name,
        left.range,
        left.requested_by,
        right.range,
        right.requested_by
    )
}

fn package_dependencies(package_root: &Path, package_name: &str) -> Result<Vec<String>> {
    let manifest_path = package_root.join("deka.json");
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_str(&raw).with_context(|| {
        format!(
            "failed to parse {} for {}",
            manifest_path.display(),
            package_name
        )
    })?;
    let Some(dependencies) = manifest.get("dependencies").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut specs = Vec::new();
    for (name, version) in dependencies {
        let spec = match version.as_str().map(str::trim) {
            Some("") | Some("*") | Some("latest") | None => name.clone(),
            Some(version) => format!("{}@{}", name, version),
        };
        specs.push(spec);
    }
    specs.sort();
    Ok(specs)
}

fn reject_vendored_php_modules(package_root: &Path, package_name: &str) -> Result<()> {
    let mut directories = vec![package_root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let entry_name = entry.file_name();
            if entry_name
                .to_string_lossy()
                .eq_ignore_ascii_case("php_modules")
            {
                bail!(
                    "package {} contains vendored php_modules at {}; packages must declare dependencies in deka.json",
                    package_name,
                    entry.path().display()
                );
            }
            // Never follow artifact symlinks.  A package must be a real tree;
            // otherwise a link can escape the staging root or alias vendored modules.
            if file_type.is_symlink() {
                bail!(
                    "package {} contains symlink {}; package artifacts may not contain symlinks",
                    package_name,
                    entry.path().display()
                );
            }
            if file_type.is_dir() {
                directories.push(entry.path());
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct InstalledSource {
    version: String,
    repo: Option<String>,
    git_ref: Option<String>,
    source: &'static str,
}

#[derive(Debug, Clone)]
struct LockedPackage {
    version: String,
    resolved: String,
    module_graph: String,
    fs_graph: String,
}

fn install_from_linkhash(
    client: &LinkhashClient,
    name: &str,
    version_range: &str,
    destination: &Path,
) -> Result<InstalledSource> {
    let resolved = client.resolve(name, version_range)?;
    client.download(name, &resolved.version, destination)?;
    Ok(InstalledSource {
        version: resolved.version,
        repo: resolved.repo,
        git_ref: resolved.git_ref,
        source: "linkhash",
    })
}

fn install_from_bundled_stdlib(
    name: &str,
    locked: Option<&LockedPackage>,
    destination: &Path,
) -> Result<InstalledSource> {
    if let Some(locked) = locked {
        if locked.version != BUNDLED_STDLIB_VERSION {
            bail!(
                "bundled stdlib cannot satisfy locked {}@{} (bundle has {})",
                name,
                locked.version,
                BUNDLED_STDLIB_VERSION
            );
        }
    }
    let source_prefix = bundled_stdlib_prefix(name)
        .ok_or_else(|| anyhow!("no bundled stdlib package for {}", name))?;
    let mut copied = 0usize;
    for (relative, bytes) in bundled_stdlib::STDLIB_FILES {
        let Some(package_relative) = relative.strip_prefix(&source_prefix) else {
            continue;
        };
        let Some(package_relative) = package_relative.strip_prefix('/') else {
            continue;
        };
        if package_relative.is_empty() {
            continue;
        }
        let target = destination.join(package_relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&target, bytes)
            .with_context(|| format!("failed to write {}", target.display()))?;
        copied += 1;
    }

    if copied == 0 {
        bail!("bundled stdlib package {} is empty or missing", name);
    }

    write_bundled_package_manifest(name, destination)?;

    Ok(InstalledSource {
        version: locked
            .map(|locked| locked.version.clone())
            .unwrap_or_else(|| BUNDLED_STDLIB_VERSION.to_string()),
        repo: None,
        git_ref: None,
        source: "linkhash",
    })
}

fn install_staging_path(destination: &Path) -> Result<PathBuf> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("install destination has no parent"))?;
    let package_dir = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("install destination has invalid package directory"))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    Ok(parent
        .join(format!(".deka-install-{}-{}", std::process::id(), nanos))
        .join(package_dir))
}

fn replace_installed_package(staging: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_dir_all(destination)
            .with_context(|| format!("failed to remove {}", destination.display()))?;
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let staging_root = staging.parent().map(Path::to_path_buf);
    fs::rename(staging, destination).or_else(|_| {
        copy_dir_all(staging, destination)?;
        fs::remove_dir_all(staging)
            .with_context(|| format!("failed to remove {}", staging.display()))
    })?;
    if let Some(root) = staging_root {
        let _ = fs::remove_dir(root);
    }
    Ok(())
}

fn cleanup_install_staging(staging: &Path) {
    if let Some(root) = staging.parent() {
        let _ = fs::remove_dir_all(root);
    }
}

fn copy_dir_all(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target).with_context(|| format!("failed to create {}", target.display()))?;
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn write_bundled_package_manifest(name: &str, destination: &Path) -> Result<()> {
    let Some(package_name) = name.strip_prefix("@deka/") else {
        return Ok(());
    };
    let main = if package_name == "encoding" {
        "json/index.phpx"
    } else {
        "index.phpx"
    };
    let manifest = format!(
        "{{\n  \"name\": \"{}\",\n  \"version\": \"{}\",\n  \"description\": \"PHPX stdlib: {}\",\n  \"main\": \"{}\",\n  \"deka.security\": {{ \"allow\": {{ \"run\": true }} }}\n}}\n",
        name, BUNDLED_STDLIB_VERSION, package_name, main
    );
    let target = destination.join("deka.json");
    fs::write(&target, manifest).with_context(|| format!("failed to write {}", target.display()))
}

fn bundled_stdlib_prefix(name: &str) -> Option<String> {
    let rest = name.strip_prefix("@deka/")?;
    if rest == "encoding-json" {
        return Some("encoding/json".to_string());
    }
    if rest == "encoding-binary" {
        return Some("encoding/binary".to_string());
    }
    if rest == "vault" {
        return Some("deka/vault".to_string());
    }
    if rest.contains('/') || rest.contains("..") || rest.is_empty() {
        return None;
    }
    Some(rest.to_string())
}

fn collect_project_install_specs(project_dir: &Path) -> Result<Vec<String>> {
    let mut specs = BTreeMap::new();
    for spec in collect_locked_deka_specs(project_dir) {
        let (name, _) = parse_package_spec(&spec);
        specs.insert(name, spec);
    }
    for spec in collect_deka_json_deps_in(project_dir)? {
        let (name, _) = parse_package_spec(&spec);
        specs.insert(name, spec);
    }

    Ok(specs.into_values().collect())
}

fn collect_deka_json_deps_in(project_dir: &Path) -> Result<Vec<String>> {
    let path = project_dir.join("deka.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let json: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(deps) = json.get("dependencies").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (name, version) in deps {
        let spec = if let Some(version) = version.as_str() {
            let version = version.trim();
            if version.is_empty() || version == "*" || version == "latest" {
                name.to_string()
            } else {
                format!("{}@{}", name, version)
            }
        } else {
            name.to_string()
        };
        out.push(spec);
    }
    Ok(out)
}

fn collect_locked_deka_specs(project_dir: &Path) -> Vec<String> {
    let lock_path = project_dir.join(lock::LOCKFILE_NAME);
    let lock = lock::read_lockfile_at(&lock_path);
    lock.packages
        .into_iter()
        .filter_map(|(name, entry)| {
            if !is_deka_package(&name) {
                return None;
            }
            let (descriptor, _, _, _) = entry;
            if descriptor.starts_with(&format!("{}@", name)) {
                Some(descriptor)
            } else if is_version_descriptor(&descriptor) {
                Some(format!("{}@{}", name, descriptor))
            } else {
                Some(name)
            }
        })
        .collect()
}

fn is_deka_package(name: &str) -> bool {
    name.starts_with("@deka/")
}

fn locked_package(lock: &lock::DekaLock, name: &str) -> Result<Option<LockedPackage>> {
    let Some((descriptor, resolved, metadata, _)) = lock.packages.get(name) else {
        return Ok(None);
    };
    let version = locked_version(name, descriptor)
        .ok_or_else(|| anyhow!("locked package {} is missing an exact version", name))?;
    let module_graph = metadata_hash(metadata, "moduleGraph")
        .ok_or_else(|| anyhow!("locked package {} is missing moduleGraph hash", name))?;
    let fs_graph = metadata_hash(metadata, "fsGraph")
        .ok_or_else(|| anyhow!("locked package {} is missing fsGraph hash", name))?;
    Ok(Some(LockedPackage {
        version,
        resolved: resolved.clone(),
        module_graph,
        fs_graph,
    }))
}

fn locked_version(name: &str, descriptor: &str) -> Option<String> {
    descriptor
        .strip_prefix(&format!("{}@", name))
        .filter(|version| !version.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            if is_version_descriptor(descriptor) {
                Some(descriptor.to_string())
            } else {
                None
            }
        })
}

fn is_version_descriptor(descriptor: &str) -> bool {
    let mut parts = descriptor.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    let Some(patch) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && [major, minor, patch]
            .into_iter()
            .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

fn metadata_hash(metadata: &Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .filter(|hash| !hash.is_empty())
        .map(ToString::to_string)
}

fn verify_locked_integrity(
    name: &str,
    locked: &LockedPackage,
    installed: &InstalledSource,
    integrity: &modules_php::integrity::PackageIntegrity,
) -> Result<()> {
    if installed.version != locked.version {
        bail!(
            "integrity verification failed for {}: locked version {} but installed {}",
            name,
            locked.version,
            installed.version
        );
    }
    if locked.resolved != format!("linkhash:{}", name) {
        bail!(
            "integrity verification failed for {}: unsupported lock source {}",
            name,
            locked.resolved
        );
    }
    if integrity.module_graph != locked.module_graph {
        bail!(
            "integrity verification failed for {}: moduleGraph hash mismatch (locked {}, got {})",
            name,
            locked.module_graph,
            integrity.module_graph
        );
    }
    if integrity.fs_graph != locked.fs_graph {
        bail!(
            "integrity verification failed for {}: fsGraph hash mismatch (locked {}, got {})",
            name,
            locked.fs_graph,
            integrity.fs_graph
        );
    }
    Ok(())
}

async fn rehash_php_packages(payload: &InstallPayload) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    rehash_php_packages_in(payload, &cwd).await
}

async fn rehash_php_packages_in(payload: &InstallPayload, project_dir: &Path) -> Result<()> {
    let lock_path = project_dir.join(lock::LOCKFILE_NAME);
    let lock = lock::read_lockfile_at(&lock_path);
    let mut specs = payload.specs.clone();
    if specs.is_empty() {
        specs = lock.packages.keys().cloned().collect();
    }
    if specs.is_empty() {
        bail!("no PHP packages found to rehash");
    }

    for name in specs {
        let package_root = php_modules_path_for_in(project_dir, &name)?;
        if !package_root.is_dir() {
            bail!(
                "package '{}' is missing from php_modules (expected {})",
                name,
                package_root.display()
            );
        }
        let integrity = compute_package_integrity(&package_root)
            .map_err(|err| anyhow!("failed to compute package integrity for {}: {}", name, err))?;
        let entry = lock.packages.get(&name).cloned();
        let Some((descriptor, resolved, mut metadata, integrity_field)) = entry else {
            bail!("package '{}' is missing from deka.lock", name);
        };
        if let Value::Object(map) = &mut metadata {
            map.insert(
                "moduleGraph".to_string(),
                json!({ "algo": "sha256", "hash": integrity.module_graph }),
            );
            map.insert(
                "fsGraph".to_string(),
                json!({ "algo": "sha256", "hash": integrity.fs_graph }),
            );
        }
        lock::update_lock_entry_at(
            &lock_path,
            &name,
            descriptor,
            resolved,
            metadata,
            integrity_field,
        )?;
    }

    Ok(())
}

fn normalize_php_spec(spec: &str) -> Result<String> {
    let trimmed = spec.trim();
    if trimmed.starts_with('@') {
        if is_valid_scoped_name(trimmed) {
            return Ok(trimmed.to_string());
        }
        bail!(
            "invalid php package `{}` (expected @scope/name[@version])",
            trimmed
        );
    }
    if let Some(mapped) = canonical_php_package_spec(trimmed) {
        return Ok(mapped);
    }
    bail!(
        "unscoped php package `{}` is not allowed. use @scope/name (bare names map to @deka/*)",
        trimmed
    )
}

fn is_valid_scoped_name(spec: &str) -> bool {
    if !spec.starts_with('@') {
        return false;
    }
    let without_version = if let Some(idx) = spec[1..].find('@') {
        &spec[..idx + 1]
    } else {
        spec
    };
    let mut parts = without_version.split('/');
    let scope = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return false;
    }
    if !scope.starts_with('@') || scope.len() <= 1 || name.is_empty() {
        return false;
    }
    scope[1..]
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn linkhash_registry_url() -> String {
    std::env::var("LINKHASH_REGISTRY_URL")
        .or_else(|_| std::env::var("LINKHASH_REGISTRY"))
        .or_else(|_| std::env::var("TANA_GIT_SERVER"))
        .unwrap_or_else(|_| "https://git.tana.gg".to_string())
}

fn linkhash_token() -> Option<String> {
    std::env::var("LINKHASH_TOKEN")
        .or_else(|_| std::env::var("TANA_GIT_TOKEN"))
        .ok()
}

#[cfg(test)]
fn php_modules_path_for(package_name: &str) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    php_modules_path_for_in(&cwd, package_name)
}

fn php_modules_path_for_in(project_dir: &Path, package_name: &str) -> Result<PathBuf> {
    let mut path = project_dir.join("php_modules");
    // Scoped packages (@scope/name) map to php_modules/@scope/name on disk.
    // This matches the layout produced by the bundler's module resolver and
    // the stdlib install layout. Unscoped names are still supported as-is.
    let segments: Vec<&str> = package_name.split('/').collect();
    for segment in segments {
        if segment.is_empty() || segment == "." || segment == ".." {
            bail!("invalid php package name segment");
        }
        path = path.join(segment);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::{
        InstalledSource, LockedPackage, bundled_stdlib_prefix, collect_project_install_specs,
        enqueue_package_spec, install_from_bundled_stdlib, locked_package, package_dependencies,
        php_modules_path_for, rehash_php_packages_in, reject_vendored_php_modules,
        replace_installed_package, run_php_install_in, verify_locked_integrity,
    };
    use crate::{lock, payload::InstallPayload};
    use modules_php::integrity::{PackageIntegrity, compute_package_integrity};
    use serde_json::json;
    use std::{collections::BTreeMap, fs};

    #[test]
    fn scoped_package_installs_to_scoped_php_modules_path() {
        let cwd = std::env::current_dir().expect("cwd");
        let path = php_modules_path_for("@deka/component").expect("path");
        assert_eq!(
            path,
            cwd.join("php_modules").join("@deka").join("component")
        );
    }

    #[test]
    fn non_deka_scoped_package_preserves_scope_in_path() {
        let cwd = std::env::current_dir().expect("cwd");
        let path = php_modules_path_for("@tana/store").expect("path");
        assert_eq!(path, cwd.join("php_modules").join("@tana").join("store"));
    }

    #[test]
    fn unscoped_package_preserves_legacy_path() {
        let cwd = std::env::current_dir().expect("cwd");
        let path = php_modules_path_for("legacy").expect("path");
        assert_eq!(path, cwd.join("php_modules").join("legacy"));
    }

    #[tokio::test]
    async fn rehash_resolves_locked_deka_package_to_stdlib_root() {
        let tmp = tempfile::tempdir().expect("tmp");
        let package_root = tmp
            .path()
            .join("php_modules")
            .join("@deka")
            .join("component");
        fs::create_dir_all(&package_root).expect("mkdir package");
        fs::write(
            package_root.join("index.phpx"),
            "import { ok } from '@deka/core';\nexport function component_ok() { return ok(); }\n",
        )
        .expect("write module");
        fs::write(
            tmp.path().join("deka.lock"),
            json!({
                "lockfileVersion": 1,
                "packages": {
                    "@deka/component": [
                        "@deka/component@0.1.0",
                        "linkhash:@deka/component",
                        {
                            "moduleGraph": { "algo": "sha256", "hash": "stale" },
                            "fsGraph": { "algo": "sha256", "hash": "stale" }
                        },
                        "sha512-stale"
                    ]
                }
            })
            .to_string(),
        )
        .expect("write lock");

        let payload = InstallPayload {
            specs: Vec::new(),
            yes: true,
            prompt: false,
            quiet: true,
            rehash: true,
        };
        rehash_php_packages_in(&payload, tmp.path())
            .await
            .expect("rehash");

        let expected = compute_package_integrity(&package_root).expect("integrity");
        let lock = lock::read_lockfile_at(&tmp.path().join("deka.lock"));
        let (_, _, metadata, _) = lock.packages.get("@deka/component").expect("lock entry");
        assert_eq!(
            metadata
                .get("moduleGraph")
                .and_then(|value| value.get("hash"))
                .and_then(|value| value.as_str()),
            Some(expected.module_graph.as_str())
        );
        assert_eq!(
            metadata
                .get("fsGraph")
                .and_then(|value| value.get("hash"))
                .and_then(|value| value.as_str()),
            Some(expected.fs_graph.as_str())
        );
    }

    #[test]
    fn empty_install_collects_deka_json_dependencies() {
        let tmp = tempfile::tempdir().expect("tmp");
        fs::write(
            tmp.path().join("deka.json"),
            json!({
                "dependencies": {
                    "@deka/core": "^0.1.0",
                    "@deka/encoding": "latest"
                }
            })
            .to_string(),
        )
        .expect("write deka.json");

        let specs = collect_project_install_specs(tmp.path()).expect("specs");
        assert_eq!(
            specs,
            vec![
                "@deka/core@^0.1.0".to_string(),
                "@deka/encoding".to_string()
            ]
        );
    }

    #[test]
    fn empty_install_falls_back_to_locked_deka_packages() {
        let tmp = tempfile::tempdir().expect("tmp");
        fs::write(
            tmp.path().join("deka.lock"),
            json!({
                "lockfileVersion": 1,
                "packages": {
                    "@deka/encoding": [
                        "@deka/encoding@0.1.0",
                        "linkhash:@deka/encoding",
                        {},
                        ""
                    ],
                    "@tana/app": [
                        "@tana/app@1.0.0",
                        "linkhash:@tana/app",
                        {},
                        ""
                    ]
                }
            })
            .to_string(),
        )
        .expect("write lock");

        let specs = collect_project_install_specs(tmp.path()).expect("specs");
        assert_eq!(specs, vec!["@deka/encoding@0.1.0".to_string()]);
    }

    #[test]
    fn empty_install_merges_locked_deka_packages_with_manifest_deps() {
        let tmp = tempfile::tempdir().expect("tmp");
        fs::write(
            tmp.path().join("deka.json"),
            json!({
                "dependencies": {
                    "@deka/encoding": "^0.2.0"
                }
            })
            .to_string(),
        )
        .expect("write deka.json");
        fs::write(
            tmp.path().join("deka.lock"),
            json!({
                "lockfileVersion": 1,
                "packages": {
                    "@deka/core": [
                        "@deka/core@0.1.0",
                        "linkhash:@deka/core",
                        {},
                        ""
                    ],
                    "@deka/encoding": [
                        "@deka/encoding@0.1.0",
                        "linkhash:@deka/encoding",
                        {},
                        ""
                    ]
                }
            })
            .to_string(),
        )
        .expect("write lock");

        let specs = collect_project_install_specs(tmp.path()).expect("specs");
        assert_eq!(
            specs,
            vec![
                "@deka/core@0.1.0".to_string(),
                "@deka/encoding@^0.2.0".to_string()
            ]
        );
    }

    #[test]
    fn bundled_stdlib_prefix_maps_nested_encoding_packages() {
        assert_eq!(
            bundled_stdlib_prefix("@deka/encoding-json").as_deref(),
            Some("encoding/json")
        );
        assert_eq!(
            bundled_stdlib_prefix("@deka/encoding").as_deref(),
            Some("encoding")
        );
    }

    #[test]
    fn deka_packages_always_install_to_canonical_scoped_paths() {
        let cwd = std::env::current_dir().expect("cwd");
        assert_eq!(
            php_modules_path_for("@deka/array").expect("path"),
            cwd.join("php_modules").join("@deka").join("array")
        );
        assert_eq!(
            php_modules_path_for("@deka/crypto").expect("path"),
            cwd.join("php_modules").join("@deka").join("crypto")
        );
    }

    #[test]
    fn bundled_stdlib_install_writes_resolver_compatible_shape() {
        let tmp = tempfile::tempdir().expect("tmp");
        let destination = tmp
            .path()
            .join("php_modules")
            .join("@deka")
            .join("encoding");
        install_from_bundled_stdlib("@deka/encoding", None, &destination).expect("install bundled");

        assert!(destination.join("json").join("index.phpx").is_file());
        assert!(destination.join("binary").join("index.phpx").is_file());

        let integrity = compute_package_integrity(&destination).expect("integrity");
        assert!(!integrity.module_graph.is_empty());
        assert!(!integrity.fs_graph.is_empty());
    }

    #[test]
    fn bundled_stdlib_installs_canonical_scoped_shape() {
        let tmp = tempfile::tempdir().expect("tmp");
        let destination = tmp.path().join("php_modules").join("@deka").join("http");
        install_from_bundled_stdlib("@deka/http", None, &destination).expect("install bundled");

        assert!(destination.join("index.phpx").is_file());
        let manifest = fs::read_to_string(destination.join("deka.json")).expect("manifest");
        assert!(manifest.contains("\"name\": \"@deka/http\""));
    }

    #[test]
    fn locked_package_accepts_legacy_version_descriptor() {
        let lock = lock::DekaLock {
            lockfile_version: 1,
            packages: BTreeMap::from([(
                "@deka/core".to_string(),
                (
                    "0.1.0".to_string(),
                    "linkhash:@deka/core".to_string(),
                    json!({
                        "moduleGraph": { "hash": "module-hash" },
                        "fsGraph": { "hash": "fs-hash" }
                    }),
                    String::new(),
                ),
            )]),
        };

        let locked = locked_package(&lock, "@deka/core")
            .expect("lock parse")
            .expect("locked package");

        assert_eq!(locked.version, "0.1.0");
        assert_eq!(locked.module_graph, "module-hash");
        assert_eq!(locked.fs_graph, "fs-hash");
    }

    #[test]
    fn locked_integrity_rejects_tampered_package_hashes() {
        let locked = LockedPackage {
            version: "0.1.0".to_string(),
            resolved: "linkhash:@deka/core".to_string(),
            module_graph: "locked-module".to_string(),
            fs_graph: "locked-fs".to_string(),
        };
        let installed = InstalledSource {
            version: "0.1.0".to_string(),
            repo: None,
            git_ref: None,
            source: "linkhash",
        };
        let integrity = PackageIntegrity {
            module_graph: "attacker-module".to_string(),
            fs_graph: "attacker-fs".to_string(),
        };

        let err =
            verify_locked_integrity("@deka/core", &locked, &installed, &integrity).unwrap_err();

        assert!(
            err.to_string().contains("moduleGraph hash mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn bundled_stdlib_can_satisfy_locked_entry_without_bundled_rewrite() {
        let tmp = tempfile::tempdir().expect("tmp");
        let destination = tmp
            .path()
            .join("php_modules")
            .join("@deka")
            .join("encoding");
        install_from_bundled_stdlib("@deka/encoding", None, &destination)
            .expect("install first bundle copy");
        let integrity = compute_package_integrity(&destination).expect("integrity");
        fs::remove_dir_all(&destination).expect("remove first copy");

        let lock_path = tmp.path().join("deka.lock");
        let lock_json = json!({
            "lockfileVersion": 1,
            "packages": {
                "@deka/encoding": [
                    "@deka/encoding@0.1.0",
                    "linkhash:@deka/encoding",
                    {
                        "moduleGraph": { "algo": "sha256", "hash": integrity.module_graph },
                        "fsGraph": { "algo": "sha256", "hash": integrity.fs_graph }
                    },
                    ""
                ]
            }
        });
        let lock_bytes = serde_json::to_string_pretty(&lock_json).expect("lock json");
        fs::write(&lock_path, &lock_bytes).expect("write lock");
        let lock = lock::read_lockfile_at(&lock_path);
        let locked = locked_package(&lock, "@deka/encoding")
            .expect("lock parse")
            .expect("locked package");

        let installed = install_from_bundled_stdlib("@deka/encoding", Some(&locked), &destination)
            .expect("install locked bundle copy");
        let installed_integrity = compute_package_integrity(&destination).expect("integrity");
        verify_locked_integrity("@deka/encoding", &locked, &installed, &installed_integrity)
            .expect("locked verification");

        assert_eq!(installed.version, "0.1.0");
        assert_eq!(installed.source, "linkhash");
        let after = fs::read_to_string(&lock_path).expect("read lock");
        assert_eq!(after, lock_bytes);
    }

    #[test]
    fn bundled_stdlib_rejects_locked_version_not_in_bundle_manifest() {
        let tmp = tempfile::tempdir().expect("tmp");
        let locked = LockedPackage {
            version: "9.9.9".to_string(),
            resolved: "linkhash:@deka/encoding".to_string(),
            module_graph: "unused".to_string(),
            fs_graph: "unused".to_string(),
        };

        let err = install_from_bundled_stdlib(
            "@deka/encoding",
            Some(&locked),
            &tmp.path()
                .join("php_modules")
                .join("@deka")
                .join("encoding"),
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("bundled stdlib cannot satisfy"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn staged_install_preserves_existing_package_until_verified_replace() {
        let tmp = tempfile::tempdir().expect("tmp");
        let destination = tmp.path().join("php_modules").join("core");
        fs::create_dir_all(&destination).expect("mkdir destination");
        fs::write(
            destination.join("index.phpx"),
            "export function ok() { return true; }\n",
        )
        .expect("write original");
        let staging = tmp
            .path()
            .join("php_modules")
            .join(".deka-install-test")
            .join("core");
        fs::create_dir_all(&staging).expect("mkdir staging");
        fs::write(
            staging.join("index.phpx"),
            "export function ok() { return 'verified'; }\n",
        )
        .expect("write staged");

        assert_eq!(
            fs::read_to_string(destination.join("index.phpx")).expect("read original"),
            "export function ok() { return true; }\n"
        );

        replace_installed_package(&staging, &destination).expect("replace");

        assert_eq!(
            fs::read_to_string(destination.join("index.phpx")).expect("read replaced"),
            "export function ok() { return 'verified'; }\n"
        );
        assert!(
            !tmp.path()
                .join("php_modules")
                .join(".deka-install-test")
                .exists()
        );
    }

    #[test]
    fn transitive_dependencies_are_queued_as_a_flat_scoped_graph() {
        let tmp = tempfile::tempdir().expect("tmp");
        let package = tmp.path().join("package-a");
        fs::create_dir_all(&package).expect("mkdir");
        fs::write(
            package.join("deka.json"),
            json!({ "dependencies": { "@tana/b": "^1.0.0", "crypto": "0.1.0" } }).to_string(),
        )
        .expect("manifest");

        let deps = package_dependencies(&package, "@tana/a").expect("deps");
        assert_eq!(deps, vec!["@tana/b@^1.0.0", "crypto@0.1.0"]);

        let mut pending = std::collections::VecDeque::new();
        let mut requested = BTreeMap::new();
        enqueue_package_spec(&mut pending, &mut requested, "@tana/a@1.0.0", "root").expect("root");
        for dep in deps {
            enqueue_package_spec(&mut pending, &mut requested, &dep, "@tana/a")
                .expect("dependency");
        }
        assert_eq!(
            pending.into_iter().collect::<Vec<_>>(),
            vec!["@tana/a", "@tana/b", "@deka/crypto"]
        );
        assert_eq!(
            php_modules_path_for("@tana/b").expect("path"),
            std::env::current_dir()
                .expect("cwd")
                .join("php_modules/@tana/b")
        );
        assert_eq!(
            php_modules_path_for("@deka/crypto").expect("path"),
            std::env::current_dir()
                .expect("cwd")
                .join("php_modules/@deka/crypto")
        );
    }

    #[test]
    fn install_rejects_vendored_php_modules_in_package_tree() {
        let tmp = tempfile::tempdir().expect("tmp");
        let nested = tmp
            .path()
            .join("src")
            .join("php_modules")
            .join("@tana")
            .join("b");
        fs::create_dir_all(&nested).expect("mkdir nested vendor tree");
        let err = reject_vendored_php_modules(tmp.path(), "@tana/a").expect_err("must reject");
        assert!(err.to_string().contains("contains vendored php_modules"));
    }

    #[cfg(unix)]
    #[test]
    fn install_rejects_case_variant_and_symlinked_vendor_trees() {
        let tmp = tempfile::tempdir().expect("tmp");
        fs::create_dir_all(tmp.path().join("Php_Modules")).expect("case vendor");
        assert!(reject_vendored_php_modules(tmp.path(), "@tana/a").is_err());
        fs::remove_dir_all(tmp.path().join("Php_Modules")).expect("remove case vendor");
        std::os::unix::fs::symlink("outside", tmp.path().join("php_modules"))
            .expect("vendor symlink");
        assert!(reject_vendored_php_modules(tmp.path(), "@tana/a").is_err());
    }

    #[tokio::test]
    async fn install_flattens_transitive_graph_and_writes_lock_integrity() {
        let packages = BTreeMap::from([
            ("@scope/a".to_string(), json!({ "@scope/b": "^1.0.0" })),
            ("@scope/b".to_string(), json!({ "@scope/c": "^1.0.0" })),
            ("@scope/c".to_string(), json!({})),
        ]);
        let (registry, shutdown) = fixture_registry(packages).await;
        let tmp = tempfile::tempdir().expect("project");
        tokio::task::spawn_blocking({
            let root = tmp.path().to_path_buf();
            let registry = registry.clone();
            move || {
                run_php_install_in(
                    vec!["@scope/a@^1.0.0".to_string()],
                    true,
                    &root,
                    &registry,
                    None,
                )
            }
        })
        .await
        .expect("installer task")
        .expect("install graph");
        let modules = tmp.path().join("php_modules").join("@scope");
        for package in ["a", "b", "c"] {
            assert!(modules.join(package).is_dir(), "missing flat {package}");
            assert!(!modules.join(package).join("php_modules").exists());
        }
        let lock = lock::read_lockfile_at(&tmp.path().join("deka.lock"));
        assert_eq!(lock.packages.len(), 3);
        for package in ["@scope/a", "@scope/b", "@scope/c"] {
            let (_, _, metadata, _) = lock.packages.get(package).expect("lock entry");
            assert!(metadata["moduleGraph"]["hash"].as_str().is_some());
            assert!(metadata["fsGraph"]["hash"].as_str().is_some());
        }
        shutdown.send(()).expect("shutdown registry");
    }

    #[test]
    fn install_places_deka_packages_at_scoped_path() {
        let tmp = tempfile::tempdir().expect("project");
        run_php_install_in(
            vec!["@deka/http".to_string()],
            true,
            tmp.path(),
            "http://127.0.0.1:1",
            None,
        )
        .expect("bundled deka install");
        assert!(
            tmp.path()
                .join("php_modules/@deka/http/index.phpx")
                .is_file()
        );
        assert!(!tmp.path().join("php_modules/http").exists());
    }

    #[tokio::test]
    async fn install_rejects_disjoint_transitive_version_constraints() {
        let packages = BTreeMap::from([
            ("@scope/a".to_string(), json!({ "@scope/c": "^1.0.0" })),
            ("@scope/b".to_string(), json!({ "@scope/c": "^2.0.0" })),
            ("@scope/c".to_string(), json!({})),
        ]);
        let (registry, shutdown) = fixture_registry(packages).await;
        let tmp = tempfile::tempdir().expect("project");
        let error = tokio::task::spawn_blocking({
            let root = tmp.path().to_path_buf();
            let registry = registry.clone();
            move || {
                run_php_install_in(
                    vec!["@scope/a".to_string(), "@scope/b".to_string()],
                    true,
                    &root,
                    &registry,
                    None,
                )
            }
        })
        .await
        .expect("installer task")
        .expect_err("disjoint ranges must fail");
        assert!(
            error.to_string().contains(
                "version conflict: @scope/c required as ^1.0.0 by @scope/a and ^2.0.0 by @scope/b"
            ),
            "{error}"
        );
        shutdown.send(()).expect("shutdown registry");
    }

    #[tokio::test]
    async fn install_uses_one_highest_version_for_overlapping_constraints() {
        let packages = BTreeMap::from([
            ("@scope/a".to_string(), json!({ "@scope/c": "^1.1.0" })),
            ("@scope/b".to_string(), json!({ "@scope/c": "^1.3.0" })),
            ("@scope/c".to_string(), json!({})),
        ]);
        let (registry, shutdown) = fixture_registry(packages).await;
        let tmp = tempfile::tempdir().expect("project");
        tokio::task::spawn_blocking({
            let root = tmp.path().to_path_buf();
            let registry = registry.clone();
            move || {
                run_php_install_in(
                    vec!["@scope/a".to_string(), "@scope/b".to_string()],
                    true,
                    &root,
                    &registry,
                    None,
                )
            }
        })
        .await
        .expect("installer task")
        .expect("overlap installs");
        let lock = lock::read_lockfile_at(&tmp.path().join("deka.lock"));
        assert_eq!(lock.packages.len(), 3);
        assert_eq!(lock.packages["@scope/c"].0, "@scope/c@1.4.0");
        shutdown.send(()).expect("shutdown registry");
    }

    async fn fixture_registry(
        packages: BTreeMap<String, serde_json::Value>,
    ) -> (String, tokio::sync::oneshot::Sender<()>) {
        use axum::{
            Json, Router,
            extract::{Path as AxumPath, Query, State},
            routing::get,
        };
        use std::{collections::HashMap, sync::Arc};
        #[derive(Clone)]
        struct Fixture(Arc<BTreeMap<String, serde_json::Value>>);
        async fn versions(
            AxumPath((_scope, package)): AxumPath<(String, String)>,
        ) -> Json<serde_json::Value> {
            let versions = if package == "c" {
                vec!["1.1.0", "1.3.0", "1.4.0", "2.0.0"]
            } else {
                vec!["1.0.0"]
            };
            Json(json!({ "versions": versions }))
        }
        async fn resolve(
            AxumPath((_scope, _package, version)): AxumPath<(String, String, String)>,
        ) -> Json<serde_json::Value> {
            Json(json!({ "version": version, "repo": "fixture", "git_ref": "fixture" }))
        }
        async fn tree(
            AxumPath((scope, package, _version)): AxumPath<(String, String, String)>,
            State(Fixture(packages)): State<Fixture>,
        ) -> Json<serde_json::Value> {
            let name = format!("@{scope}/{package}");
            assert!(
                packages.contains_key(&name),
                "unknown fixture package {name}"
            );
            Json(json!({ "files": [{ "path": "deka.json" }, { "path": "index.phpx" }] }))
        }
        async fn blob(
            AxumPath((scope, package, version)): AxumPath<(String, String, String)>,
            Query(query): Query<HashMap<String, String>>,
            State(Fixture(packages)): State<Fixture>,
        ) -> Json<serde_json::Value> {
            let name = format!("@{scope}/{package}");
            let content = if query.get("path").is_some_and(|path| path == "deka.json") {
                json!({ "name": name, "version": version, "dependencies": packages[&name] })
                    .to_string()
            } else {
                "export const fixture = true;\n".to_string()
            };
            Json(json!({ "content": content }))
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("registry listener");
        let address = listener.local_addr().expect("registry address");
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let app = Router::new()
            .route(
                "/api/scoped-packages/:scope/:package/versions",
                get(versions),
            )
            .route(
                "/api/scoped-packages/:scope/:package/:version",
                get(resolve),
            )
            .route(
                "/api/scoped-packages/:scope/:package/:version/tree",
                get(tree),
            )
            .route(
                "/api/scoped-packages/:scope/:package/:version/blob",
                get(blob),
            )
            .with_state(Fixture(Arc::new(packages)));
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("fixture registry");
        });
        (format!("http://{address}"), shutdown_tx)
    }
}

pub fn run_probe(path: &PathBuf) -> Result<()> {
    let canonical = fs::canonicalize(path).context("failed to resolve probe path")?;
    emit_probe(&canonical)?;
    Ok(())
}

fn emit_summary(installed: usize, duration_ms: u64, quiet: bool) -> Result<()> {
    if quiet {
        return Ok(());
    }

    if installed > 0 {
        if installed == 1 {
            eprintln!(" 1 package installed [{:.2}ms]", duration_ms);
        } else {
            eprintln!(" {} packages installed [{:.2}ms]", installed, duration_ms);
        }
    }
    Ok(())
}

fn emit_probe(path: &PathBuf) -> Result<()> {
    eprintln!("📁 {}", path.display());
    Ok(())
}
