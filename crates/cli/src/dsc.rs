//! Exec the dsc compiler when it is shipped beside deka (rfd#38).
//!
//! Lookup order:
//! 1. `DEKA_DSC` — explicit path. If set and missing, fail closed.
//! 2. `dsc` next to this `deka` binary.
//! 3. `dsc` on `PATH`.
//!
//! `check` / `fmt` / `transpile` / `lsp` require dsc. `DEKA_NO_DSC=1` forces
//! the in-process crates until those crates leave this repo.

use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn find_dsc() -> Result<Option<PathBuf>, String> {
    runtime_core::dsc::find_dsc()
}

/// Exec dsc with the same argv tail. Does not return on success.
///
/// `DEKA_NO_DSC=1` skips exec so the in-process handler runs. Otherwise a
/// missing dsc is a hard error — silent fallback hid the split.
pub fn exec_if_present() {
    if env::var_os("DEKA_NO_DSC").is_some() {
        return;
    }
    match find_dsc() {
        Ok(None) => {
            stdio::error(
                "cli",
                "dsc is required for check, fmt, transpile, and lsp. Install dsc (https://deka.gg/install), set DEKA_DSC, or put dsc next to deka / on PATH. DEKA_NO_DSC=1 uses the in-process compiler until those crates are removed.",
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
            stdio::error(
                "cli",
                &format!("failed to exec {}: {err}", bin.display()),
            );
            std::process::exit(1);
        }
    }
}
