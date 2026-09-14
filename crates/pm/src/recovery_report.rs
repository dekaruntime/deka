//! Failure-time reporting for install-transaction recovery backups
//! (deka#1012, deka#979).
//!
//! `InstallTransaction::begin` (in `install.rs`) snapshots `deka.lock`,
//! `deka.json` and `deka.grants.json` into `.cache` before mutating them, and
//! a normal rollback (`recover_install_transaction`) restores every one of
//! those backups and removes them -- so on a successful rollback there is
//! nothing left to report. This module exists for the rarer case where the
//! rollback itself fails partway through: whatever backups the *current*
//! journal still references are the reader's only path back to a known-good
//! state, so the failure message names them explicitly.
//!
//! Deliberately reads the journal file's JSON shape directly instead of
//! importing `install::InstallJournal` (which stays private to install.rs):
//! this module is a read-only consumer of the on-disk contract, not another
//! owner of the Rust type.

use serde::Deserialize;
use std::path::{Path, PathBuf};

const JOURNAL_FILE: &str = ".deka-install-transaction.json";

#[derive(Deserialize)]
struct JournalBackups {
    lock_backup: Option<PathBuf>,
    #[serde(default)]
    manifest: Option<(PathBuf, Option<PathBuf>)>,
    #[serde(default)]
    grant: GrantBackupField,
}

#[derive(Deserialize, Default)]
struct GrantBackupField {
    #[serde(default)]
    backup: Option<PathBuf>,
}

/// Read the current install-transaction journal, if one exists, and list the
/// recovery backups it references together with what each is a backup of.
/// Must be called *before* `recover_install_transaction` runs: recovery
/// consumes these backups (renaming them back into place) and deletes the
/// journal itself, so calling this after the fact would always see nothing.
/// Scoping the report to this one journal -- rather than scanning `.cache`
/// for anything matching a backup-ish name -- is what keeps a failure report
/// from misattributing unrelated stale files to the operation that just
/// failed (deka#1012 QA finding 5).
pub(crate) fn journal_backup_paths(project_dir: &Path) -> Vec<(PathBuf, &'static str)> {
    let journal_path = project_dir.join(JOURNAL_FILE);
    let Ok(bytes) = std::fs::read(&journal_path) else {
        return Vec::new();
    };
    let Ok(journal) = serde_json::from_slice::<JournalBackups>(&bytes) else {
        return Vec::new();
    };

    let mut backups = Vec::new();
    if let Some(backup) = journal.lock_backup {
        backups.push((backup, "previous deka.lock"));
    }
    if let Some((_, Some(backup))) = journal.manifest {
        backups.push((backup, "previous deka.json"));
    }
    if let Some(backup) = journal.grant.backup {
        backups.push((backup, "previous deka.grants.json"));
    }
    backups
}

/// Format only the backups from [`journal_backup_paths`] that are still on
/// disk -- a failed rollback may have restored some of them before the step
/// that actually failed.
pub(crate) fn format_backups(backups: &[(PathBuf, &'static str)]) -> String {
    let lines: Vec<String> = backups
        .iter()
        .filter(|(path, _)| path.exists())
        .map(|(path, purpose)| format!("  - {} (recovery backup of {})", path.display(), purpose))
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "\nA recovery backup was left behind and can be used to restore prior state:\n{}",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A journal referencing all three backup kinds -- lock, manifest, and
    /// grant table -- is parsed into all three (deka#1012 QA finding 4: the
    /// original blind `.cache` scan never matched `deka.grants.json-backup-*`
    /// at all).
    #[test]
    fn journal_backup_paths_covers_lock_manifest_and_grant_backups() {
        let tmp = tempfile::tempdir().expect("tmp");
        let lock_backup = tmp.path().join(".deka.lock-backup-1-1");
        let manifest_backup = tmp.path().join(".deka.json-backup-1-2");
        let grant_backup = tmp.path().join(".deka.grants.json-backup-1-3");
        for path in [&lock_backup, &manifest_backup, &grant_backup] {
            fs::write(path, b"snapshot").expect("write backup");
        }
        let journal = serde_json::json!({
            "lock_path": tmp.path().join("deka.lock"),
            "lock_backup": lock_backup,
            "grant": { "path": tmp.path().join("deka.grants.json"), "backup": grant_backup },
            "manifest": [tmp.path().join("deka.json"), manifest_backup],
            "packages": [],
        });
        fs::write(
            tmp.path().join(JOURNAL_FILE),
            serde_json::to_vec(&journal).expect("serialize journal"),
        )
        .expect("write journal");

        let mut backups = journal_backup_paths(tmp.path());
        backups.sort();
        let mut expected = vec![
            (lock_backup, "previous deka.lock"),
            (manifest_backup, "previous deka.json"),
            (grant_backup, "previous deka.grants.json"),
        ];
        expected.sort();
        assert_eq!(backups, expected);
    }

    /// No journal file (the common case: nothing failed, or the failure was
    /// before any transaction began) reports nothing -- never falls back to
    /// scanning `.cache` for anything backup-shaped.
    #[test]
    fn journal_backup_paths_is_empty_without_a_journal() {
        let tmp = tempfile::tempdir().expect("tmp");
        assert!(journal_backup_paths(tmp.path()).is_empty());
    }

    /// `format_backups` only names backups that are still on disk, and
    /// includes the filename plus what each one is a backup of.
    #[test]
    fn format_backups_lists_only_existing_files_with_their_purpose() {
        let tmp = tempfile::tempdir().expect("tmp");
        let present = tmp.path().join(".deka.lock-backup-1-1");
        fs::write(&present, b"snapshot").expect("write backup");
        let already_restored = tmp.path().join(".deka.json-backup-1-2");

        let report = format_backups(&[
            (present.clone(), "previous deka.lock"),
            (already_restored, "previous deka.json"),
        ]);

        assert!(report.contains(&present.display().to_string()));
        assert!(report.contains("previous deka.lock"));
        assert!(!report.contains("previous deka.json"));
    }

    /// Nothing to report formats to an empty string, so a fully successful
    /// rollback appends nothing to the plain error.
    #[test]
    fn format_backups_is_empty_string_when_nothing_survived() {
        assert_eq!(format_backups(&[]), "");
    }
}
