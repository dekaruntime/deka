use super::types::TreeEntry;
use std::process::Command;

pub(super) fn git_ref_exists(
    repo_path: &std::path::Path,
    git_ref: &str,
) -> Result<bool, anyhow::Error> {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repo_path.display()))
        .arg("rev-parse")
        .arg("--verify")
        .arg(git_ref)
        .output()?;

    Ok(output.status.success())
}

pub(super) fn list_files_at_ref(
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

pub(super) fn git_show_file(
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
pub(super) fn parse_ls_tree_line(line: &str) -> Option<TreeEntry> {
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
pub(super) fn git_resolve_commit(
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
