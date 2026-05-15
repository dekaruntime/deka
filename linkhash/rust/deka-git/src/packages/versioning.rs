use super::snapshot::build_api_snapshot;
use super::types::{ApiChangeKind, ApiSnapshot, ApiValidationIssue, PackageRelease};
use semver::Version;

pub(super) fn minimum_for_bump(previous: &Version, bump: ApiChangeKind) -> Version {
    let mut next = previous.clone();
    match bump {
        ApiChangeKind::Initial => next,
        ApiChangeKind::Patch => {
            next.patch += 1;
            next.pre = semver::Prerelease::EMPTY;
            next.build = semver::BuildMetadata::EMPTY;
            next
        }
        ApiChangeKind::Minor => {
            next.minor += 1;
            next.patch = 0;
            next.pre = semver::Prerelease::EMPTY;
            next.build = semver::BuildMetadata::EMPTY;
            next
        }
        ApiChangeKind::Major => {
            next.major += 1;
            next.minor = 0;
            next.patch = 0;
            next.pre = semver::Prerelease::EMPTY;
            next.build = semver::BuildMetadata::EMPTY;
            next
        }
    }
}

pub(super) fn parse_semver(value: &str) -> Result<Version, anyhow::Error> {
    Version::parse(value).map_err(|e| anyhow::anyhow!("invalid semver `{}`: {}", value, e))
}

pub(super) fn release_snapshot(release: &PackageRelease) -> Result<ApiSnapshot, anyhow::Error> {
    match &release.api_snapshot {
        Some(value) => Ok(serde_json::from_str(value)?),
        None => {
            let repo_path = crate::repo::storage::get_repo_path(&release.owner, &release.repo);
            if !repo_path.exists() {
                anyhow::bail!(
                    "latest release {}@{} has no api_snapshot and repo {}/{} is missing",
                    release.package_name,
                    release.version,
                    release.owner,
                    release.repo
                );
            }
            build_api_snapshot(&repo_path, &release.git_ref)
        }
    }
}

pub(super) fn classify_change(
    old: &ApiSnapshot,
    new: &ApiSnapshot,
) -> (ApiChangeKind, Vec<String>, Vec<ApiValidationIssue>) {
    let mut reasons = Vec::new();
    let mut issues = Vec::new();
    let mut has_major = false;
    let mut has_minor = false;

    for (key, old_sig) in &old.exports {
        match new.exports.get(key) {
            None => {
                has_major = true;
                reasons.push(format!("removed export `{}`", key));
                issues.push(ApiValidationIssue {
                    code: "API_REMOVED_EXPORT".to_string(),
                    severity: "error".to_string(),
                    symbol: key.clone(),
                    message: "public export was removed".to_string(),
                    old_source: Some(old_sig.source.clone()),
                    new_source: None,
                    old_signature: Some(old_sig.signature.clone()),
                    new_signature: None,
                });
            }
            Some(new_sig) if new_sig != old_sig => {
                has_major = true;
                reasons.push(format!("changed export `{}`", key));
                issues.push(ApiValidationIssue {
                    code: "API_CHANGED_EXPORT".to_string(),
                    severity: "error".to_string(),
                    symbol: key.clone(),
                    message: "public export signature changed".to_string(),
                    old_source: Some(old_sig.source.clone()),
                    new_source: Some(new_sig.source.clone()),
                    old_signature: Some(old_sig.signature.clone()),
                    new_signature: Some(new_sig.signature.clone()),
                });
            }
            _ => {}
        }
    }

    if !has_major {
        for (key, new_sig) in &new.exports {
            if !old.exports.contains_key(key) {
                has_minor = true;
                reasons.push(format!("added export `{}`", key));
                issues.push(ApiValidationIssue {
                    code: "API_ADDED_EXPORT".to_string(),
                    severity: "info".to_string(),
                    symbol: key.clone(),
                    message: "new public export was added".to_string(),
                    old_source: None,
                    new_source: Some(new_sig.source.clone()),
                    old_signature: None,
                    new_signature: Some(new_sig.signature.clone()),
                });
            }
        }
    }

    if has_major {
        (ApiChangeKind::Major, reasons, issues)
    } else if has_minor {
        (ApiChangeKind::Minor, reasons, issues)
    } else {
        (
            ApiChangeKind::Patch,
            vec!["no public API changes".to_string()],
            Vec::new(),
        )
    }
}
