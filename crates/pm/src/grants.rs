//! RFD 27 grant-table delivery (deka#797).
//!
//! `deka add` / `deka install` derive the host grants a locked `@deka/*`
//! release unlocks and persist them in the project's grant table
//! (`deka.grants.json`, next to `deka.lock`). The runtime ESM loader reads
//! that file after the explicit override channels (`PoolConfig.host_grants`,
//! then the `DEKA_HOST_GRANTS` env var) so a freshly installed stdlib
//! package can call its bridge kinds with no programmatic config and no
//! environment variable.
//!
//! Trust root, in both directions:
//!
//! - Kinds come from the authoritative runtime catalog
//!   (`permissions::host_bridge::HOST_CATALOG`): every kind whose grant
//!   owner is exactly the package identity. A package can never obtain a
//!   kind the published catalog does not assign to its identity — never
//!   from its own manifest asking for it (a dependency `host.kinds` field is
//!   untrusted and ignored).
//! - The lookup key is the lockfile-pinned `fsGraph` digest of the installed
//!   tree, computed and verified by the installer. A grant recorded for one
//!   digest unlocks nothing else: a tampered lockfile digest, or any install
//!   whose bytes fail integrity verification, ends up with no matching grant.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use permissions::host_bridge::{GrantTable, HostGrant};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::lock::LockEntry;

/// Project grant-table file, written next to `deka.lock`. Same JSON schema as
/// `GrantTable` / the `DEKA_HOST_GRANTS` override: an array of
/// `{ name, version, digest, kinds }` records keyed by the fsGraph digest.
pub const GRANT_TABLE_FILE: &str = "deka.grants.json";

pub fn grant_table_path(project_dir: &Path) -> PathBuf {
    project_dir.join(GRANT_TABLE_FILE)
}

/// Bridge kinds the authoritative catalog grants to an official package
/// identity: every catalog kind whose grant owner is exactly `name`
/// (`@deka/fs` → `["fs"]`, `@deka/crypto` → `["crypto"]`, ...). Packages the
/// catalog assigns no kinds to (`@deka/json`, `@deka/http`, ...) get none.
pub fn catalog_kinds_for_package(name: &str) -> Vec<String> {
    permissions::host_bridge::HOST_CATALOG
        .iter()
        .filter(|kind| kind.grant_owner == name)
        .map(|kind| kind.name.to_string())
        .collect()
}

/// Derive the grant table for exactly the installed graph: one record per
/// installed package the catalog assigns kinds to, keyed by the lockfile
/// metadata's `fsGraph` hash. Packages with no catalog kinds or no pinned
/// digest contribute no record.
pub fn derive_grant_table(installed: &std::collections::BTreeMap<String, LockEntry>) -> GrantTable {
    let mut grants = Vec::new();
    for (name, (descriptor, _, metadata, _)) in installed {
        let kinds = catalog_kinds_for_package(name);
        if kinds.is_empty() {
            continue;
        }
        let Some(digest) = fs_graph_hash(metadata) else {
            continue;
        };
        let version = descriptor
            .strip_prefix(&format!("{name}@"))
            .filter(|version| !version.is_empty())
            .unwrap_or(descriptor)
            .to_string();
        grants.push(HostGrant {
            name: name.clone(),
            version,
            digest,
            kinds,
        });
    }
    GrantTable { grants }
}

fn fs_graph_hash(metadata: &Value) -> Option<String> {
    metadata
        .get("fsGraph")
        .and_then(|graph| graph.get("hash"))
        .and_then(Value::as_str)
        .filter(|hash| !hash.is_empty())
        .map(ToString::to_string)
}

/// What the installer should do with `deka.grants.json` for a given derived
/// table. The canonical on-disk state is: file present with exactly the
/// derived records when there are any, file absent when there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantTablePlan {
    /// Write (or replace) the file with this table.
    Write(GrantTable),
    /// Remove a stale file: the derived table is empty.
    Remove,
    /// On-disk state already matches; leave it untouched.
    Unchanged,
}

/// Compare the derived table against what is on disk and decide the write.
/// A missing or malformed existing file is treated as "not matching" and is
/// rewritten — the file is install-managed, and the lockfile-verified
/// derivation is the only authority.
pub fn plan_grant_table_write(project_dir: &Path, derived: &GrantTable) -> Result<GrantTablePlan> {
    let path = grant_table_path(project_dir);
    let existing = read_grant_table_at(&path);
    match (derived.grants.is_empty(), existing) {
        (true, None) => Ok(GrantTablePlan::Unchanged),
        (true, Some(_)) => Ok(GrantTablePlan::Remove),
        (false, Some(table)) if &table == derived => Ok(GrantTablePlan::Unchanged),
        (false, _) => Ok(GrantTablePlan::Write(derived.clone())),
    }
}

/// Execute a [`GrantTablePlan`]: atomic temp-file + rename write, or removal.
pub fn apply_grant_table_plan(project_dir: &Path, plan: &GrantTablePlan) -> Result<()> {
    let path = grant_table_path(project_dir);
    match plan {
        GrantTablePlan::Unchanged => {}
        GrantTablePlan::Remove => {
            if path.exists() {
                fs::remove_file(&path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
            }
        }
        GrantTablePlan::Write(table) => write_grant_table_at(&path, table)?,
    }
    Ok(())
}

/// Read the project grant table. Absent or malformed yields `None` (i.e. no
/// grants — the same failure mode as a missing table entry).
pub fn read_grant_table_at(path: &Path) -> Option<GrantTable> {
    let text = fs::read_to_string(path).ok()?;
    GrantTable::from_json(&text).ok()
}

/// Write the grant table atomically (temp file in the same directory +
/// rename), matching the lockfile's durability pattern so a concurrent
/// reader never observes a truncated table.
pub fn write_grant_table_at(path: &Path, table: &GrantTable) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("grant table path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.tmp-{}-{}",
        GRANT_TABLE_FILE,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    let mut file = File::create(&temp)?;
    serde_json::to_writer_pretty(&mut file, table)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// --locked freshness check for the grant table (deka#797): the on-disk
/// `deka.grants.json` must already equal what this install would write,
/// with the same strictness `lock_diff` applies to `deka.lock`.
pub fn ensure_locked_grant_plan(locked: bool, plan: &GrantTablePlan) -> Result<()> {
    if locked && !matches!(plan, GrantTablePlan::Unchanged) {
        bail!(
            "--locked install would change {} (run a normal install to refresh the grant table)",
            GRANT_TABLE_FILE
        );
    }
    Ok(())
}

/// Install-transaction entry point (deka#797): plan the grant table write for
/// the installed graph, enforce the `--locked` freshness check, and apply.
/// The table is rewritten with the lockfile (same exact-graph semantics:
/// stale grants for removed packages are dropped), inside the install
/// transaction so an interrupted install restores the snapshot.
pub fn deliver_grant_table(
    project_dir: &Path,
    installed: &std::collections::BTreeMap<String, LockEntry>,
    locked: bool,
) -> Result<()> {
    let plan = plan_grant_table_write(project_dir, &derive_grant_table(installed))?;
    ensure_locked_grant_plan(locked, &plan)?;
    apply_grant_table_plan(project_dir, &plan)
}

/// Re-derive and persist the grant table for the given installed graph
/// (plan + apply in one step). Used by install refresh paths that have no
/// `--locked` gate, such as rehash: re-key the table with the recomputed
/// digests so a rehashed package keeps (only) the grants its refreshed lock
/// entry pins.
pub fn rewrite_grant_table(
    project_dir: &Path,
    installed: &std::collections::BTreeMap<String, LockEntry>,
) -> Result<GrantTablePlan> {
    let plan = plan_grant_table_write(project_dir, &derive_grant_table(installed))?;
    apply_grant_table_plan(project_dir, &plan)?;
    Ok(plan)
}

/// Journal snapshot of the project grant table, taken when the install
/// transaction begins and restored on recovery with the same atomic-swap
/// discipline as the lockfile (deka#797). Deserializes empty from journals
/// written before grant plumbing, in which case recovery is a no-op.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GrantTableSnapshot {
    #[serde(default)]
    path: PathBuf,
    #[serde(default)]
    backup: Option<PathBuf>,
}

impl GrantTableSnapshot {
    pub fn snapshot(project_dir: &Path) -> Result<Self> {
        let (path, backup) = crate::lock::snapshot_file(project_dir.join(GRANT_TABLE_FILE))?;
        Ok(Self { path, backup })
    }

    /// Recovery: restore the backup when one was taken; otherwise remove the
    /// table a pre-crash install may have created.
    pub fn restore(&self) -> Result<()> {
        if let Some(backup) = &self.backup {
            if backup.exists() {
                if self.path.exists() {
                    fs::remove_file(&self.path)?;
                }
                fs::rename(backup, &self.path)?;
                if let Some(parent) = self.path.parent() {
                    crate::lock::sync_directory(parent)?;
                }
            }
        } else if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        Ok(())
    }

    /// Commit: drop the backup; the live table is the new state.
    pub fn discard(&self) {
        if let Some(backup) = &self.backup {
            let _ = fs::remove_file(backup);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn installed_fixture() -> std::collections::BTreeMap<String, LockEntry> {
        let mut installed = std::collections::BTreeMap::new();
        installed.insert(
            "@deka/fs".to_string(),
            (
                "@deka/fs@9.9.9-fixture".to_string(),
                "deka.gg:@deka/fs".to_string(),
                json!({
                    "fsGraph": { "algo": "sha256", "hash": "sha256:fsdigest" },
                    "moduleGraph": { "algo": "sha256", "hash": "sha256:mod" }
                }),
                String::new(),
            ),
        );
        installed.insert(
            "@deka/json".to_string(),
            (
                "@deka/json@1.0.0".to_string(),
                "deka.gg:@deka/json".to_string(),
                json!({ "fsGraph": { "algo": "sha256", "hash": "sha256:jsondigest" } }),
                String::new(),
            ),
        );
        installed
    }

    #[test]
    fn catalog_kinds_follow_the_grant_owner_binding() {
        assert_eq!(catalog_kinds_for_package("@deka/fs"), vec!["fs"]);
        assert_eq!(catalog_kinds_for_package("@deka/crypto"), vec!["crypto"]);
        // tls owns its own kind; tcp owns net.
        assert_eq!(catalog_kinds_for_package("@deka/tcp"), vec!["net"]);
        assert_eq!(catalog_kinds_for_package("@deka/tls"), vec!["tls"]);
        // Packages the catalog assigns no host kinds get none.
        assert!(catalog_kinds_for_package("@deka/json").is_empty());
        assert!(catalog_kinds_for_package("@user/evil").is_empty());
    }

    #[test]
    fn derive_grant_table_keys_records_by_lockfile_fsgraph_digest() {
        let table = derive_grant_table(&installed_fixture());
        assert_eq!(table.grants.len(), 1, "json package derives no grant");
        let grant = &table.grants[0];
        assert_eq!(grant.name, "@deka/fs");
        assert_eq!(grant.version, "9.9.9-fixture");
        assert_eq!(grant.digest, "sha256:fsdigest");
        assert_eq!(grant.kinds, vec!["fs"]);
    }

    #[test]
    fn derive_grant_table_skips_entries_without_fsgraph_digest() {
        let mut installed = installed_fixture();
        let (_, _, metadata, _) = installed.get_mut("@deka/fs").expect("fs entry");
        *metadata = json!({ "moduleGraph": { "algo": "sha256", "hash": "sha256:mod" } });
        assert!(derive_grant_table(&installed).grants.is_empty());
    }

    #[test]
    fn grant_table_plan_write_remove_unchanged() {
        let tmp = tempfile::tempdir().expect("tmp");
        let derived = derive_grant_table(&installed_fixture());

        // No file, non-empty table → write.
        let plan = plan_grant_table_write(tmp.path(), &derived).expect("plan");
        assert!(matches!(plan, GrantTablePlan::Write(_)));
        apply_grant_table_plan(tmp.path(), &plan).expect("apply");

        // Identical table on disk → unchanged.
        let plan = plan_grant_table_write(tmp.path(), &derived).expect("plan");
        assert_eq!(plan, GrantTablePlan::Unchanged);

        // Different table → rewrite.
        let mut changed = derived.clone();
        changed.grants[0].digest = "sha256:other".to_string();
        let plan = plan_grant_table_write(tmp.path(), &changed).expect("plan");
        assert!(matches!(plan, GrantTablePlan::Write(_)));

        // Empty table with a file on disk → remove.
        let empty = GrantTable::default();
        let plan = plan_grant_table_write(tmp.path(), &empty).expect("plan");
        assert_eq!(plan, GrantTablePlan::Remove);
        apply_grant_table_plan(tmp.path(), &plan).expect("apply");
        assert!(!grant_table_path(tmp.path()).exists());

        // Empty table, no file → nothing to do.
        let plan = plan_grant_table_write(tmp.path(), &empty).expect("plan");
        assert_eq!(plan, GrantTablePlan::Unchanged);
    }

    #[test]
    fn write_grant_table_is_atomic_and_roundtrips() {
        let tmp = tempfile::tempdir().expect("tmp");
        let path = grant_table_path(tmp.path());
        let table = derive_grant_table(&installed_fixture());
        write_grant_table_at(&path, &table).expect("write");
        assert_eq!(read_grant_table_at(&path).expect("read"), table);
        // No temp files left behind.
        let leftovers: Vec<_> = fs::read_dir(tmp.path())
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    mod install_plumbing {
        use super::super::*;
        use crate::install::{
            InstallTransaction, recover_install_transaction, rehash_php_packages_in,
        };
        use crate::payload::InstallPayload;
        use deka_host::integrity::compute_package_integrity;
        use runtime_core::modules::MODULES_DIR;
        use serde_json::json;
        use std::collections::BTreeMap;
        use std::fs;

        #[test]
        fn locked_install_rejects_any_grant_table_drift() {
            let derived = derive_grant_table(&BTreeMap::from([(
                "@deka/fs".to_string(),
                (
                    "@deka/fs@1.0.0".to_string(),
                    "deka.gg:@deka/fs".to_string(),
                    json!({ "fsGraph": { "algo": "sha256", "hash": "sha256:abc" } }),
                    String::new(),
                ),
            )]));
            // Unchanged plan passes under --locked...
            ensure_locked_grant_plan(true, &GrantTablePlan::Unchanged).expect("unchanged");
            // ...any write or removal is a lockfile-style freshness failure.
            for plan in [
                GrantTablePlan::Write(derived.clone()),
                GrantTablePlan::Remove,
            ] {
                let err = ensure_locked_grant_plan(true, &plan).expect_err("must bail");
                assert!(
                    err.to_string()
                        .contains("--locked install would change deka.grants.json"),
                    "unexpected error: {err}"
                );
            }
            // Non-locked installs never fail the check.
            ensure_locked_grant_plan(false, &GrantTablePlan::Remove).expect("not locked");
        }

        #[test]
        fn interrupted_transaction_restores_grant_table_snapshot() {
            let tmp = tempfile::tempdir().expect("tmp");
            let grant_path = tmp.path().join(GRANT_TABLE_FILE);
            let old_grants = "[{\"name\":\"@deka/fs\",\"version\":\"1.0.0\",\"digest\":\"sha256:old\",\"kinds\":[\"fs\"]}]\n";
            fs::write(&grant_path, old_grants).expect("old grant table");

            let transaction = InstallTransaction::begin(tmp.path(), &tmp.path().join("deka.lock"))
                .expect("begin");
            // The install overwrites the table, then "crashes" before finish.
            fs::write(
                &grant_path,
                "[{\"name\":\"@deka/fs\",\"version\":\"2.0.0\",\"digest\":\"sha256:new\",\"kinds\":[\"fs\"]}]\n",
            )
            .expect("new grant table");
            drop(transaction);

            recover_install_transaction(tmp.path()).expect("recover");
            assert_eq!(
                fs::read_to_string(&grant_path).expect("restored grant table"),
                old_grants
            );
        }

        #[test]
        fn interrupted_transaction_removes_grant_table_created_mid_install() {
            let tmp = tempfile::tempdir().expect("tmp");
            let transaction = InstallTransaction::begin(tmp.path(), &tmp.path().join("deka.lock"))
                .expect("begin");
            // No grant table existed at begin(); the crashed install created one.
            fs::write(
                tmp.path().join(GRANT_TABLE_FILE),
                "[{\"name\":\"@deka/fs\",\"version\":\"1.0.0\",\"digest\":\"sha256:new\",\"kinds\":[\"fs\"]}]\n",
            )
            .expect("created grant table");
            drop(transaction);

            recover_install_transaction(tmp.path()).expect("recover");
            assert!(
                !tmp.path().join(GRANT_TABLE_FILE).exists(),
                "recovery must remove a grant table the crashed install created"
            );
        }

        #[tokio::test]
        async fn rehash_rekeys_grant_table_with_recomputed_digest() {
            let tmp = tempfile::tempdir().expect("tmp");
            let package_root = tmp.path().join(MODULES_DIR).join("@deka").join("fs");
            fs::create_dir_all(&package_root).expect("mkdir package");
            fs::write(
                package_root.join("index.ds"),
                "export const fixture = true;\n",
            )
            .expect("module");
            fs::write(
                tmp.path().join("deka.lock"),
                json!({
                    "lockfileVersion": 1,
                    "packages": {
                        "@deka/fs": [
                            "@deka/fs@1.0.0",
                            "deka.gg:@deka/fs",
                            {
                                "moduleGraph": { "algo": "sha256", "hash": "stale-module" },
                                "fsGraph": { "algo": "sha256", "hash": "stale-fs" }
                            },
                            ""
                        ]
                    }
                })
                .to_string(),
            )
            .expect("write lock");
            // Stale grant keyed by the stale digest, as written before the tree
            // was edited on disk.
            fs::write(
                tmp.path().join(GRANT_TABLE_FILE),
                json!([{
                    "name": "@deka/fs",
                    "version": "1.0.0",
                    "digest": "stale-fs",
                    "kinds": ["fs"]
                }])
                .to_string(),
            )
            .expect("write stale grant table");

            let payload = InstallPayload {
                specs: Vec::new(),
                yes: true,
                prompt: false,
                quiet: true,
                rehash: true,
                locked: false,
            };
            rehash_php_packages_in(&payload, tmp.path())
                .await
                .expect("rehash");

            let integrity = compute_package_integrity(&package_root).expect("integrity");
            let table = read_grant_table_at(&tmp.path().join(GRANT_TABLE_FILE))
                .expect("re-keyed grant table");
            assert_eq!(table.grants.len(), 1);
            assert_eq!(table.grants[0].name, "@deka/fs");
            assert_eq!(table.grants[0].digest, integrity.fs_graph);
            assert_eq!(table.grants[0].kinds, vec!["fs"]);
        }
    }
}
