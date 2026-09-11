//! Exec the dsc compiler when it is shipped beside deka (rfd#38).
//!
//! Lookup order:
//! 1. `DEKA_DSC` — explicit path. If set and missing, fail closed.
//! 2. `dsc` next to this `deka` binary.
//! 3. `dsc` on `PATH`.
//!
//! `check` / `fmt` / `transpile` / `lsp` require dsc. There is no in-process
//! compiler fallback.

use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn find_dsc() -> Result<Option<PathBuf>, String> {
    // Compiler selection is a CLI concern. Runtime and pool receive their
    // policy explicitly, but build/check/fmt/transpile/lsp are CLI commands
    // and their documented DEKA_DSC override must reach their subprocesses.
    //
    // In particular, `deka build` is intentionally run with a real compiler
    // before artifact-only serve tests remove DEKA_DSC and arm their poisoned
    // PATH canary. Do not move this ambient lookup back into runtime_core.
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

    if let Some(dsc) = runtime_core::dsc::find_dsc()? {
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

/// Exec dsc with the same argv tail. Does not return on success.
///
/// Missing dsc is a hard error. `DEKA_NO_DSC` still hides dsc from lookup
/// (`find_dsc` returns None) and therefore errors — there is no in-process
/// compiler left in this binary.
pub fn exec_if_present() {
    match find_dsc() {
        Ok(None) => {
            stdio::error(
                "cli",
                "dsc is required for check, fmt, transpile, and lsp. Install dsc (https://deka.gg/install), set DEKA_DSC, or put dsc next to deka / on PATH.",
            );
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
