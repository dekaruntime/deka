use anyhow::{Context, Result, bail};
use runtime_core::module_spec::{canonical_php_package_spec, is_valid_package_name};
use runtime_core::modules::{LinkEntry, read_links_at, write_links_at};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Link a package working tree into a consumer project without modifying the
/// consumer's dependency manifest, lockfile, or the package working tree.
pub fn link_package_at(project_dir: &Path, package_dir: &Path) -> Result<String> {
    let target = fs::canonicalize(package_dir).with_context(|| {
        format!(
            "local package directory does not exist: {}",
            package_dir.display()
        )
    })?;
    if !target.is_dir() {
        bail!(
            "local package target is not a directory: {}",
            target.display()
        );
    }

    let manifest_path = target.join("deka.json");
    let raw = fs::read_to_string(&manifest_path)
        .with_context(|| format!("linked package is missing {}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_str(&raw).with_context(|| {
        format!(
            "linked package manifest is invalid: {}",
            manifest_path.display()
        )
    })?;
    let name = manifest
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .context("linked package deka.json must contain a non-empty `name`")?;
    if !is_valid_package_name(name) {
        bail!("linked package name must use @scope/name format: `{name}`");
    }

    let mut links = read_links_at(project_dir).map_err(|error| anyhow::anyhow!(error))?;
    links
        .packages
        .insert(name.to_string(), LinkEntry { path: target });
    write_links_at(project_dir, &links).map_err(|error| anyhow::anyhow!(error))?;
    Ok(name.to_string())
}

/// Remove a project-local link. This only changes `.deka/links.json`; it
/// never removes or edits the linked package directory.
pub fn unlink_package_at(project_dir: &Path, package: &str) -> Result<PathBuf> {
    let name = canonical_php_package_spec(package)
        .filter(|name| is_valid_package_name(name))
        .with_context(|| {
            format!("invalid linked package name `{package}` (expected @scope/name)")
        })?;
    let mut links = read_links_at(project_dir).map_err(|error| anyhow::anyhow!(error))?;
    let Some(entry) = links.packages.remove(&name) else {
        bail!("no local link registered for {name}");
    };
    let target = entry.path;
    write_links_at(project_dir, &links).map_err(|error| anyhow::anyhow!(error))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::{link_package_at, unlink_package_at};
    use runtime_core::modules::{links_path, read_links_at};
    use std::fs;

    #[test]
    fn link_persists_package_identity_and_canonical_target() {
        let project = tempfile::tempdir().unwrap();
        let package = tempfile::tempdir().unwrap();
        fs::write(
            package.path().join("deka.json"),
            r#"{"name":"@deka/example","version":"0.1.0"}"#,
        )
        .unwrap();

        assert_eq!(
            link_package_at(project.path(), package.path()).unwrap(),
            "@deka/example"
        );
        let links = read_links_at(project.path()).unwrap();
        assert_eq!(
            links.packages["@deka/example"].path,
            package.path().canonicalize().unwrap()
        );
        assert!(links_path(project.path()).is_file());
    }

    #[test]
    fn unlink_only_removes_state_and_preserves_target() {
        let project = tempfile::tempdir().unwrap();
        let package = tempfile::tempdir().unwrap();
        fs::write(
            package.path().join("deka.json"),
            r#"{"name":"@deka/example","version":"0.1.0"}"#,
        )
        .unwrap();
        fs::write(package.path().join("index.ds"), "export const value = 1;\n").unwrap();

        link_package_at(project.path(), package.path()).unwrap();
        let target = unlink_package_at(project.path(), "@deka/example").unwrap();
        assert_eq!(target, package.path().canonicalize().unwrap());
        assert!(package.path().join("index.ds").is_file());
        assert!(read_links_at(project.path()).unwrap().packages.is_empty());
    }

    #[test]
    fn link_rejects_unscoped_package_names() {
        let project = tempfile::tempdir().unwrap();
        let package = tempfile::tempdir().unwrap();
        fs::write(package.path().join("deka.json"), r#"{"name":"example"}"#).unwrap();

        let error = link_package_at(project.path(), package.path()).unwrap_err();
        assert!(error.to_string().contains("@scope/name"));
        assert!(!links_path(project.path()).exists());
    }
}
