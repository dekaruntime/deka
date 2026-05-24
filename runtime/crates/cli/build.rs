use std::process::Command;

fn main() {
    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()),
    );
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest_dir.clone());
    let git_dir = resolve_git_dir(&repo_root);
    let head_path = git_dir.join("HEAD");

    println!("cargo:rerun-if-changed={}", head_path.display());
    if let Ok(head) = std::fs::read_to_string(&head_path)
        && let Some(reference) = head.strip_prefix("ref: ").map(str::trim)
    {
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join(reference).display()
        );
    }

    let git_sha = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let build_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());

    println!("cargo:rustc-env=DEKA_GIT_SHA={}", git_sha);
    println!("cargo:rustc-env=DEKA_BUILD_UNIX={}", build_unix);
    println!("cargo:rustc-env=DEKA_TARGET={}", target);
    println!("cargo:rustc-env=DEKA_RUNTIME_ABI=deka-runtime-phpx-v1");
}

fn resolve_git_dir(repo_root: &std::path::Path) -> std::path::PathBuf {
    let dot_git = repo_root.join(".git");
    if dot_git.is_dir() {
        return dot_git;
    }
    if let Ok(contents) = std::fs::read_to_string(&dot_git)
        && let Some(path) = contents.strip_prefix("gitdir: ").map(str::trim)
    {
        let resolved = std::path::Path::new(path);
        if resolved.is_absolute() {
            return resolved.to_path_buf();
        }
        return repo_root.join(resolved);
    }
    dot_git
}
