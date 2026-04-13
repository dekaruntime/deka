use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

static REPOS_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn init_repos_root(path: &str) {
    let root = PathBuf::from(path);
    REPOS_ROOT
        .set(root)
        .expect("repos root already initialized");
}

fn repos_root() -> &'static PathBuf {
    REPOS_ROOT.get().expect("repos root not initialized")
}

fn sanitize_path_segment(value: &str) -> Result<String, anyhow::Error> {
    if value.is_empty() {
        return Err(anyhow::anyhow!("empty path segment"));
    }
    if value.contains('/') || value.contains("..") || value.contains('\0') {
        return Err(anyhow::anyhow!("invalid path segment"));
    }
    Ok(value.to_string())
}

fn normalize_git_ref(reference: &str) -> Result<String, anyhow::Error> {
    let raw = reference.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("head") {
        return Ok("HEAD".to_string());
    }
    if raw.contains('\0') || raw.contains("..") || raw.contains(" ") || raw.starts_with('/') {
        return Err(anyhow::anyhow!("invalid ref"));
    }
    Ok(raw.to_string())
}

#[derive(Debug, Clone)]
pub struct ResolvedRef {
    pub requested_ref: String,
    pub normalized_ref: String,
    pub commit: String,
}

pub fn create_bare_repo(owner: &str, repo: &str) -> Result<PathBuf, anyhow::Error> {
    let owner = sanitize_path_segment(owner)?;
    let repo = sanitize_path_segment(repo)?;

    let repo_path = repos_root().join(owner).join(format!("{}.git", repo));
    if repo_path.exists() {
        return Err(anyhow::anyhow!("Repository already exists"));
    }

    fs::create_dir_all(repo_path.parent().expect("repo parent"))?;

    let output = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg(&repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("git init failed: {}", stderr));
    }

    tracing::info!("Created bare repo: {}", repo_path.display());
    Ok(repo_path)
}

pub fn list_repos(owner: &str) -> Result<Vec<String>, anyhow::Error> {
    let owner = sanitize_path_segment(owner)?;
    let user_dir = repos_root().join(owner);

    if !user_dir.exists() {
        return Ok(Vec::new());
    }

    let mut repos = Vec::new();
    for entry in fs::read_dir(user_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".git") {
                repos.push(name.trim_end_matches(".git").to_string());
            }
        }
    }

    repos.sort();
    Ok(repos)
}

pub fn resolve_ref(owner: &str, repo: &str, reference: &str) -> Result<ResolvedRef, anyhow::Error> {
    let owner = sanitize_path_segment(owner)?;
    let repo = sanitize_path_segment(repo)?;
    let requested_ref = reference.trim().to_string();
    let normalized_ref = normalize_git_ref(reference)?;
    let repo_path = repos_root().join(owner).join(format!("{}.git", repo));
    if !repo_path.exists() {
        return Err(anyhow::anyhow!("repository not found"));
    }

    let rev_target = format!("{}^{{commit}}", normalized_ref);
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(&repo_path)
        .arg("rev-parse")
        .arg("--verify")
        .arg(&rev_target)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow::anyhow!("failed to resolve ref: {}", stderr));
    }
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if commit.is_empty() {
        return Err(anyhow::anyhow!("failed to resolve ref: empty commit"));
    }

    Ok(ResolvedRef {
        requested_ref,
        normalized_ref,
        commit,
    })
}

pub fn fork_bare_repo(
    source_owner: &str,
    source_repo: &str,
    target_owner: &str,
    target_repo: &str,
) -> Result<PathBuf, anyhow::Error> {
    let source_owner = sanitize_path_segment(source_owner)?;
    let source_repo = sanitize_path_segment(source_repo)?;
    let target_owner = sanitize_path_segment(target_owner)?;
    let target_repo = sanitize_path_segment(target_repo)?;

    let source_path = repos_root()
        .join(source_owner)
        .join(format!("{}.git", source_repo));
    if !source_path.exists() {
        return Err(anyhow::anyhow!("source repository not found"));
    }

    let target_path = repos_root()
        .join(target_owner)
        .join(format!("{}.git", target_repo));
    if target_path.exists() {
        return Err(anyhow::anyhow!("target repository already exists"));
    }
    fs::create_dir_all(target_path.parent().expect("fork target parent"))?;

    let output = Command::new("git")
        .arg("clone")
        .arg("--bare")
        .arg(&source_path)
        .arg(&target_path)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("git clone failed: {}", stderr.trim()));
    }

    tracing::info!(
        "Forked bare repo: {} -> {}",
        source_path.display(),
        target_path.display()
    );
    Ok(target_path)
}

#[allow(dead_code)]
pub fn delete_repo(owner: &str, repo: &str) -> Result<(), anyhow::Error> {
    let owner = sanitize_path_segment(owner)?;
    let repo = sanitize_path_segment(repo)?;
    let repo_path = repos_root().join(owner).join(format!("{}.git", repo));

    if !repo_path.exists() {
        return Err(anyhow::anyhow!("Repository not found"));
    }

    fs::remove_dir_all(&repo_path)?;
    tracing::info!("Deleted repo: {}", repo_path.display());
    Ok(())
}

pub fn get_repo_path(owner: &str, repo: &str) -> PathBuf {
    let owner = sanitize_path_segment(owner).unwrap_or_else(|_| owner.to_string());
    let repo_name = repo.strip_suffix(".git").unwrap_or(repo);
    let repo_name = sanitize_path_segment(repo_name).unwrap_or_else(|_| repo_name.to_string());
    repos_root().join(owner).join(format!("{}.git", repo_name))
}

#[cfg(test)]
mod tests {
    use super::normalize_git_ref;

    #[test]
    fn normalize_git_ref_maps_head() {
        assert_eq!(normalize_git_ref("HEAD").unwrap(), "HEAD");
        assert_eq!(normalize_git_ref("head").unwrap(), "HEAD");
        assert_eq!(normalize_git_ref("").unwrap(), "HEAD");
    }

    #[test]
    fn normalize_git_ref_keeps_branch_names() {
        assert_eq!(normalize_git_ref("main").unwrap(), "main");
        assert_eq!(
            normalize_git_ref("refs/tags/v1.0.0").unwrap(),
            "refs/tags/v1.0.0"
        );
    }

    #[test]
    fn normalize_git_ref_rejects_invalid_values() {
        assert!(normalize_git_ref("../main").is_err());
        assert!(normalize_git_ref("main branch").is_err());
        assert!(normalize_git_ref("/root").is_err());
    }
}
