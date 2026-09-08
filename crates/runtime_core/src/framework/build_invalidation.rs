//! Targeted build-slot invalidation for `deka dev` (deka#725).
//!
//! Pure decision logic over the manifest's recorded filesystem observations:
//! a local change reruns exactly the slots whose recorded inputs it affects.
//! Everything here is exact matching — the only coarse fallback (rematerialize
//! every planned slot) is the caller's deliberate, logged choice when no
//! manifest is available; this function never silently widens.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use super::build_manifest::{BuildManifest, FsObservationKind};

/// Which build slots a set of local filesystem changes invalidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotInvalidation {
    /// Slot ids that must rematerialize, matched through recorded
    /// observations (their source files did not change, so their compiler
    /// ids are still current).
    pub slots: BTreeSet<String>,
    /// Source files (project-relative spelling, see [`project_relative_path`])
    /// whose own content changed. A source edit can shift the `build {}`
    /// block's compiler span, which changes its slot id — the manifest's
    /// ids for these files are stale, so the caller must replan the file and
    /// rematerialize every slot it NOW declares, replacing the file's
    /// manifest slots wholesale (Codex review of deka#729).
    pub source_files: BTreeSet<String>,
    /// When true the caller must rematerialize every planned slot: the
    /// manifest could not prove the change irrelevant to a slot. Only set by
    /// callers that cannot read a manifest at all; this function's exact
    /// matching always proves relevance per slot.
    pub coarse: bool,
}

impl SlotInvalidation {
    /// No slot is affected; the change cannot influence materialized values.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty() && self.source_files.is_empty() && !self.coarse
    }
}

/// Compute the slots affected by `changed` local paths.
///
/// Exact rules, per slot:
/// - the slot's own source file changed → reported in `source_files` for a
///   wholesale replan (the edit may have shifted the block's span and with it
///   the slot id, so the manifest id cannot be trusted for this file);
/// - a `read` observation whose path was edited, deleted, or replaced;
/// - an `absent` observation whose path now exists (created) or changed;
/// - a `directory_listing` observation whose directory gained, lost, or
///   renamed a direct entry (the changed path is the directory itself or a
///   direct child). Nested changes do not affect a parent's listing except
///   through the direct-child rule, so they do not invalidate.
///
/// Slots with no observations are affected only by their own source file.
pub fn affected_slots(
    manifest: &BuildManifest,
    project_root: &Path,
    changed: &[PathBuf],
) -> SlotInvalidation {
    let root = clean_path(project_root);
    let root_lexical = lexical_clean(project_root);
    let changed: Vec<String> = changed
        .iter()
        .map(|path| relativize(&root, &root_lexical, &clean_path(path)))
        .collect();
    let mut slots = BTreeSet::new();
    let mut source_files = BTreeSet::new();
    for slot in &manifest.slots {
        let slot_file = relativize(
            &root,
            &root_lexical,
            &clean_path(&PathBuf::from(&slot.file)),
        );
        if changed.iter().any(|path| *path == slot_file) {
            source_files.insert(slot_file);
            continue;
        }
        let affected = slot.observations.iter().any(|observation| {
            let observed = relativize(
                &root,
                &root_lexical,
                &clean_path(&PathBuf::from(&observation.path)),
            );
            changed.iter().any(|path| match observation.kind {
                FsObservationKind::Read | FsObservationKind::Absent => *path == observed,
                FsObservationKind::DirectoryListing => {
                    *path == observed
                        || Path::new(path)
                            .parent()
                            .map(|parent| parent == Path::new(&observed))
                            .unwrap_or(false)
                }
            })
        });
        if affected {
            slots.insert(slot.id.clone());
        }
    }
    SlotInvalidation {
        slots,
        source_files,
        coarse: false,
    }
}

/// Project-relative, forward-slash spelling of `path` under `project_root`,
/// tolerant of the macOS `/tmp` -> `/private/tmp` alias and of paths that no
/// longer exist (lexical cleaning). This is the one comparison spelling for
/// source files across manifest slots, fresh compiler plans, and watch
/// events.
pub fn project_relative_path(project_root: &Path, path: &Path) -> String {
    let root = clean_path(project_root);
    let root_lexical = lexical_clean(project_root);
    relativize(&root, &root_lexical, &clean_path(path))
}

/// Canonicalize when the path exists (resolves symlinks, e.g. macOS
/// `/tmp` -> `/private/tmp`); otherwise normalize `.`/`..` lexically so a
/// just-deleted path still compares equal to the path that was observed.
fn clean_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| lexical_clean(path))
}

fn lexical_clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn relativize(root: &Path, root_lexical: &Path, path: &Path) -> String {
    // Deleted paths cannot be canonicalized, so they keep the lexical
    // spelling; try both roots to stay consistent either way.
    let rel = path
        .strip_prefix(root)
        .or_else(|_| path.strip_prefix(root_lexical))
        .unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::{FsObservation, ManifestSlot};

    fn slot(id: &str, file: &str, observations: Vec<(&str, FsObservationKind)>) -> ManifestSlot {
        ManifestSlot {
            id: id.to_string(),
            binding: "staticParams".to_string(),
            file: file.to_string(),
            descriptor_digest: String::new(),
            value_module: format!("deka:dev/{id}"),
            observations: observations
                .into_iter()
                .map(|(path, kind)| FsObservation {
                    path: path.to_string(),
                    kind,
                })
                .collect(),
        }
    }

    fn manifest(slots: Vec<ManifestSlot>) -> BuildManifest {
        BuildManifest {
            version: 1,
            compiler: super::super::build_manifest::CompilerProvenance {
                plan_version: 1,
                dsc: None,
            },
            slots,
            routes: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    #[test]
    fn read_observation_invalidates_on_exact_path_change() {
        let root = std::env::temp_dir().join(format!(
            "deka-invalidation-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let manifest = manifest(vec![slot(
            "s1",
            "app/page.dsx",
            vec![("data/a.json", FsObservationKind::Read)],
        )]);
        let changed = root.join("data").join("a.json");
        let invalidation = affected_slots(&manifest, &root, &[changed]);
        assert_eq!(invalidation.slots, ["s1"].into_iter().map(str::to_string).collect());
        assert!(!invalidation.coarse);

        // An unrelated file does not invalidate.
        let other = root.join("data").join("b.json");
        assert!(affected_slots(&manifest, &root, &[other]).is_empty());
    }

    #[test]
    fn absent_observation_invalidates_when_the_path_appears_or_changes() {
        let root = std::env::temp_dir().join(format!(
            "deka-invalidation-test-absent-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let manifest = manifest(vec![slot(
            "s1",
            "app/page.dsx",
            vec![("data/new.json", FsObservationKind::Absent)],
        )]);
        let created = root.join("data").join("new.json");
        let invalidation = affected_slots(&manifest, &root, &[created]);
        assert!(invalidation.slots.contains("s1"));
    }

    #[test]
    fn directory_listing_invalidates_on_direct_child_and_self_changes() {
        let root = std::env::temp_dir().join(format!(
            "deka-invalidation-test-dir-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("data")).unwrap();
        let manifest = manifest(vec![slot(
            "s1",
            "app/page.dsx",
            vec![("data", FsObservationKind::DirectoryListing)],
        )]);
        // Added/renamed file directly inside the listed directory.
        let added = root.join("data").join("c.txt");
        assert!(affected_slots(&manifest, &root, &[added]).slots.contains("s1"));
        // The listed directory itself went away.
        assert!(affected_slots(&manifest, &root, &[root.join("data")])
            .slots
            .contains("s1"));
        // A nested change does not affect the parent's listing.
        let nested = root.join("data").join("sub").join("deep.txt");
        assert!(affected_slots(&manifest, &root, &[nested]).is_empty());
    }

    #[test]
    fn source_file_change_requests_a_wholesale_replan_not_stale_ids() {
        let root = std::env::temp_dir().join(format!(
            "deka-invalidation-test-src-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let manifest = manifest(vec![slot("s1", "app/page.dsx", Vec::new())]);
        let edit = root.join("app").join("page.dsx");
        let invalidation = affected_slots(&manifest, &root, &[edit]);
        // The slot id embeds the compiler span; an edit above the block
        // shifts it. The caller must replan the file, not refresh a stale id.
        assert!(invalidation.slots.is_empty(), "{invalidation:?}");
        assert_eq!(
            invalidation.source_files,
            ["app/page.dsx"].into_iter().map(str::to_string).collect()
        );
        assert!(!invalidation.coarse);
        // An unrelated data change leaves a pure slot alone.
        let data = root.join("data").join("a.json");
        assert!(affected_slots(&manifest, &root, &[data]).is_empty());
    }

    #[test]
    fn only_affected_slots_are_returned() {
        let root = std::env::temp_dir().join(format!(
            "deka-invalidation-test-multi-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let manifest = manifest(vec![
            slot("s1", "app/a.dsx", vec![("data/a.json", FsObservationKind::Read)]),
            slot("s2", "app/b.dsx", vec![("data/b.json", FsObservationKind::Read)]),
        ]);
        let changed = root.join("data").join("b.json");
        let invalidation = affected_slots(&manifest, &root, &[changed]);
        assert_eq!(invalidation.slots, ["s2"].into_iter().map(str::to_string).collect());
    }
}
