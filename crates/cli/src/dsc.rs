//! Exec the dsc compiler when it is shipped beside deka (rfd#38).
//!
//! Lookup order:
//! 1. `DEKA_DSC` — explicit path. If set and missing, fail closed.
//! 2. `dsc` next to this `deka` binary.
//! 3. In-process compiler crates (fallback until the crates leave this repo).
//!
//! `DEKA_NO_DSC=1` forces the in-process fallback (tests, emergency).

use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn find_dsc() -> Result<Option<PathBuf>, String> {
    runtime_core::dsc::find_dsc()
}

/// If dsc is available, exec it with the same argv tail and do not return.
pub fn exec_if_present() {
    match find_dsc() {
        Ok(None) => {}
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
