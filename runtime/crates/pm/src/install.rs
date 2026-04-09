use crate::{
    bun_lock::BunLock,
    cache::{
        CachePaths, cache_key, compute_sha512, copy_package, download_tarball, extract_tarball,
    },
    lock,
    npm::{fetch_npm_metadata, resolve_package_version},
    payload::InstallPayload,
    spec::{Ecosystem, parse_hinted_spec, parse_package_spec},
};
use anyhow::{Context, Result, bail, anyhow};
use runtime_core::module_spec::canonical_php_package_spec;
use runtime_core::security_policy::{RuleList, SecurityPolicy, parse_deka_security_policy};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use modules_php::integrity::compute_package_integrity;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::Instant,
};
use tokio::{sync::Semaphore, task::JoinSet};

pub async fn run_install(payload: InstallPayload) -> Result<()> {
    if payload.rehash {
        rehash_php_packages(&payload).await?;
        return Ok(());
    }
    let mut specs = payload.specs.clone();
    let auto_resolved = specs.is_empty();
    if auto_resolved {
        specs = collect_project_specs()?;
    }

    let override_ecosystem = payload
        .ecosystem
        .as_deref()
        .and_then(Ecosystem::from_str)
        .unwrap_or(Ecosystem::Node);
    let quiet = payload.quiet;

    if override_ecosystem == Ecosystem::Php {
        return run_php_install(specs, quiet).await;
    }

    if payload.prompt && !payload.yes {
        let message = if auto_resolved {
            "Install dependencies listed in package.json? [y/N]:"
        } else {
            "Install requested dependencies? [y/N]:"
        };
        if !prompt_yes_no(message)? {
            bail!("installation aborted by user");
        }
    }

    let cache = Arc::new(CachePaths::new()?);
    cache.ensure()?;

    let bun_lock = BunLock::load()?.map(Arc::new);
    let mut ctx = InstallContext::new(cache.clone(), bun_lock.clone());
    for spec in specs {
        ctx.enqueue(spec, None, false, None);
    }

    let start = Instant::now();
    let semaphore = Arc::new(Semaphore::new(100)); // Allow many concurrent downloads like Bun
    let mut join_set = JoinSet::new();
    let mut copy_tasks = JoinSet::new();
    let mut installed_count = 0;

    // Spawn initial tasks
    while let Some(task) = ctx.next_task() {
        let (ecosystem, spec_str) = parse_hinted_spec(&task.spec, Some(override_ecosystem))?;
        if ecosystem != Ecosystem::Node {
            if !quiet {
                eprintln!(
                    "⚠️  Skipping unsupported ecosystem {} for {}",
                    ecosystem.as_str(),
                    spec_str
                );
            }
            continue;
        }

        let cache_clone = cache.clone();
        let bun_lock_clone = bun_lock.clone();
        let spec_clone = spec_str.clone();
        let lock_key_clone = task.lock_key.clone();
        let sem_clone = semaphore.clone();
        let optional = task.optional;

        join_set.spawn(async move {
            let _permit = sem_clone.acquire().await.unwrap();
            let result = install_node_package(
                &cache_clone,
                bun_lock_clone.as_deref(),
                &spec_clone,
                lock_key_clone.as_deref(),
            )
            .await;
            (spec_clone, optional, result)
        });
    }

    // Process all tasks as they complete
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok((_spec, _optional, Ok(result))) => {
                // Copy package to final location (spawn in parallel)
                let destination = ctx.determine_install_path(
                    &result.name,
                    &result.version,
                    result.lock_key.as_deref(),
                );
                if let Some(dest) = destination {
                    let cache_dir = result.cache_dir.clone();
                    let name = result.name.clone();
                    let version = result.version.clone();
                    let resolved = result.resolved.clone();
                    let metadata = result.metadata.clone();
                    let integrity = result.integrity.clone();
                    let dest_clone = dest.clone();

                    // Spawn copy operation without awaiting (runs in parallel)
                    copy_tasks.spawn_blocking(move || -> Result<()> {
                        copy_package(&cache_dir, &dest_clone)?;
                        let descriptor = format!("{}@{}", name, version);
                        lock::update_lock_entry(
                            "node", &name, descriptor, resolved, metadata, integrity,
                        )?;
                        Ok(())
                    });

                    installed_count += 1;
                }

                // Enqueue dependencies
                for dep in result.dependencies {
                    ctx.enqueue(
                        dep.spec.clone(),
                        Some(dep.descriptor.clone()),
                        false,
                        Some(dep.lock_key.clone()),
                    );
                }
                for opt in result.optional_dependencies {
                    if ctx.should_install_optional(&opt.lock_key, &opt.spec) {
                        ctx.enqueue(
                            opt.spec.clone(),
                            Some(opt.descriptor.clone()),
                            true,
                            Some(opt.lock_key),
                        );
                    }
                }
                for peer in result.optional_peers {
                    if let Some(lock) = bun_lock.as_deref() {
                        if lock.get(&peer.lock_key).is_some() {
                            ctx.enqueue(
                                peer.spec.clone(),
                                Some(peer.descriptor.clone()),
                                true,
                                Some(peer.lock_key),
                            );
                        }
                    }
                }
            }
            Ok((spec, optional, Err(err))) => {
                if optional {
                    if !quiet {
                        eprintln!("⚠️  Skipping optional {}: {}", spec, err);
                    }
                } else {
                    return Err(err);
                }
            }
            Err(e) => return Err(anyhow::anyhow!("task join error: {}", e)),
        }

        // Spawn ALL queued tasks after processing each completed task
        while let Some(task) = ctx.next_task() {
            let (ecosystem, spec_str) = parse_hinted_spec(&task.spec, Some(override_ecosystem))?;
            if ecosystem != Ecosystem::Node {
                if !quiet {
                    eprintln!(
                        "⚠️  Skipping unsupported ecosystem {} for {}",
                        ecosystem.as_str(),
                        spec_str
                    );
                }
                continue;
            }

            let cache_c = cache.clone();
            let lock_c = bun_lock.clone();
            let spec_c = spec_str.clone();
            let key_c = task.lock_key.clone();
            let sem_c = semaphore.clone();
            let opt = task.optional;

            join_set.spawn(async move {
                let _permit = sem_c.acquire().await.unwrap();
                let result =
                    install_node_package(&cache_c, lock_c.as_deref(), &spec_c, key_c.as_deref())
                        .await;
                (spec_c, opt, result)
            });
        }
    }

    // Process remaining tasks
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok((_spec, _optional, Ok(result))) => {
                let destination = ctx.determine_install_path(
                    &result.name,
                    &result.version,
                    result.lock_key.as_deref(),
                );
                if let Some(dest) = destination {
                    let cache_dir = result.cache_dir.clone();
                    let name = result.name.clone();
                    let version = result.version.clone();
                    let resolved = result.resolved.clone();
                    let metadata = result.metadata.clone();
                    let integrity = result.integrity.clone();

                    copy_tasks.spawn_blocking(move || -> Result<()> {
                        copy_package(&cache_dir, &dest)?;
                        let descriptor = format!("{}@{}", name, version);
                        lock::update_lock_entry(
                            "node", &name, descriptor, resolved, metadata, integrity,
                        )?;
                        Ok(())
                    });

                    installed_count += 1;
                }
            }
            Ok((spec, optional, Err(err))) => {
                if optional {
                    if !quiet {
                        eprintln!("⚠️  Skipping optional {}: {}", spec, err);
                    }
                } else {
                    return Err(err);
                }
            }
            Err(e) => return Err(anyhow::anyhow!("task join error: {}", e)),
        }
    }

    // Wait for all copy operations to complete
    while let Some(res) = copy_tasks.join_next().await {
        match res {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(anyhow::anyhow!("copy task join error: {}", e)),
        }
    }

    let duration = Instant::now().duration_since(start);
    emit_summary(installed_count, duration.as_millis() as u64, quiet)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct PhpPackageSummary {
    latest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PhpPackageRelease {
    package_name: String,
    version: String,
    owner: String,
    repo: String,
    git_ref: String,
    description: Option<String>,
    manifest: Option<Value>,
    capability_metadata: Option<Value>,
}

async fn run_php_install(specs: Vec<String>, quiet: bool) -> Result<()> {
    if specs.is_empty() {
        bail!("no PHP package specs provided (use --spec)");
    }

    let cache = CachePaths::new()?;
    cache.ensure()?;
    let registry = linkhash_registry_url();
    let project_policy = load_project_security_policy()?;
    let start = Instant::now();
    let mut installed_count = 0usize;

    for spec in specs {
        let normalized = normalize_php_spec(&spec)?;
        let (name, version_hint) = parse_package_spec(&normalized);
        let version = if let Some(v) = version_hint {
            v
        } else {
            fetch_php_latest(&registry, &name).await?
        };

        let release = fetch_php_release(&registry, &name, &version).await?;
        enforce_release_policy(&release, &project_policy)?;
        let tarball_url = package_download_url(&registry, &name, &version)?;
        let bytes = download_tarball(&tarball_url).await?;
        let integrity = compute_sha512(&bytes);

        let key = cache_key(&format!("php+{}", name), &version);
        let archive_path = cache.archive_path(&key);
        fs::write(&archive_path, &bytes)
            .with_context(|| format!("failed to write archive {}", archive_path.display()))?;

        let cache_dir = cache.cache_dir(&key);
        extract_tarball(&archive_path, &cache_dir, &cache.tmp)?;

        let destination = php_modules_path_for(&name)?;
        copy_package(&cache_dir, &destination)?;

        let package_integrity = compute_package_integrity(&destination)
            .map_err(|err| anyhow!("failed to compute package integrity for {}: {}", name, err))?;

        let metadata = json!({
            "owner": release.owner,
            "repo": release.repo,
            "gitRef": release.git_ref,
            "description": release.description,
            "manifest": release.manifest,
            "moduleGraph": {
                "algo": "sha256",
                "hash": package_integrity.module_graph,
            },
            "fsGraph": {
                "algo": "sha256",
                "hash": package_integrity.fs_graph,
            },
        });
        lock::update_lock_entry(
            "php",
            &release.package_name,
            format!("{}@{}", release.package_name, release.version),
            tarball_url,
            metadata,
            integrity,
        )?;
        installed_count += 1;
    }

    let duration = Instant::now().duration_since(start);
    emit_summary(installed_count, duration.as_millis() as u64, quiet)?;
    Ok(())
}

async fn rehash_php_packages(payload: &InstallPayload) -> Result<()> {
    let lock = lock::read_lockfile();
    let mut specs = payload.specs.clone();
    if specs.is_empty() {
        specs = lock.php.packages.keys().cloned().collect();
    }
    if specs.is_empty() {
        bail!("no PHP packages found to rehash");
    }

    for name in specs {
        let package_root = php_modules_path_for(&name)?;
        if !package_root.is_dir() {
            bail!(
                "package '{}' is missing from php_modules (expected {})",
                name,
                package_root.display()
            );
        }
        let integrity = compute_package_integrity(&package_root)
            .map_err(|err| anyhow!("failed to compute package integrity for {}: {}", name, err))?;
        let entry = lock.php.packages.get(&name).cloned();
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
        lock::update_lock_entry(
            "php",
            &name,
            descriptor,
            resolved,
            metadata,
            integrity_field,
        )?;
    }

    Ok(())
}

fn load_project_security_policy() -> Result<SecurityPolicy> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    let path = cwd.join("deka.json");
    if !path.is_file() {
        return Ok(SecurityPolicy::default());
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let json: Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    let parsed = parse_deka_security_policy(&json);
    if parsed.has_errors() {
        let details = parsed
            .diagnostics
            .into_iter()
            .filter(|d| {
                matches!(
                    d.level,
                    runtime_core::security_policy::PolicyDiagnosticLevel::Error
                )
            })
            .map(|d| format!("{} at {}: {}", d.code, d.path, d.message))
            .collect::<Vec<_>>()
            .join("\n");
        bail!("invalid security policy:\n{}", details);
    }
    Ok(parsed.policy)
}

fn enforce_release_policy(release: &PhpPackageRelease, policy: &SecurityPolicy) -> Result<()> {
    let capabilities = extract_release_capabilities(release);
    if capabilities.iter().any(|cap| cap == "run") && !matches!(policy.deny.run, RuleList::None) {
        bail!(
            "install blocked by security policy: package {}@{} requires `run` capability",
            release.package_name,
            release.version
        );
    }
    if capabilities.iter().any(|cap| cap == "dynamic") && policy.deny.dynamic {
        bail!(
            "install blocked by security policy: package {}@{} requires `dynamic` capability",
            release.package_name,
            release.version
        );
    }
    Ok(())
}

fn extract_release_capabilities(release: &PhpPackageRelease) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(meta) = &release.capability_metadata {
        if let Some(detected) = meta.get("detected").and_then(|v| v.as_array()) {
            for cap in detected.iter().filter_map(|v| v.as_str()) {
                if !out.iter().any(|existing| existing == cap) {
                    out.push(cap.to_string());
                }
            }
        }
    }
    if let Some(manifest) = &release.manifest {
        let allow = manifest
            .get("security")
            .and_then(|v| v.get("allow"))
            .cloned()
            .unwrap_or(Value::Null);
        if allow
            .get("dynamic")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && !out.iter().any(|existing| existing == "dynamic")
        {
            out.push("dynamic".to_string());
        }
        let run_enabled = allow
            .get("run")
            .map(|run| match run {
                Value::Bool(v) => *v,
                Value::String(s) => !s.trim().is_empty(),
                Value::Array(arr) => arr
                    .iter()
                    .any(|item| item.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false)),
                _ => false,
            })
            .unwrap_or(false);
        if run_enabled && !out.iter().any(|existing| existing == "run") {
            out.push("run".to_string());
        }
    }
    out
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
    std::env::var("LINKHASH_REGISTRY_URL").unwrap_or_else(|_| "http://localhost:8508".to_string())
}

async fn fetch_php_latest(registry: &str, name: &str) -> Result<String> {
    let url = package_summary_url(registry, name)?;

    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("failed to fetch package summary for {}", name))?;
    if !response.status().is_success() {
        bail!(
            "package summary request failed ({}): {}",
            response.status(),
            name
        );
    }
    let payload = response
        .json::<PhpPackageSummary>()
        .await
        .context("failed to parse package summary")?;
    payload
        .latest
        .context("no latest version found for package")
}

async fn fetch_php_release(registry: &str, name: &str, version: &str) -> Result<PhpPackageRelease> {
    let url = package_release_url(registry, name, version)?;
    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("failed to fetch package release {}@{}", name, version))?;
    if !response.status().is_success() {
        bail!(
            "package release request failed ({}): {}@{}",
            response.status(),
            name,
            version
        );
    }
    response
        .json::<PhpPackageRelease>()
        .await
        .context("failed to parse package release")
}

fn package_summary_url(registry: &str, name: &str) -> Result<String> {
    let (scope, pkg) = parse_scoped_package(name)?;
    Ok(format!(
        "{}/api/scoped-packages/{}/{}",
        registry.trim_end_matches('/'),
        urlencoding::encode(scope),
        urlencoding::encode(pkg)
    ))
}

fn package_release_url(registry: &str, name: &str, version: &str) -> Result<String> {
    let (scope, pkg) = parse_scoped_package(name)?;
    Ok(format!(
        "{}/api/scoped-packages/{}/{}/{}",
        registry.trim_end_matches('/'),
        urlencoding::encode(scope),
        urlencoding::encode(pkg),
        urlencoding::encode(version)
    ))
}

fn package_download_url(registry: &str, name: &str, version: &str) -> Result<String> {
    let (scope, pkg) = parse_scoped_package(name)?;
    Ok(format!(
        "{}/api/scoped-packages/{}/{}/{}/download",
        registry.trim_end_matches('/'),
        urlencoding::encode(scope),
        urlencoding::encode(pkg),
        urlencoding::encode(version)
    ))
}

fn parse_scoped_package(name: &str) -> Result<(&str, &str)> {
    if !name.starts_with('@') {
        bail!("invalid package name `{}`: expected @scope/name", name);
    }
    let mut parts = name.splitn(3, '/');
    let scope = parts.next().unwrap_or("");
    let pkg = parts.next().unwrap_or("");
    if scope.is_empty() || pkg.is_empty() || parts.next().is_some() {
        bail!("invalid package name `{}`: expected @scope/name", name);
    }
    Ok((scope, pkg))
}

fn php_modules_path_for(package_name: &str) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    let mut path = cwd.join("php_modules");
    let segments: Vec<&str> = if let Some(rest) = package_name.strip_prefix("@deka/") {
        vec![rest]
    } else {
        package_name.split('/').collect()
    };
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
        PhpPackageRelease, enforce_release_policy, extract_release_capabilities, php_modules_path_for,
    };
    use runtime_core::security_policy::{RuleList, SecurityPolicy, SecurityScope};
    use serde_json::json;

    fn sample_release(
        manifest: Option<serde_json::Value>,
        capability_metadata: Option<serde_json::Value>,
    ) -> PhpPackageRelease {
        PhpPackageRelease {
            package_name: "@scope/pkg".to_string(),
            version: "1.0.0".to_string(),
            owner: "scope".to_string(),
            repo: "pkg".to_string(),
            git_ref: "HEAD".to_string(),
            description: None,
            manifest,
            capability_metadata,
        }
    }

    #[test]
    fn extracts_capabilities_from_metadata_and_manifest() {
        let release = sample_release(
            Some(json!({
                "security": {
                    "allow": {
                        "dynamic": true,
                        "run": ["git"]
                    }
                }
            })),
            Some(json!({
                "detected": ["run"]
            })),
        );
        let caps = extract_release_capabilities(&release);
        assert!(caps.iter().any(|c| c == "run"));
        assert!(caps.iter().any(|c| c == "dynamic"));
    }

    #[test]
    fn blocks_install_when_policy_denies_detected_capability() {
        let release = sample_release(
            None,
            Some(json!({
                "detected": ["dynamic"]
            })),
        );
        let policy = SecurityPolicy {
            allow: SecurityScope::default(),
            deny: SecurityScope {
                dynamic: true,
                ..SecurityScope::default()
            },
            prompt: true,
        };
        let result = enforce_release_policy(&release, &policy);
        assert!(result.is_err());
    }

    #[test]
    fn allows_install_when_policy_has_no_denies() {
        let release = sample_release(
            Some(json!({
                "security": {
                    "allow": { "run": ["git"] }
                }
            })),
            None,
        );
        let policy = SecurityPolicy {
            allow: SecurityScope::default(),
            deny: SecurityScope {
                run: RuleList::None,
                ..SecurityScope::default()
            },
            prompt: true,
        };
        let result = enforce_release_policy(&release, &policy);
        assert!(result.is_ok());
    }

    #[test]
    fn deka_scoped_package_installs_to_unscoped_php_modules_path() {
        let cwd = std::env::current_dir().expect("cwd");
        let path = php_modules_path_for("@deka/component").expect("path");
        assert_eq!(path, cwd.join("php_modules").join("component"));
    }
}

pub fn run_probe(path: &PathBuf) -> Result<()> {
    let canonical = fs::canonicalize(path).context("failed to resolve probe path")?;
    emit_probe(&canonical)?;
    Ok(())
}

async fn install_node_package(
    cache: &CachePaths,
    bun_lock: Option<&BunLock>,
    spec: &str,
    lock_key: Option<&str>,
) -> Result<InstallResult> {
    let (name, version_spec) = parse_package_spec(spec);
    let metadata = fetch_npm_metadata(&name).await?;
    let lock_entry = bun_lock.and_then(|lock| lock.lookup(lock_key, &name));
    let version = lock_entry
        .map(|entry| entry.version.clone())
        .or_else(|| resolve_package_version(&metadata, version_spec.as_deref()))
        .context("unable to resolve package version")?;

    let version_info = metadata
        .get("versions")
        .and_then(|versions| versions.get(&version))
        .context("missing version info")?;
    let key = cache_key(&name, &version);
    let cache_dir = cache.cache_dir(&key);

    let (integrity, resolved) = if cache_dir.exists() {
        let meta_path = cache.metadata_path(&key);
        let meta = crate::cache::read_metadata(&meta_path);
        let integrity = meta
            .as_ref()
            .and_then(|value| value.get("integrity"))
            .and_then(|value| value.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                version_info
                    .get("dist")
                    .and_then(|dist| dist.get("integrity"))
                    .and_then(|value| value.as_str())
                    .map(|s| s.to_string())
            })
            .context("integrity unavailable")?;
        let resolved = meta
            .as_ref()
            .and_then(|value| value.get("resolved"))
            .and_then(|value| value.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                version_info
                    .get("dist")
                    .and_then(|dist| dist.get("tarball"))
                    .and_then(|value| value.as_str())
                    .map(|s| s.to_string())
            })
            .context("resolved URL missing")?;
        (integrity, resolved)
    } else {
        let tarball_url = version_info
            .get("dist")
            .and_then(|dist| dist.get("tarball"))
            .and_then(|value| value.as_str())
            .context("tarball URL missing")?;
        let bytes = download_tarball(tarball_url).await?;
        let integrity = version_info
            .get("dist")
            .and_then(|dist| dist.get("integrity"))
            .and_then(|value| value.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| compute_sha512(&bytes));
        let archive_path = cache.archive_path(&key);
        fs::write(&archive_path, &bytes)?;
        extract_tarball(&archive_path, &cache_dir, &cache.tmp)?;
        let meta = json!({
            "integrity": integrity,
            "resolved": tarball_url,
            "version": version,
        });
        crate::cache::write_metadata(&cache.metadata_path(&key), &meta)?;
        (integrity, tarball_url.to_string())
    };

    let dependencies = build_dependency_specs(
        lock_key,
        collect_spec_list(version_info.get("dependencies")),
        bun_lock,
    );
    let optional_dependencies = build_dependency_specs(
        lock_key,
        collect_spec_list(version_info.get("optionalDependencies")),
        bun_lock,
    );

    let optional_peers = if let Some(entry) = lock_entry {
        let peer_names = collect_optional_peers(Some(&entry.metadata));
        let peer_deps = version_info.get("peerDependencies");
        let peer_specs: Vec<String> = peer_names
            .into_iter()
            .filter_map(|peer_name| {
                if let Some(Value::Object(peers)) = peer_deps {
                    peers.get(&peer_name).and_then(|v| v.as_str()).map(|range| {
                        if std::env::var("DEKA_DEBUG").is_ok() {
                            eprintln!("[DEBUG] optionalPeer found: {}@{}", peer_name, range);
                        }
                        format!("{peer_name}@{range}")
                    })
                } else {
                    None
                }
            })
            .collect();
        build_dependency_specs(lock_key, peer_specs, bun_lock)
    } else {
        Vec::new()
    };

    Ok(InstallResult {
        name: name.to_string(),
        version,
        cache_dir,
        lock_key: lock_key.map(|s| s.to_string()),
        integrity,
        resolved,
        metadata: build_lock_metadata(version_info),
        dependencies,
        optional_dependencies,
        optional_peers,
    })
}

fn collect_spec_list(value: Option<&Value>) -> Vec<String> {
    if let Some(Value::Object(map)) = value {
        map.iter()
            .filter_map(|(name, version)| version.as_str().map(|range| format!("{name}@{range}")))
            .collect()
    } else {
        Vec::new()
    }
}

fn collect_optional_peers(lock_entry: Option<&Value>) -> Vec<String> {
    if let Some(entry) = lock_entry {
        if let Some(Value::Array(peers)) = entry.get("optionalPeers") {
            return peers
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
    }
    Vec::new()
}

fn build_lock_metadata(value: &Value) -> Value {
    let mut map = Map::new();
    for key in [
        "dependencies",
        "peerDependencies",
        "optionalDependencies",
        "bin",
    ] {
        if let Some(val) = value.get(key) {
            if !val.is_null() {
                map.insert(key.to_string(), val.clone());
            }
        }
    }
    Value::Object(map)
}

fn collect_project_specs() -> Result<Vec<String>> {
    let manifest = fs::read_to_string("package.json")
        .context("package.json not found in current directory")?;
    let pkg_json: Value =
        serde_json::from_str(&manifest).context("failed to parse package.json")?;
    let mut deps = BTreeMap::new();
    for section in ["dependencies", "devDependencies"] {
        if let Some(Value::Object(map)) = pkg_json.get(section) {
            for (name, value) in map {
                if let Some(version) = value.as_str() {
                    deps.insert(name.clone(), version.to_string());
                }
            }
        }
    }

    let lock = lock::read_lockfile();
    let mut specs = Vec::new();
    for (name, version) in deps {
        if let Some(entry) = lock.node.packages.get(&name) {
            specs.push(entry.0.clone());
        } else if !version.trim().is_empty() {
            specs.push(format!("{name}@{version}"));
        } else {
            specs.push(name.clone());
        }
    }
    Ok(specs)
}

fn prompt_yes_no(message: &str) -> Result<bool> {
    print!("  {} ", message);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let normalized = input.trim().to_lowercase();
    Ok(normalized == "y" || normalized == "yes")
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

struct InstallContext {
    cache: Arc<CachePaths>,
    queue: VecDeque<InstallTask>,
    scheduled: HashSet<String>,
    installed: HashSet<String>,
    bun_lock: Option<Arc<BunLock>>,
    current_os: String,
    current_cpu: String,
}

struct InstallTask {
    spec: String,
    optional: bool,
    lock_key: Option<String>,
}

impl InstallContext {
    fn new(cache: Arc<CachePaths>, bun_lock: Option<Arc<BunLock>>) -> Self {
        Self {
            cache,
            queue: VecDeque::new(),
            scheduled: HashSet::new(),
            installed: HashSet::new(),
            bun_lock,
            current_os: normalize_os(),
            current_cpu: normalize_cpu(),
        }
    }

    fn enqueue(
        &mut self,
        spec: String,
        descriptor: Option<String>,
        optional: bool,
        lock_key: Option<String>,
    ) {
        let key = descriptor
            .as_deref()
            .map(|desc| desc.to_string())
            .unwrap_or_else(|| build_task_key(&spec, lock_key.as_deref()));
        if self.scheduled.insert(key) {
            self.queue.push_back(InstallTask {
                spec,
                optional,
                lock_key,
            });
        }
    }

    fn next_task(&mut self) -> Option<InstallTask> {
        self.queue.pop_front()
    }

    fn should_install_optional(&self, lock_key: &str, spec: &str) -> bool {
        if let Some(lock) = self.bun_lock.as_deref() {
            let (name, _) = parse_package_spec(spec);
            if let Some(entry) = lock.lookup(Some(lock_key), &name) {
                if !matches_requirement(entry.metadata.get("os"), &self.current_os) {
                    return false;
                }
                if !matches_requirement(entry.metadata.get("cpu"), &self.current_cpu) {
                    return false;
                }
            }
        }
        true
    }

    fn determine_install_path(
        &self,
        name: &str,
        version: &str,
        lock_key: Option<&str>,
    ) -> Option<PathBuf> {
        let target_path = if let Some(key) = lock_key {
            let is_scoped = key.starts_with('@');
            let segments: Vec<&str> = key.split('/').collect();
            let is_nested = if is_scoped {
                segments.len() > 2
            } else {
                segments.len() > 1
            };

            if is_nested {
                let mut path = self.cache.node_modules.clone();
                let parent_segments = if is_scoped {
                    &segments[..segments.len() - 1]
                } else {
                    &segments[..segments.len() - 1]
                };

                for segment in parent_segments {
                    path = path.join(segment);
                }
                path = path.join("node_modules");

                let name_segments: Vec<&str> = name.split('/').collect();
                for segment in name_segments {
                    path = path.join(segment);
                }
                path
            } else {
                self.cache.project_path_for(name)
            }
        } else {
            self.cache.project_path_for(name)
        };

        let path_str = target_path.to_string_lossy();
        let install_key = format!("{}:{}@{}", path_str, name, version);

        if self.installed.contains(&install_key) {
            if std::env::var("DEKA_DEBUG").is_ok() {
                eprintln!(
                    "[DEBUG] {} already installed at {:?}, skipping",
                    install_key, target_path
                );
            }
            return None;
        }

        if std::env::var("DEKA_DEBUG").is_ok() {
            eprintln!(
                "[DEBUG] {} (lock_key={:?}) -> {:?}",
                name, lock_key, target_path
            );
        }
        Some(target_path)
    }
}

struct DependencySpec {
    spec: String,
    lock_key: String,
    descriptor: String,
}

struct InstallResult {
    name: String,
    version: String,
    cache_dir: PathBuf,
    lock_key: Option<String>,
    integrity: String,
    resolved: String,
    metadata: Value,
    dependencies: Vec<DependencySpec>,
    optional_dependencies: Vec<DependencySpec>,
    optional_peers: Vec<DependencySpec>,
}

fn normalize_os() -> String {
    match std::env::consts::OS {
        "macos" => "darwin".to_string(),
        "windows" => "win32".to_string(),
        other => other.to_string(),
    }
}

fn normalize_cpu() -> String {
    match std::env::consts::ARCH {
        "aarch64" => "arm64".to_string(),
        "x86_64" => "x64".to_string(),
        "x86" | "i586" => "ia32".to_string(),
        other => other.to_string(),
    }
}

fn matches_requirement(value: Option<&Value>, current: &str) -> bool {
    match value {
        Some(Value::String(expected)) => expected == "none" || expected == current,
        Some(Value::Array(list)) => list
            .iter()
            .any(|item| matches_requirement(Some(item), current)),
        _ => true,
    }
}

fn build_task_key(spec: &str, lock_key: Option<&str>) -> String {
    lock_key
        .map(|key| key.to_string())
        .unwrap_or_else(|| spec.to_string())
}

fn build_child_lock_key(parent: Option<&str>, name: &str) -> String {
    if let Some(parent_key) = parent {
        format!("{parent_key}/{name}")
    } else {
        name.to_string()
    }
}

fn build_dependency_specs(
    parent_lock_key: Option<&str>,
    specs: Vec<String>,
    bun_lock: Option<&BunLock>,
) -> Vec<DependencySpec> {
    specs
        .into_iter()
        .map(|spec| {
            let (name, _) = parse_package_spec(&spec);
            let (lock_key, descriptor) = if let Some(lock) = bun_lock {
                if let Some(parent) = parent_lock_key {
                    let nested_key = build_child_lock_key(Some(parent), &name);
                    if let Some(entry) = lock.get(&nested_key) {
                        if std::env::var("DEKA_DEBUG").is_ok() {
                            eprintln!("[DEBUG] {}: nested key '{}' found", name, nested_key);
                        }
                        (nested_key, entry.descriptor.clone())
                    } else if let Some(entry) = lock.get(&name) {
                        if std::env::var("DEKA_DEBUG").is_ok() {
                            eprintln!(
                                "[DEBUG] {}: using top-level key '{}' (parent={})",
                                name, name, parent
                            );
                        }
                        (name.clone(), entry.descriptor.clone())
                    } else {
                        if std::env::var("DEKA_DEBUG").is_ok() {
                            eprintln!("[DEBUG] {}: not in lockfile", name);
                        }
                        (name.clone(), spec.clone())
                    }
                } else {
                    if let Some(entry) = lock.get(&name) {
                        (name.clone(), entry.descriptor.clone())
                    } else {
                        (name.clone(), spec.clone())
                    }
                }
            } else {
                (name.clone(), spec.clone())
            };

            DependencySpec {
                spec,
                lock_key,
                descriptor,
            }
        })
        .collect()
}
