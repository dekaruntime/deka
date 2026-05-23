use chrono::{DateTime, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::BTreeMap;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct PublishPackageRequest {
    pub name: String,
    pub version: String,
    pub repo: String,
    pub git_ref: Option<String>,
    pub description: Option<String>,
    pub manifest: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct PackageRelease {
    pub package_name: String,
    pub version: String,
    pub owner: String,
    pub repo: String,
    pub git_ref: String,
    pub description: Option<String>,
    pub manifest: Option<serde_json::Value>,
    pub api_snapshot: Option<serde_json::Value>,
    pub api_change_kind: Option<String>,
    pub required_bump: Option<String>,
    pub capability_metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct PackageSummary {
    pub name: String,
    pub versions: Vec<String>,
    pub latest: Option<String>,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApiChangeKind {
    Initial,
    Patch,
    Minor,
    Major,
}

#[derive(Debug, Serialize, Clone)]
pub struct PublishPreflight {
    pub package_name: String,
    pub requested_version: String,
    pub previous_version: Option<String>,
    pub detected_change: ApiChangeKind,
    pub required_bump: ApiChangeKind,
    pub minimum_allowed_version: String,
    pub allowed: bool,
    pub reasons: Vec<String>,
    pub issues: Vec<ApiValidationIssue>,
    pub capabilities: CapabilityReport,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct ApiSnapshot {
    pub exports: BTreeMap<String, ExportSignature>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ExportSignature {
    pub kind: String,
    pub signature: String,
    pub source: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub examples: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ApiValidationIssue {
    pub code: String,
    pub severity: String,
    pub symbol: String,
    pub message: String,
    pub old_source: Option<String>,
    pub new_source: Option<String>,
    pub old_signature: Option<String>,
    pub new_signature: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CapabilityReport {
    pub detected: Vec<String>,
    pub declared: Vec<String>,
    pub missing: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PackageDocsResponse {
    pub package_name: String,
    pub version: String,
    pub symbols: Vec<DocSymbol>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DocSymbol {
    pub symbol: String,
    pub kind: String,
    pub signature: String,
    pub source: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub examples: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReleaseTreeResponse {
    pub package_name: String,
    pub version: String,
    pub git_ref: String,
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TreeEntry {
    pub mode: String,
    pub kind: String,
    pub object: String,
    pub size: Option<u64>,
    pub path: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct BlobResponse {
    pub package_name: String,
    pub version: String,
    pub path: String,
    pub git_ref: String,
    pub content: String,
}

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
    let snapshot_json = serde_json::to_value(snapshot)?;
    let capability_meta_json = serde_json::to_value(preflight.capabilities.clone())?;

    let pool = crate::db::pool();
    let row = sqlx::query_as::<_, PackageRelease>(
        r#"
        INSERT INTO package_releases
            (package_name, version, owner, repo, git_ref, description, manifest, api_snapshot, api_change_kind, required_bump, capability_metadata)
        VALUES
            ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING package_name, version, owner, repo, git_ref, description, manifest, api_snapshot, api_change_kind, required_bump, capability_metadata, created_at
        "#,
    )
    .bind(&req.name)
    .bind(&req.version)
    .bind(owner)
    .bind(&req.repo)
    .bind(&git_ref)
    .bind(&req.description)
    .bind(&manifest.raw)
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
        WHERE package_name = $1
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
        WHERE package_name = $1 AND version = $2
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

pub async fn get_release_doc_symbol(
    name: &str,
    version: &str,
    symbol: &str,
) -> Result<Option<DocSymbol>, anyhow::Error> {
    let Some(response) = get_release_docs(name, version).await? else {
        return Ok(None);
    };
    Ok(response
        .symbols
        .into_iter()
        .find(|entry| entry.symbol == symbol))
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

pub fn build_release_tarball(release: &PackageRelease) -> Result<Vec<u8>, anyhow::Error> {
    let repo_path = crate::repo::storage::get_repo_path(&release.owner, &release.repo);
    if !repo_path.exists() {
        anyhow::bail!(
            "Repository not found for package {}@{}",
            release.package_name,
            release.version
        );
    }

    let prefix = "package/".to_string();
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repo_path.display()))
        .arg("archive")
        .arg("--format=tar.gz")
        .arg(format!("--prefix={}", prefix))
        .arg(&release.git_ref)
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "git archive failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(output.stdout)
}

impl ApiChangeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }
}

fn minimum_for_bump(previous: &Version, bump: ApiChangeKind) -> Version {
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

fn parse_semver(value: &str) -> Result<Version, anyhow::Error> {
    Version::parse(value).map_err(|e| anyhow::anyhow!("invalid semver `{}`: {}", value, e))
}

fn release_snapshot(release: &PackageRelease) -> Result<ApiSnapshot, anyhow::Error> {
    match &release.api_snapshot {
        Some(value) => Ok(serde_json::from_value(value.clone())?),
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

fn classify_change(
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

async fn latest_release_for_package(name: &str) -> Result<Option<PackageRelease>, sqlx::Error> {
    let pool = crate::db::pool();
    sqlx::query_as::<_, PackageRelease>(
        r#"
        SELECT package_name, version, owner, repo, git_ref, description, manifest, api_snapshot, api_change_kind, required_bump, capability_metadata, created_at
        FROM package_releases
        WHERE package_name = $1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(name)
    .fetch_optional(pool)
    .await
}

fn git_ref_exists(repo_path: &std::path::Path, git_ref: &str) -> Result<bool, anyhow::Error> {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repo_path.display()))
        .arg("rev-parse")
        .arg("--verify")
        .arg(git_ref)
        .output()?;

    Ok(output.status.success())
}

fn build_api_snapshot(
    repo_path: &std::path::Path,
    git_ref: &str,
) -> Result<ApiSnapshot, anyhow::Error> {
    let files = list_files_at_ref(repo_path, git_ref)?;
    let mut exports = BTreeMap::new();

    for file in files {
        if !file.ends_with(".phpx") || file.contains("/.cache/") {
            continue;
        }
        let source = git_show_file(repo_path, git_ref, &file)?;
        parse_file_exports(&file, &source, &mut exports);
    }

    Ok(ApiSnapshot { exports })
}

fn list_files_at_ref(
    repo_path: &std::path::Path,
    git_ref: &str,
) -> Result<Vec<String>, anyhow::Error> {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repo_path.display()))
        .arg("ls-tree")
        .arg("-r")
        .arg("--name-only")
        .arg(git_ref)
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "git ls-tree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn git_show_file(
    repo_path: &std::path::Path,
    git_ref: &str,
    file: &str,
) -> Result<String, anyhow::Error> {
    let spec = format!("{}:{}", git_ref, file);
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repo_path.display()))
        .arg("show")
        .arg(spec)
        .output()?;

    if !output.status.success() {
        anyhow::bail!(
            "git show failed for {}: {}",
            file,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_file_exports(file: &str, source: &str, out: &mut BTreeMap<String, ExportSignature>) {
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if !trimmed.starts_with("export ") {
            i += 1;
            continue;
        }

        if trimmed.starts_with("export struct ") || trimmed.starts_with("export enum ") {
            let (decl, next) = collect_brace_block(&lines, i);
            let docs = parse_doc_comment_above(&lines, i);
            if let Some((kind, name)) = extract_named_kind(trimmed) {
                insert_export(file, &kind, &name, &decl, i + 1, docs.as_ref(), out);
            }
            i = next;
            continue;
        }

        let docs = parse_doc_comment_above(&lines, i);
        if trimmed.starts_with("export {") {
            let (decl, next) = collect_brace_block(&lines, i);
            for name in parse_reexport_names(&decl) {
                insert_export(file, "reexport", &name, &decl, i + 1, docs.as_ref(), out);
            }
            i = next;
            continue;
        }

        let (decl, next) = collect_statement(&lines, i);
        if trimmed.starts_with("export function ") {
            if let Some(name) = extract_name_after(trimmed, "export function") {
                insert_export(file, "function", &name, &decl, i + 1, docs.as_ref(), out);
            }
        } else if trimmed.starts_with("export const ") {
            if let Some(name) = extract_name_after(trimmed, "export const") {
                insert_export(file, "const", &name, &decl, i + 1, docs.as_ref(), out);
            }
        } else if trimmed.starts_with("export type ") {
            if let Some(name) = extract_name_after(trimmed, "export type") {
                insert_export(file, "type", &name, &decl, i + 1, docs.as_ref(), out);
            }
        }

        i = next;
    }
}

fn module_id(file: &str) -> String {
    file.strip_suffix(".phpx")
        .unwrap_or(file)
        .trim_start_matches("./")
        .to_string()
}

fn insert_export(
    file: &str,
    kind: &str,
    name: &str,
    decl: &str,
    line: usize,
    docs: Option<&DocComment>,
    out: &mut BTreeMap<String, ExportSignature>,
) {
    let key = format!("{}::{}", module_id(file), name);
    out.insert(
        key,
        ExportSignature {
            kind: kind.to_string(),
            signature: docs
                .and_then(|d| d.typed_signature.clone())
                .unwrap_or_else(|| normalize_ws(decl)),
            source: format!("{}:{}", file, line),
            summary: docs.and_then(|d| d.summary.clone()),
            description: docs.and_then(|d| d.description.clone()),
            examples: docs.map(|d| d.examples.clone()).unwrap_or_default(),
        },
    );
}

#[derive(Debug, Clone, Default)]
struct DocComment {
    summary: Option<String>,
    description: Option<String>,
    examples: Vec<String>,
    typed_signature: Option<String>,
}

fn parse_doc_comment_above(lines: &[&str], export_line: usize) -> Option<DocComment> {
    if export_line == 0 {
        return None;
    }

    let mut i = export_line;
    while i > 0 {
        i -= 1;
        let line = lines[i].trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("///") {
            let mut block = vec![lines[i].to_string()];
            let mut j = i;
            while j > 0 {
                let prev = lines[j - 1].trim_start();
                if prev.starts_with("///") {
                    j -= 1;
                    block.push(lines[j].to_string());
                    continue;
                }
                break;
            }
            block.reverse();
            return parse_slash_doc_block(&block);
        }

        if !line.ends_with("*/") {
            return None;
        }

        let mut block = vec![lines[i].to_string()];
        let mut j = i;
        let found_start = line.starts_with("/**");
        while !found_start {
            if j == 0 {
                return None;
            }
            j -= 1;
            let current = lines[j];
            block.push(current.to_string());
            if current.trim_start().starts_with("/**") {
                break;
            }
        }
        block.reverse();
        return parse_doc_block(&block);
    }
    None
}

fn parse_doc_block(block: &[String]) -> Option<DocComment> {
    if block.is_empty() {
        return None;
    }

    let mut lines = Vec::new();
    for (idx, raw) in block.iter().enumerate() {
        let mut line = raw.trim().to_string();
        if idx == 0 {
            line = line.trim_start_matches("/**").trim().to_string();
        }
        if idx + 1 == block.len() {
            line = line.trim_end_matches("*/").trim().to_string();
        }
        line = line.trim_start_matches('*').trim().to_string();
        if !line.is_empty() {
            lines.push(line);
        }
    }

    if lines.is_empty() {
        return None;
    }

    let mut summary = None;
    let mut description_lines = Vec::new();
    let mut examples = Vec::new();
    for line in lines {
        if line.starts_with("@example") {
            let example = line.trim_start_matches("@example").trim().to_string();
            if !example.is_empty() {
                examples.push(example);
            }
            continue;
        }
        if line.starts_with('@') {
            continue;
        }
        if summary.is_none() {
            summary = Some(line.clone());
        }
        description_lines.push(line);
    }

    let description = if description_lines.is_empty() {
        None
    } else {
        Some(description_lines.join("\n"))
    };

    Some(DocComment {
        summary,
        description,
        examples,
        typed_signature: None,
    })
}

fn parse_slash_doc_block(block: &[String]) -> Option<DocComment> {
    if block.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    for raw in block {
        let mut line = raw.trim_start().to_string();
        line = line.trim_start_matches("///").trim().to_string();
        if !line.is_empty() {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        return None;
    }

    let mut summary = None;
    let mut description = None;
    let mut examples = Vec::new();
    let mut typed_signature = None;

    if let Some(function_name) = extract_xml_attr(&lines, "Function", "name") {
        let params = extract_parameter_signature_parts(&lines);
        let return_type = extract_xml_attr(&lines, "ReturnType", "type");
        let mut sig = format!("{}(", function_name);
        sig.push_str(&params.join(", "));
        sig.push(')');
        if let Some(rt) = return_type {
            if !rt.trim().is_empty() {
                sig.push_str(": ");
                sig.push_str(rt.trim());
            }
        }
        typed_signature = Some(sig);
    }

    if let Some(desc) = extract_xml_tag(&lines, "Description") {
        let clean = desc.trim().to_string();
        if !clean.is_empty() {
            summary = Some(clean.clone());
            description = Some(clean);
        }
    }

    for line in &lines {
        if line.starts_with("@example") {
            let ex = line.trim_start_matches("@example").trim().to_string();
            if !ex.is_empty() {
                examples.push(ex);
            }
        }
    }

    if summary.is_none() {
        for line in &lines {
            if line.starts_with("docid:") || line.starts_with('<') {
                continue;
            }
            let clean = line.trim().to_string();
            if !clean.is_empty() {
                summary = Some(clean.clone());
                description = Some(clean);
                break;
            }
        }
    }

    if summary.is_none() && description.is_none() && examples.is_empty() {
        return None;
    }

    Some(DocComment {
        summary,
        description,
        examples,
        typed_signature,
    })
}

fn extract_xml_attr(lines: &[String], tag: &str, attr: &str) -> Option<String> {
    let tag_open = format!("<{}", tag);
    let needle = format!("{}=\"", attr);
    for line in lines {
        if !line.contains(&tag_open) {
            continue;
        }
        let start = line.find(&needle)?;
        let rest = &line[start + needle.len()..];
        let end = rest.find('"')?;
        let value = rest[..end].trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

fn extract_parameter_signature_parts(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for line in lines {
        if !line.contains("<Parameter ") {
            continue;
        }
        let name = extract_attr_from_line(line, "name").unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let param_type = extract_attr_from_line(line, "type").unwrap_or_default();
        if param_type.is_empty() {
            out.push(name);
        } else {
            out.push(format!("{} {}", name, param_type));
        }
    }
    out
}

fn extract_attr_from_line(line: &str, attr: &str) -> Option<String> {
    let needle = format!("{}=\"", attr);
    let start = line.find(&needle)?;
    let rest = &line[start + needle.len()..];
    let end = rest.find('"')?;
    let value = rest[..end].trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn extract_xml_tag(lines: &[String], tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let mut collecting = false;
    let mut out = Vec::new();
    for line in lines {
        if !collecting {
            if let Some(start) = line.find(&open) {
                let rest = &line[start + open.len()..];
                if let Some(end) = rest.find(&close) {
                    let inner = rest[..end].trim().to_string();
                    if !inner.is_empty() {
                        return Some(inner);
                    }
                    continue;
                }
                collecting = true;
                let first = rest.trim();
                if !first.is_empty() {
                    out.push(first.to_string());
                }
            }
            continue;
        }

        if let Some(end) = line.find(&close) {
            let part = line[..end].trim();
            if !part.is_empty() {
                out.push(part.to_string());
            }
            break;
        }
        let t = line.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("\n"))
    }
}

fn collect_statement(lines: &[&str], start: usize) -> (String, usize) {
    let mut out = Vec::new();
    let mut i = start;
    while i < lines.len() {
        out.push(lines[i]);
        let t = lines[i].trim_end();
        if t.ends_with(';') || t.ends_with('{') {
            i += 1;
            break;
        }
        i += 1;
    }
    (out.join("\n"), i)
}

fn collect_brace_block(lines: &[&str], start: usize) -> (String, usize) {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut seen_open = false;
    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        out.push(line);
        for ch in line.chars() {
            if ch == '{' {
                depth += 1;
                seen_open = true;
            } else if ch == '}' {
                depth -= 1;
            }
        }
        i += 1;
        if seen_open && depth <= 0 {
            break;
        }
    }
    (out.join("\n"), i)
}

fn extract_named_kind(trimmed: &str) -> Option<(String, String)> {
    if trimmed.starts_with("export struct ") {
        extract_name_after(trimmed, "export struct").map(|name| ("struct".to_string(), name))
    } else if trimmed.starts_with("export enum ") {
        extract_name_after(trimmed, "export enum").map(|name| ("enum".to_string(), name))
    } else {
        None
    }
}

fn extract_name_after(line: &str, prefix: &str) -> Option<String> {
    let tail = line.strip_prefix(prefix)?.trim_start();
    let mut name = String::new();
    for ch in tail.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            name.push(ch);
        } else {
            break;
        }
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn parse_reexport_names(decl: &str) -> Vec<String> {
    let open = match decl.find('{') {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let close = match decl[open + 1..].find('}') {
        Some(idx) => open + 1 + idx,
        None => return Vec::new(),
    };

    decl[open + 1..close]
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .filter_map(|item| {
            if let Some((_, alias)) = item.split_once(" as ") {
                let v = alias.trim();
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                }
            } else {
                Some(item.to_string())
            }
        })
        .collect()
}

fn parse_ls_tree_line(line: &str) -> Option<TreeEntry> {
    let (left, path) = line.split_once('\t')?;
    let mut parts = left.split_whitespace();
    let mode = parts.next()?.to_string();
    let kind = parts.next()?.to_string();
    let object = parts.next()?.to_string();
    let size_raw = parts.next().unwrap_or("-");
    let size = if size_raw == "-" {
        None
    } else {
        size_raw.parse::<u64>().ok()
    };
    Some(TreeEntry {
        mode,
        kind,
        object,
        size,
        path: path.to_string(),
    })
}

fn normalize_ws(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn capability_report(
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
struct ParsedPackageName {
    scope: String,
    package: String,
}

#[derive(Debug, Clone)]
struct ManifestIdentity {
    name: String,
    version: String,
    raw: serde_json::Value,
}

fn enforce_repo_identity(
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

fn enforce_release_tag_alignment(
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

fn resolved_manifest(
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

fn enforce_manifest_identity(
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

fn declared_capabilities(manifest: Option<&serde_json::Value>) -> Vec<String> {
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

fn validate_package_name(name: &str) -> Result<ParsedPackageName, anyhow::Error> {
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

fn validate_version(version: &str) -> Result<(), anyhow::Error> {
    if version.is_empty() || version.len() > 64 {
        anyhow::bail!("Invalid version length");
    }
    parse_semver(version)?;
    Ok(())
}

fn git_resolve_commit(
    repo_path: &std::path::Path,
    git_ref: &str,
) -> Result<Option<String>, anyhow::Error> {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repo_path.display()))
        .arg("rev-parse")
        .arg("--verify")
        .arg(git_ref)
        .output()?;

    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(items: &[(&str, &str)]) -> ApiSnapshot {
        let mut exports = BTreeMap::new();
        for (k, sig) in items {
            exports.insert(
                (*k).to_string(),
                ExportSignature {
                    kind: "function".to_string(),
                    signature: (*sig).to_string(),
                    source: "src/index.phpx:1".to_string(),
                    summary: None,
                    description: None,
                    examples: Vec::new(),
                },
            );
        }
        ApiSnapshot { exports }
    }

    #[test]
    fn classify_patch_when_no_change() {
        let a = snap(&[("src/index::foo", "export function foo($x: int): int {")]);
        let b = snap(&[("src/index::foo", "export function foo($x: int): int {")]);
        let (kind, reasons, issues) = classify_change(&a, &b);
        assert_eq!(kind, ApiChangeKind::Patch);
        assert_eq!(reasons, vec!["no public API changes"]);
        assert!(issues.is_empty());
    }

    #[test]
    fn classify_minor_when_added_export() {
        let a = snap(&[("src/index::foo", "export function foo($x: int): int {")]);
        let b = snap(&[
            ("src/index::foo", "export function foo($x: int): int {"),
            ("src/index::bar", "export function bar(): int {"),
        ]);
        let (kind, reasons, issues) = classify_change(&a, &b);
        assert_eq!(kind, ApiChangeKind::Minor);
        assert!(reasons.iter().any(|r| r.contains("added export")));
        assert!(issues.iter().any(|i| i.code == "API_ADDED_EXPORT"));
    }

    #[test]
    fn classify_major_when_changed_or_removed() {
        let a = snap(&[
            ("src/index::foo", "export function foo($x: int): int {"),
            ("src/index::bar", "export function bar(): int {"),
        ]);
        let b = snap(&[("src/index::foo", "export function foo($x: string): int {")]);
        let (kind, reasons, issues) = classify_change(&a, &b);
        assert_eq!(kind, ApiChangeKind::Major);
        assert!(reasons.iter().any(|r| r.contains("changed export")));
        assert!(reasons.iter().any(|r| r.contains("removed export")));
        assert!(issues.iter().any(|i| i.code == "API_CHANGED_EXPORT"));
        assert!(issues.iter().any(|i| i.code == "API_REMOVED_EXPORT"));
    }

    #[test]
    fn parses_reexport_aliases() {
        let names = parse_reexport_names("export { Foo as Bar, Baz } from './x.phpx';");
        assert_eq!(names, vec!["Bar".to_string(), "Baz".to_string()]);
    }

    #[test]
    fn parses_multiline_reexport_exports() {
        let source = r#"
export {
  json_encode,
  json_decode,
} from 'encoding/json';
"#;
        let mut out = BTreeMap::new();
        parse_file_exports("json/index.phpx", source, &mut out);
        assert!(out.contains_key("json/index::json_encode"));
        assert!(out.contains_key("json/index::json_decode"));
    }

    #[test]
    fn semver_minimum_for_bumps() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(
            minimum_for_bump(&v, ApiChangeKind::Patch).to_string(),
            "1.2.4"
        );
        assert_eq!(
            minimum_for_bump(&v, ApiChangeKind::Minor).to_string(),
            "1.3.0"
        );
        assert_eq!(
            minimum_for_bump(&v, ApiChangeKind::Major).to_string(),
            "2.0.0"
        );
    }

    #[test]
    fn parses_declared_capabilities_from_manifest() {
        let manifest = serde_json::json!({
            "deka.security": {
                "allow": {
                    "run": ["git"],
                    "dynamic": true
                }
            }
        });
        let declared = declared_capabilities(Some(&manifest));
        assert!(declared.iter().any(|v| v == "run"));
        assert!(declared.iter().any(|v| v == "dynamic"));
    }

    #[test]
    fn parses_doc_comment_above_export() {
        let source = r#"
/** Adds two integers.
 * More detail line.
 * @example sum(1, 2)
 */
export function sum($a: int, $b: int): int {
  return $a + $b;
}
"#;
        let mut out = BTreeMap::new();
        parse_file_exports("math.phpx", source, &mut out);
        let entry = out.get("math::sum").expect("export");
        assert_eq!(entry.summary.as_deref(), Some("Adds two integers."));
        assert_eq!(
            entry.description.as_deref(),
            Some("Adds two integers.\nMore detail line.")
        );
        assert_eq!(entry.examples, vec!["sum(1, 2)".to_string()]);
    }

    #[test]
    fn parses_triple_slash_xml_description() {
        let source = r#"
/// docid: phpx/array/array()
/// <Function name="array">
///   <Description>
///     Creates an array from the given arguments.
///   </Description>
///   <Parameter name="$values" type="mixed" required="false" />
///   <ReturnType type="array" />
/// </Function>
export function array() {
  return [];
}
"#;
        let mut out = BTreeMap::new();
        parse_file_exports("array.phpx", source, &mut out);
        let entry = out.get("array::array").expect("export");
        assert_eq!(
            entry.summary.as_deref(),
            Some("Creates an array from the given arguments.")
        );
        assert_eq!(
            entry.description.as_deref(),
            Some("Creates an array from the given arguments.")
        );
        assert_eq!(entry.signature, "array($values mixed): array");
    }

    #[test]
    fn parses_ls_tree_rows() {
        let row = "100644 blob abcdef1234567890 42\tsrc/main.phpx";
        let parsed = parse_ls_tree_line(row).expect("tree entry");
        assert_eq!(parsed.mode, "100644");
        assert_eq!(parsed.kind, "blob");
        assert_eq!(parsed.object, "abcdef1234567890");
        assert_eq!(parsed.size, Some(42));
        assert_eq!(parsed.path, "src/main.phpx");
    }
}
