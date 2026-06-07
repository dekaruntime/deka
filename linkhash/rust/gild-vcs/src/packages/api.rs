use super::git::{git_ref_exists, parse_ls_tree_line};
use super::manifest::{
    capability_report, enforce_manifest_identity, enforce_release_tag_alignment,
    enforce_repo_identity, resolved_manifest, validate_package_name, validate_version,
};
use super::snapshot::build_api_snapshot;
use super::types::{
    ApiChangeKind, BlobResponse, DocSymbol, PackageDocsResponse, PackageRelease, PackageSummary,
    PublishPackageRequest, PublishPreflight, ReleaseTreeResponse,
};
use super::versioning::{classify_change, minimum_for_bump, parse_semver, release_snapshot};
use semver::Version;
use std::collections::BTreeMap;
use std::process::Command;

pub async fn preflight_publish(
    owner: &str,
    req: &PublishPackageRequest,
) -> Result<PublishPreflight, anyhow::Error> {
    let parsed_name = validate_package_name(&req.name)?;
    validate_version(&req.version)?;
    enforce_repo_identity(owner, &parsed_name, &req.repo)?;

    let requested = parse_semver(&req.version)?;
    let git_ref = req.git_ref.as_deref().unwrap_or("HEAD");
    let repo_path = crate::repo::storage::get_repo_path(owner, &req.repo);
    if !repo_path.exists() {
        anyhow::bail!("Repository does not exist: {}/{}", owner, req.repo);
    }

    if !git_ref_exists(&repo_path, git_ref)? {
        anyhow::bail!("Git ref does not exist: {}", git_ref);
    }

    enforce_release_tag_alignment(&repo_path, git_ref, &req.version)?;
    let manifest = resolved_manifest(&repo_path, git_ref, req.manifest.as_ref())?;
    enforce_manifest_identity(&manifest, req)?;

    let snapshot = build_api_snapshot(&repo_path, git_ref)?;
    let capabilities = capability_report(&repo_path, git_ref, Some(&manifest.raw))?;
    if !capabilities.missing.is_empty() {
        anyhow::bail!(
            "capability declaration required for: {} (add deka.security.allow.* in manifest)",
            capabilities.missing.join(", ")
        );
    }
    let previous = latest_release_for_package(&req.name).await?;

    let (previous_version, required_bump, minimum_allowed, allowed, reasons, issues) =
        if let Some(prev_release) = previous {
            let prev_version = parse_semver(&prev_release.version)?;
            if requested <= prev_version {
                anyhow::bail!(
                    "version {} must be greater than latest published {}",
                    requested,
                    prev_version
                );
            }

            let previous_snapshot = release_snapshot(&prev_release)?;
            let (detected_change, reasons, issues) = classify_change(&previous_snapshot, &snapshot);
            let required_bump = detected_change;
            let minimum_allowed = minimum_for_bump(&prev_version, required_bump);
            let allowed = requested >= minimum_allowed;
            (
                Some(prev_release.version),
                required_bump,
                minimum_allowed,
                allowed,
                reasons,
                issues,
            )
        } else {
            (
                None,
                ApiChangeKind::Initial,
                requested.clone(),
                true,
                vec!["initial publish".to_string()],
                Vec::new(),
            )
        };

    Ok(PublishPreflight {
        package_name: req.name.clone(),
        requested_version: req.version.clone(),
        previous_version,
        detected_change: required_bump,
        required_bump,
        minimum_allowed_version: minimum_allowed.to_string(),
        allowed,
        reasons,
        issues,
        capabilities,
    })
}

pub async fn publish(
    owner: &str,
    req: PublishPackageRequest,
) -> Result<PackageRelease, anyhow::Error> {
    let preflight = preflight_publish(owner, &req).await?;
    if !preflight.allowed {
        let mut details = String::new();
        for issue in &preflight.issues {
            let old_src = issue.old_source.as_deref().unwrap_or("-");
            let new_src = issue.new_source.as_deref().unwrap_or("-");
            details.push_str(&format!(
                "\n- [{}] {} ({}) old={} new={}",
                issue.code, issue.message, issue.symbol, old_src, new_src
            ));
        }
        anyhow::bail!(
            "semver gate blocked publish: detected {:?} API change, requested {}, minimum allowed {}{}",
            preflight.required_bump,
            preflight.requested_version,
            preflight.minimum_allowed_version,
            details
        );
    }

    let git_ref = req.git_ref.clone().unwrap_or_else(|| "HEAD".to_string());
    let repo_path = crate::repo::storage::get_repo_path(owner, &req.repo);
    let manifest = resolved_manifest(&repo_path, &git_ref, req.manifest.as_ref())?;
    let snapshot = build_api_snapshot(&repo_path, &git_ref)?;
    let snapshot_json = serde_json::to_string(&snapshot)?;
    let manifest_json = serde_json::to_string(&manifest.raw)?;
    let capability_meta_json = serde_json::to_string(&preflight.capabilities)?;

    let pool = crate::db::pool();
    let row = sqlx::query_as::<_, PackageRelease>(
        r#"
        INSERT INTO package_releases
            (package_name, version, owner, repo, git_ref, description, manifest, api_snapshot, api_change_kind, required_bump, capability_metadata)
        VALUES
            (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING package_name, version, owner, repo, git_ref, description, manifest, api_snapshot, api_change_kind, required_bump, capability_metadata, created_at
        "#,
    )
    .bind(&req.name)
    .bind(&req.version)
    .bind(owner)
    .bind(&req.repo)
    .bind(&git_ref)
    .bind(&req.description)
    .bind(&manifest_json)
    .bind(&snapshot_json)
    .bind(preflight.detected_change.as_str())
    .bind(preflight.required_bump.as_str())
    .bind(&capability_meta_json)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

pub async fn get_package(name: &str) -> Result<PackageSummary, sqlx::Error> {
    let pool = crate::db::pool();
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT version
        FROM package_releases
        WHERE package_name = ?
        ORDER BY created_at DESC
        "#,
    )
    .bind(name)
    .fetch_all(pool)
    .await?;

    let versions: Vec<String> = rows.into_iter().map(|(v,)| v).collect();
    let latest = versions.first().cloned();

    Ok(PackageSummary {
        name: name.to_string(),
        versions,
        latest,
    })
}

pub async fn get_release(name: &str, version: &str) -> Result<Option<PackageRelease>, sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query_as::<_, PackageRelease>(
        r#"
        SELECT package_name, version, owner, repo, git_ref, description, manifest, api_snapshot, api_change_kind, required_bump, capability_metadata, created_at
        FROM package_releases
        WHERE package_name = ? AND version = ?
        "#,
    )
    .bind(name)
    .bind(version)
    .fetch_optional(pool)
    .await
}

pub async fn get_release_docs(
    name: &str,
    version: &str,
) -> Result<Option<PackageDocsResponse>, anyhow::Error> {
    let Some(release) = get_release(name, version).await? else {
        return Ok(None);
    };
    let snapshot = release_snapshot(&release)?;
    let mut symbols = Vec::new();
    for (symbol, entry) in snapshot.exports {
        symbols.push(DocSymbol {
            symbol,
            kind: entry.kind,
            signature: entry.signature,
            source: entry.source,
            summary: entry.summary,
            description: entry.description,
            examples: entry.examples,
        });
    }
    symbols.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    Ok(Some(PackageDocsResponse {
        package_name: release.package_name,
        version: release.version,
        symbols,
    }))
}

pub async fn get_release_tree(
    name: &str,
    version: &str,
) -> Result<Option<ReleaseTreeResponse>, anyhow::Error> {
    let Some(release) = get_release(name, version).await? else {
        return Ok(None);
    };
    let repo_path = crate::repo::storage::get_repo_path(&release.owner, &release.repo);
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repo_path.display()))
        .args(["ls-tree", "-r", "-l", &release.git_ref])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git ls-tree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut entries = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(entry) = parse_ls_tree_line(line) {
            entries.push(entry);
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Some(ReleaseTreeResponse {
        package_name: release.package_name,
        version: release.version,
        git_ref: release.git_ref,
        entries,
    }))
}

pub async fn get_release_blob(
    name: &str,
    version: &str,
    path: &str,
) -> Result<Option<BlobResponse>, anyhow::Error> {
    let Some(release) = get_release(name, version).await? else {
        return Ok(None);
    };
    let clean_path = path.trim().trim_start_matches('/');
    if clean_path.is_empty() {
        anyhow::bail!("path is required");
    }
    let repo_path = crate::repo::storage::get_repo_path(&release.owner, &release.repo);
    let spec = format!("{}:{}", release.git_ref, clean_path);
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repo_path.display()))
        .arg("show")
        .arg(spec)
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(BlobResponse {
        package_name: release.package_name,
        version: release.version,
        path: clean_path.to_string(),
        git_ref: release.git_ref,
        content: String::from_utf8_lossy(&output.stdout).to_string(),
    }))
}

pub async fn get_latest_release(name: &str) -> Result<Option<PackageRelease>, anyhow::Error> {
    let pool = crate::db::pool();
    let rows: Vec<PackageRelease> = sqlx::query_as(
        r#"
        SELECT package_name, version, owner, repo, git_ref, description, manifest, api_snapshot, api_change_kind, required_bump, capability_metadata, created_at
        FROM package_releases
        WHERE package_name = ?
        ORDER BY created_at DESC
        "#,
    )
    .bind(name)
    .fetch_all(pool)
    .await?;

    // Find the highest semver version (not just most recent by date)
    let mut best: Option<(Version, PackageRelease)> = None;
    for row in rows {
        if let Ok(v) = Version::parse(&row.version) {
            match &best {
                Some((bv, _)) if v > *bv => best = Some((v, row)),
                None => best = Some((v, row)),
                _ => {}
            }
        }
    }

    Ok(best.map(|(_, release)| release))
}

pub async fn list_all_packages() -> Result<Vec<PackageSummary>, sqlx::Error> {
    let pool = crate::db::pool();
    let rows: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT package_name, version
        FROM package_releases
        ORDER BY package_name, created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut packages: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, version) in rows {
        packages.entry(name).or_default().push(version);
    }

    Ok(packages
        .into_iter()
        .map(|(name, versions)| {
            let latest = versions.first().cloned();
            PackageSummary {
                name,
                versions,
                latest,
            }
        })
        .collect())
}
async fn latest_release_for_package(name: &str) -> Result<Option<PackageRelease>, sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query_as::<_, PackageRelease>(
        r#"
        SELECT package_name, version, owner, repo, git_ref, description, manifest, api_snapshot, api_change_kind, required_bump, capability_metadata, created_at
        FROM package_releases
        WHERE package_name = ?
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(name)
    .fetch_optional(pool)
    .await
}
