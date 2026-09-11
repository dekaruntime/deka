use crate::{grants, lock, payload::InstallPayload, registry, spec::parse_package_spec};
use anyhow::{Context, Result, anyhow, bail};
use deka_host::integrity::compute_package_integrity;
use runtime_core::module_spec::canonical_php_package_spec;
use runtime_core::modules::{MODULES_DIR, install_modules_dir, is_modules_dir_name, read_links_at};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const STAGED_MARKER: &str = ".deka-staged-package";

pub async fn run_install(payload: InstallPayload) -> Result<()> {
    if payload.rehash {
        rehash_php_packages(&payload).await?;
        return Ok(());
    }

    let specs = payload.specs.clone();
    let quiet = payload.quiet;
    let locked = payload.locked;
    tokio::task::spawn_blocking(move || run_php_install(specs, quiet, locked))
        .await
        .context("install task failed")?
}

fn run_php_install(specs: Vec<String>, quiet: bool, locked: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    run_php_install_in(specs, quiet, locked, &cwd)
}

fn run_php_install_in(specs: Vec<String>, quiet: bool, locked: bool, cwd: &Path) -> Result<()> {
    recover_install_transaction(cwd)?;
    let result = run_php_install_in_transaction(specs, quiet, locked, cwd);
    if let Err(error) = result {
        let recovery = recover_install_transaction(cwd);
        return match recovery {
            Ok(()) => Err(error),
            Err(recovery_error) => Err(anyhow!(
                "install failed: {error}; rollback failed: {recovery_error}"
            )),
        };
    }
    Ok(())
}

fn run_php_install_in_transaction(
    specs: Vec<String>,
    quiet: bool,
    locked: bool,
    cwd: &Path,
) -> Result<()> {
    let explicit_add = !specs.is_empty();
    let specs = if specs.is_empty() {
        collect_project_install_specs(cwd)?
    } else {
        specs
    };

    if specs.is_empty() {
        bail!("no packages declared in deka.json or deka.lock");
    }

    let lock_path = cwd.join(lock::LOCKFILE_NAME);
    let existing_lock = lock::read_lockfile_at(&lock_path);
    if locked && existing_lock.packages.is_empty() {
        bail!("--locked install requires an existing deka.lock");
    }
    // Do not open the transaction until the first fetched package has passed
    // validation. A rejected artifact must not create a journal or participate
    // in atomic-swap recovery (deka#619).
    let mut transaction = None;
    let start = Instant::now();
    let mut pending = VecDeque::new();
    let mut requested = BTreeMap::new();
    for spec in &specs {
        enqueue_package_spec(&mut pending, &mut requested, spec, "root")?;
    }
    let mut installed = BTreeMap::new();

    // A linked package is provided by a local working tree, so there is
    // nothing to fetch and no published version to resolve. Skipping it here
    // is what makes `deka link` usable before a version exists on the
    // registry -- which is the whole point of linking (deka#512).
    let linked: std::collections::BTreeSet<String> = read_links_at(cwd)
        .map(|manifest| manifest.packages.keys().cloned().collect())
        .unwrap_or_default();

    // A release is source plus its declared dependencies, never a recursive
    // vendor tree. Resolve each declared dependency here so every package is a
    // sibling under the consumer's ds_modules directory.
    while let Some(name) = pending.pop_front() {
        if linked.contains(&name) {
            continue;
        }
        let requirements = requested.get(&name).expect("queued package requirement");
        let locked_pkg = locked_package(&existing_lock, &name)?;
        if locked && locked_pkg.is_none() {
            bail!("--locked install requires '{}' in deka.lock", name);
        }
        let destination = php_modules_path_for_in(cwd, &name)?;
        let staging = install_staging_path(&destination)?;
        cleanup_install_staging(&staging);

        let requested_version = requirements
            .iter()
            .find(|req| req.range != "latest" && req.range != "*")
            .map(|req| req.range.as_str())
            .unwrap_or("latest");

        // @deka stdlib packages are now served from deka.gg metadata + R2 tarballs.
        // Legacy linkhash/harar registry support has been removed.
        let install_source = if is_deka_package(&name) {
            install_from_registry(&name, locked_pkg.as_ref(), requested_version, &staging)
                .with_context(|| format!("failed to install {} from deka.gg", name))?
        } else {
            bail!(
                "package {} cannot be installed: legacy linkhash/harar registry support has been removed. Use @deka/* stdlib packages or publish to GitHub.",
                name
            );
        };

        if let Some(conflict) = first_unsatisfied_requirement(requirements, &install_source.version)
        {
            bail!(
                "{} {} does not satisfy requirement {} from {}",
                name,
                install_source.version,
                conflict.range,
                conflict.requested_by
            );
        }

        if let Err(err) = reject_vendored_php_modules(&staging, &name) {
            cleanup_install_staging(&staging);
            return Err(err);
        }

        if let Err(err) = reject_source_less_package(&staging, &name) {
            cleanup_install_staging(&staging);
            return Err(err);
        }

        let package_integrity = compute_package_integrity(&staging)
            .map_err(|err| anyhow!("failed to compute package integrity for {}: {}", name, err))?;

        let mut heal_legacy_module_graph = false;
        if let Some(locked) = &locked_pkg {
            match verify_locked_integrity(&name, locked, &install_source, &package_integrity) {
                Ok(heal) => heal_legacy_module_graph = heal,
                Err(err) => {
                    cleanup_install_staging(&staging);
                    return Err(err);
                }
            }
            // The package still needs to contribute its declared dependencies
            // to the flat graph, even when its bytes are already lock-verified.
        }
        // deka#611: under --locked the legacy empty moduleGraph constant is
        // accepted as-is instead of self-healing (and instead of bailing,
        // which turned "do not mutate my lockfile" into "fail if my
        // lockfile is old" and rejected every pre-existing lock under the
        // strictest mode). The constant is the hash of an empty file list —
        // identical for every legacy package, so it carries no integrity
        // signal — while fsGraph, the hash that actually gates on package
        // bytes, is verified unconditionally above. Skip the rewrite and
        // keep the lock entry byte-for-byte. A non-empty locked
        // moduleGraph that disagrees still bails inside
        // verify_locked_integrity.
        if heal_legacy_module_graph && locked {
            heal_legacy_module_graph = false;
        }

        let dependencies = package_dependencies(&staging, &name)?;
        let dependency_names = dependencies
            .iter()
            .map(|dependency| {
                let (raw_name, _) = parse_package_spec(dependency);
                normalize_php_spec(&raw_name)
            })
            .collect::<Result<Vec<_>>>()?;
        // A locked package has already been verified against its immutable
        // release bytes. Preserve its metadata byte-for-byte so a normal
        // fresh-checkout install does not rewrite the tracked lockfile. The
        // one exception is a legacy empty moduleGraph hash (deka#611), which
        // verify_locked_integrity flagged for self-healing above.
        let metadata = if locked_pkg.is_some() {
            let mut metadata = existing_lock
                .packages
                .get(&name)
                .map(|(_, _, metadata, _)| metadata.clone())
                .expect("locked package must have a lock entry");
            if heal_legacy_module_graph {
                if let Value::Object(map) = &mut metadata {
                    map.insert(
                        "moduleGraph".to_string(),
                        json!({ "algo": "sha256", "hash": package_integrity.module_graph }),
                    );
                }
            }
            metadata
        } else {
            json!({
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
            })
        };
        let transaction = transaction.get_or_insert(InstallTransaction::begin(cwd, &lock_path)?);
        transaction.commit_package(&staging, &destination)?;
        installed.insert(
            name.clone(),
            (
                format!("{}@{}", name, install_source.version),
                format!("{}:{}", install_source.source, name),
                metadata,
                String::new(),
            ),
        );
        for dependency in dependencies {
            enqueue_package_spec(&mut pending, &mut requested, &dependency, &name)?;
        }
    }

    if locked {
        if let Some(diff) = lock_diff(&existing_lock, &installed) {
            bail!("--locked install would change deka.lock: {}", diff);
        }
    }

    transaction.get_or_insert(InstallTransaction::begin(cwd, &lock_path)?);

    // The lockfile represents exactly the resolved transitive graph. This
    // also removes stale dependencies that are no longer declared by a release.
    lock::write_lockfile_at(
        &lock_path,
        &lock::DekaLock {
            lockfile_version: existing_lock.lockfile_version,
            packages: installed.clone(),
        },
    )?;

    // RFD 27 grant-table delivery (deka#797): rewrite the grant table with the lockfile.
    grants::deliver_grant_table(cwd, &installed, locked)?;
    if explicit_add {
        record_root_dependencies(cwd, &specs, &installed)?;
    }
    pause_for_kill_test("after-lock");

    transaction
        .take()
        .expect("transaction was initialized before lock commit")
        .finish()?;

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

struct UnsatisfiedRequirement {
    range: String,
    requested_by: String,
}

fn version_satisfies(range: &str, version: &str) -> bool {
    let range = range.trim();
    if range.is_empty() || range == "latest" || range == "*" {
        return true;
    }
    let range = range.trim_start_matches('v');
    let version = version.trim_start_matches('v');
    range == version
}

fn first_unsatisfied_requirement(
    requirements: &[VersionRequirement],
    version: &str,
) -> Option<UnsatisfiedRequirement> {
    for req in requirements {
        if !version_satisfies(&req.range, version) {
            return Some(UnsatisfiedRequirement {
                range: req.range.clone(),
                requested_by: req.requested_by.clone(),
            });
        }
    }
    None
}

fn lock_diff(
    existing: &lock::DekaLock,
    installed: &BTreeMap<String, (String, String, Value, String)>,
) -> Option<String> {
    for (name, (descriptor, _, _, _)) in installed {
        match existing.packages.get(name) {
            Some((existing_descriptor, _, _, _)) if existing_descriptor == descriptor => {}
            Some((existing_descriptor, _, _, _)) => {
                return Some(format!(
                    "{} resolved to {} but lock has {}",
                    name, descriptor, existing_descriptor
                ));
            }
            None => return Some(format!("{} is not in the existing lock", name)),
        }
    }
    for name in existing.packages.keys() {
        if !installed.contains_key(name) {
            return Some(format!("{} is present in lock but was not installed", name));
        }
    }
    None
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
    let canonical_root = package_root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", package_root.display()))?;
    let mut directories = vec![canonical_root.clone()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let entry_name = entry.file_name();
            if is_modules_dir_name(&entry_name.to_string_lossy()) {
                bail!(
                    "package {} contains vendored {} at {}; packages must declare dependencies in deka.json",
                    package_name,
                    entry_name.to_string_lossy(),
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
                let canonical_entry = entry.path().canonicalize().with_context(|| {
                    format!("failed to canonicalize {}", entry.path().display())
                })?;
                if !canonical_entry.starts_with(&canonical_root) {
                    bail!(
                        "package {} contains directory outside package root at {}",
                        package_name,
                        entry.path().display()
                    );
                }
                directories.push(canonical_entry);
            }
        }
    }
    Ok(())
}

fn reject_source_less_package(package_root: &Path, package_name: &str) -> Result<()> {
    if package_has_dekascript_sources(package_root)? {
        return Ok(());
    }
    bail!(
        "package {} cannot be installed: no .ds/.dsx sources (the package contributes no resolvable modules)",
        package_name
    );
}

fn package_has_dekascript_sources(root: &Path) -> Result<bool> {
    fn visit(path: &Path) -> std::io::Result<bool> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                if visit(&path)? {
                    return Ok(true);
                }
            } else if file_type.is_file()
                && matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("ds") | Some("dsx")
                )
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    visit(root).with_context(|| format!("failed to inspect package {}", root.display()))
}

#[derive(Debug, Clone)]
struct InstalledSource {
    version: String,
    repo: Option<String>,
    git_ref: Option<String>,
    source: &'static str,
    requires_registry_digest: bool,
}

#[derive(Debug, Clone)]
struct LockedPackage {
    version: String,
    resolved: String,
    module_graph: String,
    fs_graph: String,
}

const GITHUB_STDLIB_ORG: &str = "dekaruntime";

#[derive(Debug, Deserialize)]
struct RegistryPackage {
    #[serde(default)]
    versions: Vec<String>,
}

fn latest_registry_version(versions: &[String]) -> Result<String> {
    let mut best: Option<(semver::Version, String)> = None;
    for raw in versions {
        let trimmed = raw.trim().trim_start_matches('v');
        if trimmed.is_empty() {
            continue;
        }
        let parsed = semver::Version::parse(trimmed)
            .with_context(|| format!("registry version `{raw}` is not semver"))?;
        match &best {
            Some((current, _)) if parsed <= *current => {}
            _ => best = Some((parsed, trimmed.to_string())),
        }
    }
    best.map(|(_, version)| version)
        .ok_or_else(|| anyhow!("registry listed no usable versions"))
}

fn select_registry_version(registry: &RegistryPackage, requested: &str) -> Result<String> {
    let requested = requested.trim();
    if requested != "latest" && requested != "*" && !requested.is_empty() {
        return Ok(requested.trim_start_matches('v').to_string());
    }
    latest_registry_version(&registry.versions)
}

/// Install a @deka stdlib package from the deka.gg registry + R2 tarball CDN.
/// Endpoint URLs (incl. test-only env overrides) live in `crate::registry`.
///
/// Registry metadata is fetched over HTTPS from deka.gg, and the release bytes
/// come from a public Cloudflare R2 bucket. This removes the git/Linkhash
/// dependency for stdlib installs.
fn install_from_registry(
    name: &str,
    locked: Option<&LockedPackage>,
    requested: &str,
    destination: &Path,
) -> Result<InstalledSource> {
    let package_name = name.strip_prefix("@deka/").ok_or_else(|| {
        anyhow!(
            "install_from_registry called with non-@deka package: {}",
            name
        )
    })?;

    let registry_url = format!(
        "{}/api/registry/{}.json",
        registry::base_url(),
        package_name
    );
    let registry_resp = reqwest::blocking::get(&registry_url)
        .with_context(|| format!("failed to contact deka.gg registry for {}", name))?;
    if registry_resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!(
            "package {} not found in deka.gg registry (status {})",
            name,
            registry_resp.status()
        );
    }
    if !registry_resp.status().is_success() {
        bail!(
            "registry lookup failed for {}: status {}",
            name,
            registry_resp.status()
        );
    }
    let registry: RegistryPackage = registry_resp
        .json()
        .with_context(|| format!("failed to parse deka.gg registry metadata for {}", name))?;

    let version = match locked {
        Some(locked) => locked.version.clone(),
        None => select_registry_version(&registry, requested)
            .with_context(|| format!("failed to select a version for {}", name))?,
    };

    let repo_url = format!(
        "https://github.com/{}/{}.git",
        GITHUB_STDLIB_ORG, package_name
    );
    let tag = format!("v{}", version);
    let tarball_url = format!(
        "{}/{}/{}/{}-{}.tgz",
        registry::stdlib_cdn_url(),
        package_name,
        version,
        package_name,
        version
    );

    let temp_dir = tempfile::tempdir()
        .with_context(|| format!("failed to create temp dir for {}@{}", name, version))?;
    let tarball_path = temp_dir.path().join("package.tgz");

    let mut resp = reqwest::blocking::get(&tarball_url)
        .with_context(|| format!("failed to download tarball for {}@{}", name, version))?;
    if !resp.status().is_success() {
        bail!(
            "tarball download failed for {}@{}: status {}",
            name,
            version,
            resp.status()
        );
    }
    {
        let mut file = fs::File::create(&tarball_path)
            .with_context(|| format!("failed to create tarball file for {}", name))?;
        resp.copy_to(&mut file)
            .with_context(|| format!("failed to write tarball for {}", name))?;
    }

    let extract_dir = temp_dir.path().join("extract");
    fs::create_dir_all(&extract_dir)
        .with_context(|| format!("failed to create extract dir for {}", name))?;
    let status = std::process::Command::new("tar")
        .args([
            "-xzf",
            tarball_path.to_str().expect("tarball path is valid utf-8"),
            "-C",
            extract_dir.to_str().expect("extract path is valid utf-8"),
        ])
        .status()
        .with_context(|| format!("failed to run tar for {}@{}", name, version))?;
    if !status.success() {
        bail!("tar extraction failed for {}@{}", name, version);
    }

    copy_github_package_files(&extract_dir, destination)
        .with_context(|| format!("failed to copy {}@{} to staging", name, version))?;

    Ok(InstalledSource {
        version,
        repo: Some(repo_url),
        git_ref: Some(tag),
        source: "deka.gg",
        requires_registry_digest: false,
    })
}

fn copy_github_package_files(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)
        .with_context(|| format!("failed to create target dir {}", target.display()))?;

    for entry in fs::read_dir(source)
        .with_context(|| format!("failed to read source dir {}", source.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == ".git" {
            continue;
        }

        // macOS AppleDouble entries (._*) ride along in tarballs built on a
        // Mac; the reader skips them (deka#587) and they must never reach the
        // install tree.
        if name_str.starts_with("._") {
            continue;
        }

        let src_path = entry.path();
        let dst_path = target.join(&name);
        let file_type = entry.file_type()?;

        if is_modules_dir_name(&name_str) {
            bail!(
                "package artifact contains vendored {name_str} at {}; packages must declare dependencies in deka.json",
                src_path.display()
            );
        }

        if file_type.is_symlink() {
            bail!(
                "package artifact contains symlink {}; package artifacts may not contain symlinks",
                src_path.display()
            );
        }

        if file_type.is_dir() {
            copy_github_package_files(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }

    Ok(())
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstallJournal {
    lock_path: PathBuf,
    lock_backup: Option<PathBuf>,
    /// deka#797: grant table snapshot, restored on recovery (see `crate::grants`).
    #[serde(default)]
    grant: grants::GrantTableSnapshot,
    packages: Vec<InstallJournalPackage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstallJournalPackage {
    staging: PathBuf,
    destination: PathBuf,
    had_destination: bool,
    swapped: bool,
}

pub(crate) struct InstallTransaction {
    journal_path: PathBuf,
    journal: InstallJournal,
}

impl InstallTransaction {
    pub(crate) fn begin(project_dir: &Path, lock_path: &Path) -> Result<Self> {
        let journal_path = project_dir.join(".deka-install-transaction.json");
        let (lock_path, lock_backup) = lock::snapshot_file(lock_path.to_path_buf())?;
        let grant = grants::GrantTableSnapshot::snapshot(project_dir)?;
        let transaction = Self {
            journal_path,
            journal: InstallJournal {
                lock_path,
                lock_backup,
                grant,
                packages: Vec::new(),
            },
        };
        transaction.persist()?;
        Ok(transaction)
    }

    fn persist(&self) -> Result<()> {
        let parent = self
            .journal_path
            .parent()
            .ok_or_else(|| anyhow!("transaction journal has no parent"))?;
        let temp = parent.join(format!(
            ".deka-install-journal-tmp-{}",
            lock::unique_suffix()
        ));
        let bytes = serde_json::to_vec_pretty(&self.journal)?;
        fs::write(&temp, bytes)?;
        let file = fs::OpenOptions::new().read(true).open(&temp)?;
        file.sync_all()?;
        fs::rename(temp, &self.journal_path)?;
        lock::sync_directory(parent)?;
        Ok(())
    }

    fn commit_package(&mut self, staging: &Path, destination: &Path) -> Result<()> {
        mark_staging_tree(staging)?;
        self.journal.packages.push(InstallJournalPackage {
            staging: staging.to_path_buf(),
            destination: destination.to_path_buf(),
            had_destination: destination.exists(),
            swapped: false,
        });
        self.persist()?;
        pause_for_kill_test("before-swap");
        replace_installed_package(staging, destination)?;
        self.journal
            .packages
            .last_mut()
            .expect("journal entry was just added")
            .swapped = true;
        pause_for_kill_test("after-swap");
        self.persist()
    }

    fn finish(self) -> Result<()> {
        // Clearing the journal is the commit point. Cleanup after this point
        // is best-effort and cannot make the live package/lock inconsistent.
        fs::remove_file(&self.journal_path)?;
        lock::sync_directory(
            self.journal_path
                .parent()
                .ok_or_else(|| anyhow!("transaction journal has no parent"))?,
        )?;
        if let Some(backup) = self.journal.lock_backup {
            let _ = fs::remove_file(backup);
        }
        self.journal.grant.discard();
        for package in self.journal.packages {
            let _ = fs::remove_file(package.destination.join(STAGED_MARKER));
            if package.had_destination {
                cleanup_install_staging(&package.staging);
            } else if let Some(root) = package.staging.parent() {
                let _ = fs::remove_dir_all(root);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn commit_package_with_pause(
        &mut self,
        staging: &Path,
        destination: &Path,
        point: &str,
    ) -> Result<()> {
        mark_staging_tree(staging)?;
        self.journal.packages.push(InstallJournalPackage {
            staging: staging.to_path_buf(),
            destination: destination.to_path_buf(),
            had_destination: destination.exists(),
            swapped: false,
        });
        self.persist()?;
        pause_for_kill_test(point);
        replace_installed_package(staging, destination)?;
        self.journal
            .packages
            .last_mut()
            .expect("journal entry was just added")
            .swapped = true;
        pause_for_kill_test("after-swap");
        self.persist()?;
        pause_for_kill_test("after-journal");
        Ok(())
    }
}

fn pause_for_kill_test(point: &str) {
    if std::env::var("DEKA_PM_KILL_POINT").ok().as_deref() == Some(point) {
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}

fn mark_staging_tree(staging: &Path) -> Result<()> {
    fs::write(staging.join(STAGED_MARKER), b"staged\n")?;
    lock::sync_directory(
        staging
            .parent()
            .ok_or_else(|| anyhow!("staging path has no parent"))?,
    )
}

pub(crate) fn recover_install_transaction(project_dir: &Path) -> Result<()> {
    let journal_path = project_dir.join(".deka-install-transaction.json");
    if !journal_path.exists() {
        return Ok(());
    }
    let journal: InstallJournal = serde_json::from_slice(&fs::read(&journal_path)?)
        .context("failed to parse install transaction journal")?;
    for package in journal.packages.iter().rev() {
        let exchange_completed = package.swapped
            || package.destination.join(STAGED_MARKER).is_file()
            || (!package.had_destination && package.destination.exists());
        if exchange_completed && package.had_destination {
            if package.staging.exists() && package.destination.exists() {
                swap_directories(&package.staging, &package.destination)?;
            } else if package.staging.exists() {
                fs::rename(&package.staging, &package.destination)?;
            }
        } else if exchange_completed && package.destination.exists() {
            fs::remove_dir_all(&package.destination)?;
        }
        if let Some(parent) = package.destination.parent() {
            lock::sync_directory(parent)?;
        }
    }
    if let Some(backup) = journal.lock_backup {
        if backup.exists() {
            if journal.lock_path.exists() {
                fs::remove_file(&journal.lock_path)?;
            }
            fs::rename(backup, &journal.lock_path)?;
            if let Some(parent) = journal.lock_path.parent() {
                lock::sync_directory(parent)?;
            }
        }
    } else if journal.lock_path.exists() {
        fs::remove_file(&journal.lock_path)?;
    }
    journal.grant.restore()?; // deka#797: mirrors the lockfile recovery above.
    fs::remove_file(journal_path)?;
    lock::sync_directory(project_dir)?;
    Ok(())
}

fn replace_installed_package(staging: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if destination.exists() {
        swap_directories(staging, destination)?;
    } else {
        fs::rename(staging, destination)
            .with_context(|| format!("failed to install {}", destination.display()))?;
    }
    if let Some(parent) = destination.parent() {
        lock::sync_directory(parent)?;
    }
    Ok(())
}

fn swap_directories(first: &Path, second: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        let first = CString::new(first.as_os_str().as_encoded_bytes())?;
        let second = CString::new(second.as_os_str().as_encoded_bytes())?;
        let result = unsafe {
            libc::renameatx_np(
                libc::AT_FDCWD,
                first.as_ptr(),
                libc::AT_FDCWD,
                second.as_ptr(),
                libc::RENAME_SWAP,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "linux")]
        {
            use std::ffi::CString;
            use std::os::unix::ffi::OsStrExt;
            let first = CString::new(first.as_os_str().as_bytes())?;
            let second = CString::new(second.as_os_str().as_bytes())?;
            let result = unsafe {
                libc::syscall(
                    libc::SYS_renameat2,
                    libc::AT_FDCWD,
                    first.as_ptr(),
                    libc::AT_FDCWD,
                    second.as_ptr(),
                    libc::RENAME_EXCHANGE,
                )
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            return Ok(());
        }

        #[cfg(not(target_os = "linux"))]
        compile_error!("directory replacement must use an atomic platform primitive");
    }
}

fn cleanup_install_staging(staging: &Path) {
    if let Some(root) = staging.parent() {
        let _ = fs::remove_dir_all(root);
    }
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

/// Verifies a locked package against the freshly computed integrity.
///
/// Returns `Ok(true)` when the lock entry carried the legacy empty
/// moduleGraph hash (deka#611) and was accepted for self-healing — the
/// caller must then rewrite the entry's `moduleGraph` metadata with the
/// recomputed hash, except under `--locked`, where the caller skips the
/// rewrite and keeps the lock entry byte-for-byte. Returns `Ok(false)`
/// when everything matched as-is.
fn verify_locked_integrity(
    name: &str,
    locked: &LockedPackage,
    installed: &InstalledSource,
    integrity: &deka_host::integrity::PackageIntegrity,
) -> Result<bool> {
    if installed.version != locked.version {
        bail!(
            "integrity verification failed for {}: locked version {} but installed {}",
            name,
            locked.version,
            installed.version
        );
    }
    let allowed_sources = ["linkhash", "github", "deka.gg"];
    let Some((source, _)) = locked.resolved.split_once(':') else {
        bail!(
            "integrity verification failed for {}: malformed lock source {}",
            name,
            locked.resolved
        );
    };
    if !allowed_sources.contains(&source) {
        bail!(
            "integrity verification failed for {}: unsupported lock source {}",
            name,
            locked.resolved
        );
    }
    // deka#611: lock entries written while the module-graph walk only saw
    // the deleted `.phpx` extension all carry the SHA-256 of the empty
    // input. When the locked hash is exactly that constant and the package
    // has real sources (so the new walk produces a different hash), the
    // locked value carries no integrity signal — report the entry for
    // self-healing instead of failing the install.
    let heal_legacy_module_graph = locked.module_graph
        == deka_host::integrity::EMPTY_MODULE_GRAPH_HASH
        && integrity.module_graph != locked.module_graph;
    if !heal_legacy_module_graph && integrity.module_graph != locked.module_graph {
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
    Ok(heal_legacy_module_graph)
}

async fn rehash_php_packages(payload: &InstallPayload) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    rehash_php_packages_in(payload, &cwd).await
}

pub(crate) async fn rehash_php_packages_in(
    payload: &InstallPayload,
    project_dir: &Path,
) -> Result<()> {
    let lock_path = project_dir.join(lock::LOCKFILE_NAME);
    let lock = lock::read_lockfile_at(&lock_path);
    let mut specs = payload.specs.clone();
    if specs.is_empty() {
        specs = lock.packages.keys().cloned().collect();
    }
    if specs.is_empty() {
        bail!("no packages found to rehash");
    }

    for name in specs {
        let package_root = php_modules_path_for_in(project_dir, &name)?;
        if !package_root.is_dir() {
            bail!(
                "package '{}' is missing from {MODULES_DIR} (expected {})",
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

    // deka#797: re-key the grant table with the recomputed digests; the lock
    // writes above went through to disk, so re-read it before deriving.
    let refreshed = lock::read_lockfile_at(&lock_path);
    grants::rewrite_grant_table(project_dir, &refreshed.packages)?;

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

#[cfg(test)]
fn php_modules_path_for(package_name: &str) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    php_modules_path_for_in(&cwd, package_name)
}

fn php_modules_path_for_in(project_dir: &Path, package_name: &str) -> Result<PathBuf> {
    let mut path = install_modules_dir(project_dir);
    // Scoped packages (@scope/name) map to ds_modules/@scope/name on disk.
    // Unscoped names are still supported as-is.
    let segments: Vec<&str> = package_name.split('/').collect();
    for segment in segments {
        if segment.is_empty() || segment == "." || segment == ".." {
            bail!("invalid package name segment");
        }
        path = path.join(segment);
    }
    Ok(path)
}

fn record_root_dependencies(
    project_dir: &Path,
    root_specs: &[String],
    installed: &BTreeMap<String, lock::LockEntry>,
) -> Result<()> {
    let path = project_dir.join("deka.json");
    if !path.exists() {
        return Ok(());
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut manifest: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let Some(obj) = manifest.as_object_mut() else {
        bail!("{} must be a JSON object", path.display());
    };
    let deps_value = obj.entry("dependencies").or_insert_with(|| json!({}));
    let Some(deps) = deps_value.as_object_mut() else {
        bail!("{} dependencies must be an object", path.display());
    };

    for spec in root_specs {
        let (raw_name, _) = parse_package_spec(spec.trim());
        let canonical = normalize_php_spec(&raw_name)?;
        let Some((descriptor, _, _, _)) = installed.get(&canonical) else {
            continue;
        };
        let Some(version) = locked_version(&canonical, descriptor) else {
            continue;
        };
        let key = manifest_dep_key(deps, &raw_name, &canonical);
        deps.insert(key, json!(version));
    }

    let serialized = serde_json::to_string_pretty(&manifest)? + "\n";
    fs::write(&path, serialized).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn manifest_dep_key(
    deps: &serde_json::Map<String, Value>,
    raw_name: &str,
    canonical: &str,
) -> String {
    if deps.contains_key(raw_name) {
        return raw_name.to_string();
    }
    if deps.contains_key(canonical) {
        return canonical.to_string();
    }
    if raw_name.starts_with('@') {
        canonical.to_string()
    } else {
        raw_name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InstallTransaction, InstalledSource, LockedPackage, MODULES_DIR, RegistryPackage,
        collect_project_install_specs, copy_github_package_files, enqueue_package_spec,
        locked_package, package_dependencies, pause_for_kill_test, php_modules_path_for,
        record_root_dependencies, recover_install_transaction, rehash_php_packages_in,
        reject_source_less_package, reject_vendored_php_modules, run_php_install_in,
        select_registry_version, verify_locked_integrity,
    };
    use crate::{lock, payload::InstallPayload};
    use deka_host::integrity::{PackageIntegrity, compute_package_integrity};
    use serde_json::json;
    use std::{collections::BTreeMap, fs};

    #[test]
    fn copy_github_package_files_skips_appledouble_entries() {
        let src = tempfile::tempdir().expect("src");
        let dst = tempfile::tempdir().expect("dst");
        fs::write(src.path().join("mod.ds"), "fn ok() number { return 1 }").expect("real file");
        fs::write(src.path().join("._mod.ds"), "junk appledouble").expect("appledouble");
        fs::create_dir(src.path().join("nested")).expect("nested dir");
        fs::write(src.path().join("nested").join("._other"), "junk").expect("nested junk");
        fs::write(
            src.path().join("nested").join("real.ds"),
            "fn r() number { return 2 }",
        )
        .expect("nested real");

        copy_github_package_files(src.path(), dst.path()).expect("copy");

        assert!(dst.path().join("mod.ds").exists());
        assert!(dst.path().join("nested").join("real.ds").exists());
        assert!(!dst.path().join("._mod.ds").exists());
        assert!(!dst.path().join("nested").join("._other").exists());
    }

    #[test]
    fn unlocked_add_selects_latest_registry_version() {
        let registry = RegistryPackage {
            versions: vec!["0.1.0".into(), "0.1.1".into(), "0.2.0".into()],
        };
        assert_eq!(
            select_registry_version(&registry, "latest").expect("latest"),
            "0.2.0"
        );
        assert_eq!(
            select_registry_version(&registry, "0.1.1").expect("exact"),
            "0.1.1"
        );
        assert_ne!(
            select_registry_version(
                &RegistryPackage {
                    versions: vec!["0.1.1".into()],
                },
                "latest"
            )
            .expect("catalog latest"),
            "0.1.0"
        );
    }

    #[test]
    fn add_records_unscoped_dependency_in_deka_json() {
        let tmp = tempfile::tempdir().expect("tmp");
        fs::write(
            tmp.path().join("deka.json"),
            "{\n  \"name\": \"probe\",\n  \"security\": { \"prompt\": false }\n}\n",
        )
        .expect("manifest");
        let mut installed = BTreeMap::new();
        installed.insert(
            "@deka/bytes".to_string(),
            (
                "@deka/bytes@0.2.0".to_string(),
                "deka.gg:@deka/bytes".to_string(),
                json!({}),
                String::new(),
            ),
        );
        record_root_dependencies(tmp.path(), &["bytes".to_string()], &installed).expect("record");
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(tmp.path().join("deka.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["dependencies"]["bytes"], json!("0.2.0"));
        assert!(manifest["dependencies"].get("@deka/bytes").is_none());
    }

    #[test]
    fn scoped_package_installs_to_scoped_php_modules_path() {
        let cwd = std::env::current_dir().expect("cwd");
        let path = php_modules_path_for("@deka/component").expect("path");
        assert_eq!(path, cwd.join(MODULES_DIR).join("@deka").join("component"));
    }

    #[test]
    fn non_deka_scoped_package_preserves_scope_in_path() {
        let cwd = std::env::current_dir().expect("cwd");
        let path = php_modules_path_for("@tana/store").expect("path");
        assert_eq!(path, cwd.join(MODULES_DIR).join("@tana").join("store"));
    }

    #[test]
    fn unscoped_package_preserves_legacy_path() {
        let cwd = std::env::current_dir().expect("cwd");
        let path = php_modules_path_for("legacy").expect("path");
        assert_eq!(path, cwd.join(MODULES_DIR).join("legacy"));
    }

    #[tokio::test]
    async fn rehash_resolves_locked_deka_package_to_stdlib_root() {
        let tmp = tempfile::tempdir().expect("tmp");
        let package_root = tmp.path().join(MODULES_DIR).join("@deka").join("component");
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
            locked: false,
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
    fn deka_packages_always_install_to_canonical_scoped_paths() {
        let cwd = std::env::current_dir().expect("cwd");
        assert_eq!(
            php_modules_path_for("@deka/array").expect("path"),
            cwd.join(MODULES_DIR).join("@deka").join("array")
        );
        assert_eq!(
            php_modules_path_for("@deka/crypto").expect("path"),
            cwd.join(MODULES_DIR).join("@deka").join("crypto")
        );
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
            requires_registry_digest: false,
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
    fn locked_integrity_flags_legacy_empty_module_graph_hash_for_healing() {
        let locked = LockedPackage {
            version: "0.1.0".to_string(),
            resolved: "linkhash:@deka/core".to_string(),
            module_graph: deka_host::integrity::EMPTY_MODULE_GRAPH_HASH.to_string(),
            fs_graph: "locked-fs".to_string(),
        };
        let installed = InstalledSource {
            version: "0.1.0".to_string(),
            repo: None,
            git_ref: None,
            source: "linkhash",
            requires_registry_digest: false,
        };
        let integrity = PackageIntegrity {
            module_graph: "recomputed-module".to_string(),
            fs_graph: "locked-fs".to_string(),
        };

        let heal =
            verify_locked_integrity("@deka/core", &locked, &installed, &integrity).expect("verify");
        assert!(
            heal,
            "legacy empty moduleGraph hash must be flagged for self-healing, not rejected"
        );

        // A package with no source files recomputes the same empty-input
        // constant; the entry then matches as-is and must not be healed.
        let empty_integrity = PackageIntegrity {
            module_graph: deka_host::integrity::EMPTY_MODULE_GRAPH_HASH.to_string(),
            fs_graph: "locked-fs".to_string(),
        };
        let heal = verify_locked_integrity("@deka/core", &locked, &installed, &empty_integrity)
            .expect("verify");
        assert!(
            !heal,
            "a genuinely empty package must verify without healing"
        );
    }

    // The published @deka/string artifact is intentionally source-less now;
    // its old integration path cannot exercise lock healing after the
    // source-less package rejection. The pure integrity behavior is covered
    // by locked_integrity_flags_legacy_empty_module_graph_hash_for_healing.
    #[ignore = "published fixture is source-less and is rejected before lock verification"]
    #[test]
    fn locked_install_accepts_legacy_empty_module_graph_hash_without_mutating_lock() {
        let tmp = tempfile::tempdir().expect("project");
        run_php_install_in(vec!["@deka/string".to_string()], true, false, tmp.path())
            .expect("bundled deka install");
        let lock_path = tmp.path().join("deka.lock");
        assert!(lock_path.is_file());

        // Roll the lock entry back to the legacy empty-input moduleGraph
        // hash every pre-#611 lockfile carries (deka#611), leaving fsGraph
        // — the hash that gates on package bytes — untouched. This is the
        // exact lockfile shape the 824 conformance fixtures install with.
        let mut lock = lock::read_lockfile_at(&lock_path);
        let (_, _, metadata, _) = lock.packages.get_mut("@deka/string").expect("lock entry");
        if let serde_json::Value::Object(map) = metadata {
            map.insert(
                "moduleGraph".to_string(),
                json!({ "algo": "sha256", "hash": deka_host::integrity::EMPTY_MODULE_GRAPH_HASH }),
            );
        }
        lock::write_lockfile_at(&lock_path, &lock).expect("write legacy lock");
        let lock_before = fs::read_to_string(&lock_path).expect("read legacy lock");

        run_php_install_in(vec!["@deka/string".to_string()], true, true, tmp.path())
            .expect("--locked install must accept the legacy moduleGraph hash");

        assert_eq!(
            fs::read_to_string(&lock_path).expect("read lock after --locked install"),
            lock_before,
            "--locked install must not mutate deka.lock"
        );
        let (_, _, metadata, _) = lock::read_lockfile_at(&lock_path)
            .packages
            .get("@deka/string")
            .expect("lock entry")
            .clone();
        assert_eq!(
            metadata["moduleGraph"]["hash"].as_str(),
            Some(deka_host::integrity::EMPTY_MODULE_GRAPH_HASH),
            "--locked install must not self-heal the legacy entry"
        );

        // A non-empty locked moduleGraph that disagrees with the computed
        // hash is a genuine mismatch and must stay fatal under --locked.
        let mut lock = lock::read_lockfile_at(&lock_path);
        let (_, _, metadata, _) = lock.packages.get_mut("@deka/string").expect("lock entry");
        if let serde_json::Value::Object(map) = metadata {
            map.insert(
                "moduleGraph".to_string(),
                json!({ "algo": "sha256", "hash": "0".repeat(64) }),
            );
        }
        lock::write_lockfile_at(&lock_path, &lock).expect("write mismatched lock");
        let err = run_php_install_in(vec!["@deka/string".to_string()], true, true, tmp.path())
            .expect_err("non-empty moduleGraph mismatch must fail under --locked");
        assert!(
            err.to_string().contains("moduleGraph hash mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn staged_install_preserves_existing_package_until_verified_replace() {
        let tmp = tempfile::tempdir().expect("tmp");
        let destination = tmp.path().join(MODULES_DIR).join("core");
        fs::create_dir_all(&destination).expect("mkdir destination");
        fs::write(
            destination.join("index.phpx"),
            "export function ok() { return true; }\n",
        )
        .expect("write original");
        let staging = tmp
            .path()
            .join(MODULES_DIR)
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

        let mut transaction =
            InstallTransaction::begin(tmp.path(), &tmp.path().join("deka.lock")).expect("begin");
        transaction
            .commit_package(&staging, &destination)
            .expect("replace");
        transaction.finish().expect("finish");

        assert_eq!(
            fs::read_to_string(destination.join("index.phpx")).expect("read replaced"),
            "export function ok() { return 'verified'; }\n"
        );
        assert!(
            !tmp.path()
                .join(MODULES_DIR)
                .join(".deka-install-test")
                .exists()
        );
    }

    #[test]
    fn interrupted_transaction_restores_exact_package_and_lock_snapshot() {
        let tmp = tempfile::tempdir().expect("tmp");
        let destination = tmp.path().join(MODULES_DIR).join("core");
        fs::create_dir_all(&destination).expect("mkdir destination");
        fs::write(destination.join("index.phpx"), "old\n").expect("old package");
        let lock_path = tmp.path().join("deka.lock");
        let old_lock = "{\"lockfileVersion\":1,\"packages\":{\"core\":[\"core@1.0.0\",\"linkhash:core\",{},\"\"]}}\n";
        fs::write(&lock_path, old_lock).expect("old lock");

        let staging = tmp.path().join(MODULES_DIR).join(".stage").join("core");
        fs::create_dir_all(&staging).expect("mkdir staging");
        fs::write(staging.join("index.phpx"), "new\n").expect("new package");

        let mut transaction = InstallTransaction::begin(tmp.path(), &lock_path).expect("begin");
        transaction
            .commit_package(&staging, &destination)
            .expect("commit package");
        assert_eq!(
            fs::read_to_string(destination.join("index.phpx")).expect("new bytes"),
            "new\n"
        );
        drop(transaction);

        recover_install_transaction(tmp.path()).expect("recover");
        assert_eq!(
            fs::read_to_string(destination.join("index.phpx")).expect("restored bytes"),
            "old\n"
        );
        assert_eq!(
            fs::read_to_string(lock_path).expect("restored lock"),
            old_lock
        );
        assert!(!tmp.path().join(".deka-install-transaction.json").exists());
    }

    #[test]
    fn killed_transaction_recovers_at_every_commit_boundary() {
        if let Ok(project) = std::env::var("DEKA_PM_KILL_PROJECT") {
            let project = std::path::PathBuf::from(project);
            let destination = project.join(format!("{MODULES_DIR}/core"));
            let staging = project.join(format!("{MODULES_DIR}/.stage/core"));
            let mut transaction =
                InstallTransaction::begin(&project, &project.join("deka.lock")).expect("begin");
            if let Ok(point) = std::env::var("DEKA_PM_KILL_POINT") {
                transaction
                    .commit_package_with_pause(&staging, &destination, &point)
                    .expect("commit");
            } else {
                transaction
                    .commit_package(&staging, &destination)
                    .expect("commit");
                lock::write_lockfile_at(&project.join("deka.lock"), &lock::DekaLock::default())
                    .expect("lock replacement");
                pause_for_kill_test("after-lock");
            }
            return;
        }

        for point in ["before-swap", "after-swap", "after-journal", "after-lock"] {
            let tmp = tempfile::tempdir().expect("tmp");
            let destination = tmp.path().join(format!("{MODULES_DIR}/core"));
            fs::create_dir_all(&destination).expect("mkdir destination");
            fs::write(destination.join("index.phpx"), "old\n").expect("old package");
            let lock_path = tmp.path().join("deka.lock");
            let old_lock = "{\"lockfileVersion\":1,\"packages\":{\"core\":[\"core@1.0.0\",\"linkhash:core\",{},\"\"]}}\n";
            fs::write(&lock_path, old_lock).expect("old lock");
            let staging = tmp.path().join(format!("{MODULES_DIR}/.stage/core"));
            fs::create_dir_all(&staging).expect("mkdir staging");
            fs::write(staging.join("index.phpx"), "new\n").expect("new package");

            let mut command =
                std::process::Command::new(std::env::current_exe().expect("test exe"));
            command
                .args([
                    "--exact",
                    "install::tests::killed_transaction_recovers_at_every_commit_boundary",
                    "--nocapture",
                ])
                .env("DEKA_PM_KILL_PROJECT", tmp.path())
                .env("DEKA_PM_KILL_POINT", point);
            let mut child = command.spawn().expect("spawn kill test");
            std::thread::sleep(std::time::Duration::from_millis(250));
            child.kill().expect("kill test process");
            let _ = child.wait();

            recover_install_transaction(tmp.path()).expect("recover killed transaction");
            assert_eq!(
                fs::read_to_string(destination.join("index.phpx")).expect("package"),
                "old\n",
                "point {point}"
            );
            assert_eq!(
                fs::read_to_string(lock_path).expect("lock"),
                old_lock,
                "point {point}"
            );
        }
    }

    #[test]
    fn real_cli_invocation_rejects_source_less_before_transaction() {
        let cli = std::env::var_os("DEKA_PM_CLI_BIN")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                option_env!("CARGO_TARGET_DIR")
                    .map(std::path::PathBuf::from)
                    .map(|target| target.join("release/cli"))
            })
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/release/cli")
            });
        assert!(
            cli.is_file(),
            "build the release CLI before this test: {}",
            cli.display()
        );

        let tmp = tempfile::tempdir().expect("tmp");
        fs::write(
            tmp.path().join("deka.json"),
            json!({"dependencies": {"@deka/core": "0.1.0"}}).to_string(),
        )
        .expect("manifest");

        let status = std::process::Command::new(&cli)
            .args(["install", "--quiet"])
            .current_dir(tmp.path())
            .env("LINKHASH_REGISTRY_URL", "http://127.0.0.1:1")
            .status()
            .expect("run real deka install");
        assert!(!status.success(), "source-less package must be rejected");
        assert!(!tmp.path().join(".deka-install-transaction.json").exists());
        assert!(!tmp.path().join(MODULES_DIR).join("@deka/core").exists());
        assert!(!tmp.path().join("deka.lock").exists());
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
                .join(MODULES_DIR)
                .join("@tana")
                .join("b")
        );
        assert_eq!(
            php_modules_path_for("@deka/crypto").expect("path"),
            std::env::current_dir()
                .expect("cwd")
                .join(MODULES_DIR)
                .join("@deka")
                .join("crypto")
        );
    }

    #[test]
    fn install_rejects_vendored_php_modules_in_package_tree() {
        let tmp = tempfile::tempdir().expect("tmp");
        let nested = tmp
            .path()
            .join("src")
            .join(MODULES_DIR)
            .join("@tana")
            .join("b");
        fs::create_dir_all(&nested).expect("mkdir nested vendor tree");
        let err = reject_vendored_php_modules(tmp.path(), "@tana/a").expect_err("must reject");
        assert!(err.to_string().contains("contains vendored ds_modules"));
    }

    #[test]
    fn install_rejects_source_less_package() {
        let tmp = tempfile::tempdir().expect("tmp");
        fs::write(
            tmp.path().join("bridge.phpx"),
            "export const legacy = true;",
        )
        .expect("phpx");
        let err = reject_source_less_package(tmp.path(), "@deka/core")
            .expect_err("install must reject a package with no resolvable sources");
        assert!(err.to_string().contains("@deka/core"));
        assert!(err.to_string().contains("no .ds/.dsx sources"));
    }

    #[cfg(unix)]
    #[test]
    fn install_rejects_case_variant_and_symlinked_vendor_trees() {
        let tmp = tempfile::tempdir().expect("tmp");
        fs::create_dir_all(tmp.path().join("Php_Modules")).expect("case vendor");
        assert!(reject_vendored_php_modules(tmp.path(), "@tana/a").is_err());
        fs::remove_dir_all(tmp.path().join("Php_Modules")).expect("remove case vendor");
        std::os::unix::fs::symlink("outside", tmp.path().join(MODULES_DIR))
            .expect("vendor symlink");
        assert!(reject_vendored_php_modules(tmp.path(), "@tana/a").is_err());
    }

    #[tokio::test]
    #[ignore = "legacy linkhash/harar registry support removed; revisit during package-manager cleanup (see dekaruntime/deka#330)"]
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
            let _registry = registry.clone();
            move || run_php_install_in(vec!["@scope/a@^1.0.0".to_string()], true, false, &root)
        })
        .await
        .expect("installer task")
        .expect("install graph");
        let modules = tmp.path().join(MODULES_DIR).join("@scope");
        for package in ["a", "b", "c"] {
            assert!(modules.join(package).is_dir(), "missing flat {package}");
            assert!(!modules.join(package).join(MODULES_DIR).exists());
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
    fn install_rejects_source_less_registry_package_and_preserves_alias() {
        let tmp = tempfile::tempdir().expect("project");
        let alias = tmp.path().join(format!("{MODULES_DIR}/string"));
        fs::create_dir_all(&alias).expect("tracked unscoped alias");
        fs::write(
            alias.join("index.phpx"),
            "export const compatibility = true;\n",
        )
        .expect("write tracked alias");

        let err = run_php_install_in(vec!["@deka/string".to_string()], true, false, tmp.path())
            .expect_err("source-less registry package must fail installation");
        assert!(err.to_string().contains("@deka/string"));
        assert!(err.to_string().contains("no .ds/.dsx sources"));
        assert!(
            !tmp.path()
                .join(MODULES_DIR)
                .join("@deka")
                .join("string")
                .exists()
        );
        assert_eq!(
            fs::read_to_string(alias.join("index.phpx")).expect("tracked alias survives"),
            "export const compatibility = true;\n"
        );
    }

    #[tokio::test]
    #[ignore = "legacy linkhash/harar registry support removed; revisit during package-manager cleanup (see dekaruntime/deka#330)"]
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
            let _registry = registry.clone();
            move || {
                run_php_install_in(
                    vec!["@scope/a".to_string(), "@scope/b".to_string()],
                    true,
                    false,
                    &root,
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
    #[ignore = "legacy linkhash/harar registry support removed; revisit during package-manager cleanup (see dekaruntime/deka#330)"]
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
            let _registry = registry.clone();
            move || {
                run_php_install_in(
                    vec!["@scope/a".to_string(), "@scope/b".to_string()],
                    true,
                    false,
                    &root,
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
            AxumPath((scope, package, version)): AxumPath<(String, String, String)>,
            State(Fixture(packages)): State<Fixture>,
        ) -> Json<serde_json::Value> {
            let name = format!("@{scope}/{package}");
            let dependencies = packages.get(&name).cloned().unwrap_or_else(|| json!({}));
            let digest = fixture_package_digest(&name, &version, &dependencies);
            Json(json!({
                "version": version,
                "repo": "fixture",
                "git_ref": "fixture",
                "digest": format!("sha256:{digest}"),
            }))
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

    fn fixture_package_digest(
        name: &str,
        version: &str,
        dependencies: &serde_json::Value,
    ) -> String {
        let root = tempfile::tempdir().expect("digest fixture tempdir");
        fs::write(
            root.path().join("deka.json"),
            json!({ "name": name, "version": version, "dependencies": dependencies }).to_string(),
        )
        .expect("digest fixture manifest");
        fs::write(
            root.path().join("index.phpx"),
            "export const fixture = true;\n",
        )
        .expect("digest fixture module");
        compute_package_integrity(root.path())
            .expect("digest fixture integrity")
            .fs_graph
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
