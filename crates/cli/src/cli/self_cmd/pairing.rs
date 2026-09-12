//! Binary pairing for `deka self test` (deka#836).
//!
//! The content runners (dekaruntime/testsuite `corpus/run.mjs`,
//! dekaruntime/tour `tests/tour/run.mjs) locate the toolchain the same way
//! they do inside their own repos: an executable at
//! `<checkout>/target/release/cli`, optionally beside a `dsc`. `self test`
//! materializes that layout from the running CLI (or the `--deka`/`--dsc`
//! overrides) instead of exporting `DEKA_NATIVE`/`DSC`, because the `self`
//! surface reads and writes no environment variables (deka#801, deka#836).
//!
//! Two pairing shapes, one per runner contract:
//!
//! - The corpus runner execs only `cli run`, and `deka run` finds dsc via
//!   [`compiler::dsc::find_dsc`] — the `dsc` file beside the executable
//!   it is running. A `dsc` symlink is enough.
//! - The tour runner execs the `dsc` file *directly* (it prefers it over
//!   `cli`) and uses that same binary for `deka add io` on a cold package
//!   cache. A bare dsc symlink would send `add` to a compiler that has no
//!   package manager, so the paired `dsc` is a thin wrapper: package
//!   commands forward to the paired deka CLI, everything else execs the
//!   paired dsc.
//!
//! The deka CLI is always a real file copy (never a symlink): the compiler
//! lookup keys off the executable's own path, and a symlink's resolution is
//! platform-dependent. The copy is skipped when an up-to-date one exists.

use std::fs;
use std::path::{Path, PathBuf};

/// Resolve a `--deka <path>` / `--dsc <path>` argument to a binary: a path
/// to a file is the binary itself; a directory names a checkout whose
/// `target/release/<tool>` build is used (release-only build policy).
pub fn resolve_tool(arg: &str, tool: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(arg);
    let candidate = if path.is_dir() {
        path.join("target").join("release").join(tool)
    } else {
        path
    };
    if candidate.is_file() {
        Ok(candidate)
    } else {
        Err(format!(
            "--{} points at {}, but no {} build was found there \
             (expected a binary or a checkout with target/release/{})",
            tool,
            arg,
            tool,
            tool
        ))
    }
}

/// The paired-toolchain layout materialized inside a content checkout.
pub struct Pairing {
    /// The checkout's `target/release/cli` — always a real file.
    pub cli: PathBuf,
}

/// Materialize `<checkout>/target/release/cli` (a copy of `deka_bin`) and,
/// when a dsc is resolved, `<checkout>/target/release/dsc` (the wrapper, or
/// nothing when no dsc is available and the runner's own fallback — the
/// copied cli — is enough).
pub fn materialize(
    checkout: &Path,
    deka_bin: &Path,
    dsc_bin: Option<&Path>,
) -> Result<Pairing, String> {
    let bin_dir = checkout.join("target").join("release");
    fs::create_dir_all(&bin_dir)
        .map_err(|err| format!("failed to create {}: {}", bin_dir.display(), err))?;

    let cli = bin_dir.join("cli");
    copy_fresh(deka_bin, &cli)?;

    let dsc = bin_dir.join("dsc");
    match dsc_bin {
        Some(dsc_bin) => write_dsc_wrapper(&dsc, &cli, dsc_bin)?,
        None => {
            // No explicit dsc: the runner falls back to the copied cli, and
            // that cli finds its own dsc (sibling of the original binary or
            // PATH). A stale wrapper from a previous --dsc run must not
            // shadow that.
            if dsc.exists() {
                fs::remove_file(&dsc)
                    .map_err(|err| format!("failed to remove stale {}: {}", dsc.display(), err))?;
            }
        }
    }
    Ok(Pairing { cli })
}

/// Copy `src` over `dest` unless an up-to-date copy already exists
/// (same size, not older than its source). The destination is forced
/// executable.
fn copy_fresh(src: &Path, dest: &Path) -> Result<(), String> {
    let fresh = match (fs::metadata(src), fs::metadata(dest)) {
        (Ok(src_meta), Ok(dest_meta)) => {
            dest_meta.len() == src_meta.len()
                && dest_meta.modified().ok().zip(src_meta.modified().ok()).is_some_and(
                    |(dest_mtime, src_mtime)| dest_mtime >= src_mtime,
                )
        }
        _ => false,
    };
    if !fresh {
        fs::copy(src, dest)
            .map_err(|err| format!("failed to copy {}: {}", src.display(), err))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dest)
            .map_err(|err| format!("failed to stat {}: {}", dest.display(), err))?
            .permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(dest, perms)
            .map_err(|err| format!("failed to chmod {}: {}", dest.display(), err))?;
    }
    Ok(())
}

/// Write the paired `dsc`: package management forwards to the paired deka
/// CLI (which owns the package manager), everything else execs the paired
/// dsc compiler. See the module docs for why the tour runner needs this.
fn write_dsc_wrapper(wrapper: &Path, cli: &Path, dsc_bin: &Path) -> Result<(), String> {
    let script = format!(
        "#!/bin/sh\n\
         # Paired by `deka self test` (deka#836): package commands forward to\n\
         # the paired deka CLI; everything else runs the paired dsc compiler.\n\
         case \"$1\" in\n\
         \x20 add|install) exec '{}' \"$@\" ;;\n\
         esac\n\
         exec '{}' \"$@\"\n",
        cli.display(),
        dsc_bin.display()
    );
    fs::write(wrapper, script)
        .map_err(|err| format!("failed to write {}: {}", wrapper.display(), err))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(wrapper)
            .map_err(|err| format!("failed to stat {}: {}", wrapper.display(), err))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(wrapper, perms)
            .map_err(|err| format!("failed to chmod {}: {}", wrapper.display(), err))?;
    }
    Ok(())
}

/// Find a `bun` executable. Probed in the same places run.sh looks, so a
/// non-login shell (no PATH bun) still works.
pub fn find_bun() -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = vec!["bun".into()];
    if let Some(home) = std::env::home_dir() {
        candidates.push(home.join(".bun").join("bin").join("bun"));
    }
    candidates.push("/usr/local/bin/bun".into());
    candidates.push("/opt/homebrew/bin/bun".into());
    for candidate in candidates {
        if probe(&candidate) {
            return Ok(candidate);
        }
    }
    Err("bun is required to run the content suite (install from https://bun.sh)".to_string())
}

fn probe(candidate: &Path) -> bool {
    use std::process::{Command, Stdio};
    Command::new(candidate)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(path: &Path) {
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    fn resolve_tool_accepts_files_and_checkouts() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("dsc");
        fs::write(&bin, "#!/bin/sh\n").unwrap();
        assert_eq!(resolve_tool(bin.to_str().unwrap(), "dsc").unwrap(), bin);

        let checkout = tempfile::tempdir().unwrap();
        let built = checkout.path().join("target/release/cli");
        fs::create_dir_all(built.parent().unwrap()).unwrap();
        fs::write(&built, "#!/bin/sh\n").unwrap();
        assert_eq!(
            resolve_tool(checkout.path().to_str().unwrap(), "cli").unwrap(),
            built
        );

        let missing = tempfile::tempdir().unwrap();
        assert!(resolve_tool(missing.path().to_str().unwrap(), "cli").is_err());
    }

    #[test]
    fn materialize_copies_cli_and_writes_dsc_wrapper() {
        let checkout = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let deka = src.path().join("deka");
        fs::write(&deka, "deka-bytes").unwrap();

        let dsc_src = src.path().join("dsc-real");
        fs::write(&dsc_src, "dsc-bytes").unwrap();

        let pairing = materialize(checkout.path(), &deka, Some(&dsc_src)).unwrap();
        assert_eq!(fs::read(&pairing.cli).unwrap(), b"deka-bytes");
        executable(&pairing.cli);
        let wrapper = checkout.path().join("target/release/dsc");
        let text = fs::read_to_string(&wrapper).unwrap();
        assert!(text.contains("add|install"), "wrapper forwards add: {}", text);
        assert!(text.contains(dsc_src.to_str().unwrap()));
        assert!(text.contains(pairing.cli.to_str().unwrap()));

        // Second run with no dsc removes the stale wrapper, keeps the copy.
        materialize(checkout.path(), &deka, None).unwrap();
        assert!(!wrapper.exists());
        assert_eq!(fs::read(&pairing.cli).unwrap(), b"deka-bytes");
    }

    #[test]
    fn copy_fresh_updates_stale_copies() {
        let src = tempfile::tempdir().unwrap();
        let deka = src.path().join("deka");
        fs::write(&deka, "v1").unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let dest = dest_dir.path().join("cli");

        copy_fresh(&deka, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"v1");
        // Up-to-date copy is not rewritten.
        let before = fs::metadata(&dest).unwrap().modified().unwrap();
        copy_fresh(&deka, &dest).unwrap();
        assert_eq!(fs::metadata(&dest).unwrap().modified().unwrap(), before);
        // Changed source is recopied.
        fs::write(&deka, "v2-longer").unwrap();
        copy_fresh(&deka, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"v2-longer");
    }
}
