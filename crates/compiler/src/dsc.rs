//! Locate the dsc compiler binary (rfd#38).
//!
//! The compiler is a release artifact shipped beside `deka`. Test callers use
//! the repository-pinned release artifact. A compiler path is never selected
//! from the process environment.
//!
//! CLI commands additionally honor `DEKA_DSC` / `DEKA_NO_DSC` / `PATH` through
//! [`find_cli_dsc`]. Runtime and pool keep using [`find_dsc`].

use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn find_dsc() -> Result<Option<PathBuf>, String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("dsc");
            if sibling.is_file() {
                return Ok(Some(sibling));
            }
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest.ancestors().nth(2) {
        let pinned = root.join("target/release/dsc");
        if pinned.is_file() {
            return Ok(Some(pinned));
        }
    }
    Ok(None)
}

const MISSING_DSC: &str = "dsc is required for check, fmt, transpile, and lsp. Install dsc (https://deka.gg/install), set DEKA_DSC, or put dsc next to deka / on PATH.";

/// CLI compiler lookup: `DEKA_DSC`, then [`find_dsc`], then `PATH`.
pub fn find_cli_dsc() -> Result<Option<PathBuf>, String> {
    if env::var_os("DEKA_NO_DSC").is_some() {
        return Ok(None);
    }
    if let Ok(path) = env::var("DEKA_DSC") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(Some(path));
        }
        return Err(format!(
            "DEKA_DSC is set to {} but that path is not a file",
            path.display()
        ));
    }

    if let Some(dsc) = find_dsc()? {
        return Ok(Some(dsc));
    }
    if let Ok(path_var) = env::var("PATH") {
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join("dsc");
            if candidate.is_file() {
                return Ok(Some(candidate));
            }
        }
    }
    Ok(None)
}

pub fn require_dsc() -> Result<PathBuf, String> {
    find_cli_dsc()?.ok_or_else(|| {
        "dsc is required for deka build. Install dsc (https://deka.gg/install), set DEKA_DSC, or put dsc next to deka / on PATH."
            .to_string()
    })
}

/// `dsc --version` output (first line) when the binary reports one; None
/// otherwise. Recorded as compiler provenance in the build manifest.
pub fn dsc_identity() -> Option<String> {
    let dsc = find_cli_dsc().ok().flatten()?;
    let output = Command::new(&dsc).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = if output.stdout.is_empty() {
        output.stderr
    } else {
        output.stdout
    };
    String::from_utf8_lossy(&text)
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
}

/// Resolve the CLI dsc or exit 1 with the install hint. Checking a whole
/// project resolves the compiler once up front so a missing dsc is one
/// tooling error, not one hint per source file.
pub fn require_cli_dsc() -> PathBuf {
    match find_cli_dsc() {
        Ok(Some(bin)) => bin,
        Ok(None) => {
            stdio::error("cli", MISSING_DSC);
            std::process::exit(1);
        }
        Err(err) => {
            stdio::error("cli", &err);
            std::process::exit(1);
        }
    }
}

/// `dsc check <path>` with the already-resolved `dsc` binary; failure
/// carries dsc's stderr. When `project_root` is given, dsc runs from it with
/// `DEKA_MODULE_ROOT` set — the same context `deka build` compiles project
/// sources in — so project-local and bare `@deka/*` imports resolve.
pub fn check_path(
    dsc: &std::path::Path,
    path: &std::path::Path,
    project_root: Option<&std::path::Path>,
) -> Result<(), String> {
    let mut cmd = Command::new(dsc);
    if let Some(root) = project_root {
        cmd.current_dir(root).env("DEKA_MODULE_ROOT", root);
    }
    let output = cmd
        .arg("check")
        .arg(path)
        .output()
        .map_err(|err| format!("failed to exec {}: {err}", dsc.display()))?;
    if output.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

pub fn is_deka_source_path(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ds") | Some("dsx")
    )
}

/// Sorted enumeration of every `.ds`/`.dsx` file under `dir`. Shared by
/// `deka check`'s project pass and `deka build`'s plan/emit walks.
pub fn collect_deka_source_files(dir: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Ok(files);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current)
            .map_err(|err| format!("failed to read {}: {err}", current.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("read_dir entry error: {err}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|err| format!("file_type error for {}: {err}", path.display()))?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() && is_deka_source_path(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Exec dsc with the same argv tail. Does not return on success.
pub fn exec_if_present() {
    match find_cli_dsc() {
        Ok(None) => {
            stdio::error("cli", MISSING_DSC);
            std::process::exit(1);
        }
        Ok(Some(bin)) => exec_dsc(&bin),
        Err(err) => {
            stdio::error("cli", &err);
            std::process::exit(1);
        }
    }
}

fn exec_dsc(bin: &std::path::Path) {
    let args: Vec<String> = env::args().skip(1).collect();
    let status = Command::new(bin)
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    match status {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(err) => {
            stdio::error("cli", &format!("failed to exec {}: {err}", bin.display()));
            std::process::exit(1);
        }
    }
}
