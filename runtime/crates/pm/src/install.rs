use crate::{lock, payload::InstallPayload, spec::parse_package_spec};
use anyhow::{Context, Result, anyhow, bail};
use linkhash_client::LinkhashClient;
use modules_php::integrity::compute_package_integrity;
use runtime_core::module_spec::canonical_php_package_spec;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

pub async fn run_install(payload: InstallPayload) -> Result<()> {
    if payload.rehash {
        rehash_php_packages(&payload).await?;
        return Ok(());
    }

    run_php_install(payload.specs.clone(), payload.quiet).await
}

async fn run_php_install(specs: Vec<String>, quiet: bool) -> Result<()> {
    if specs.is_empty() {
        bail!("no PHP package specs provided (use --spec)");
    }

    let registry = linkhash_registry_url();
    let token = linkhash_token();
    let client = LinkhashClient::new(&registry, token.as_deref());
    let start = Instant::now();
    let mut installed_count = 0usize;

    for spec in specs {
        let normalized = normalize_php_spec(&spec)?;
        let (name, version_hint) = parse_package_spec(&normalized);
        let version_range = version_hint.as_deref().unwrap_or("latest");
        let resolved = client.resolve(&name, version_range)?;
        let destination = php_modules_path_for(&name)?;
        if destination.exists() {
            fs::remove_dir_all(&destination)
                .with_context(|| format!("failed to remove {}", destination.display()))?;
        }
        client.download(&name, &resolved.version, &destination)?;

        let package_integrity = compute_package_integrity(&destination)
            .map_err(|err| anyhow!("failed to compute package integrity for {}: {}", name, err))?;

        let metadata = json!({
            "repo": resolved.repo,
            "gitRef": resolved.git_ref,
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
            &resolved.name,
            format!("{}@{}", resolved.name, resolved.version),
            format!("linkhash:{}", resolved.name),
            metadata,
            String::new(),
        )?;
        installed_count += 1;
    }

    let duration = Instant::now().duration_since(start);
    emit_summary(installed_count, duration.as_millis() as u64, quiet)?;
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
        .unwrap_or_else(|_| "http://localhost:9418".to_string())
}

fn linkhash_token() -> Option<String> {
    std::env::var("LINKHASH_TOKEN")
        .or_else(|_| std::env::var("TANA_GIT_TOKEN"))
        .ok()
}

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
    use super::{php_modules_path_for, rehash_php_packages_in};
    use crate::{lock, payload::InstallPayload};
    use modules_php::integrity::compute_package_integrity;
    use serde_json::json;
    use std::fs;

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
    async fn rehash_preserves_deka_scope_when_resolving_package_root() {
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
