//! Authored-artifact discovery and verification for `deka serve`.
//!
//! This is deliberately separate from request handling: startup must verify
//! the artifact before `serve` constructs any listener, while the resulting
//! descriptor stays with the dispatcher for lazy client-payload checks.

use std::path::{Path, PathBuf};

use runtime_core::dist::ArtifactManifestV2;

pub(crate) struct VerifiedArtifact {
    pub(crate) root: PathBuf,
    pub(crate) manifest: ArtifactManifestV2,
}

/// Locate the authored artifact containing a resolved server entry and verify
/// its manifest. Source-posture handlers never match because their path is
/// not under an artifact's `server/` tree.
pub(crate) fn load_verified(resolved_path: &Path) -> Result<Option<VerifiedArtifact>, String> {
    let Some(root) = built_artifact_root(resolved_path) else {
        return Ok(None);
    };
    let manifest = ArtifactManifestV2::load_verified(&root)?;
    Ok(Some(VerifiedArtifact { root, manifest }))
}

/// The artifact root when the resolved handler is a compiled
/// `dist/server/serve-entry.js`: an ancestor directory that carries
/// `build-manifest.json` and owns the `server/` tree the entry lives in.
fn built_artifact_root(resolved_path: &Path) -> Option<PathBuf> {
    let start = if resolved_path.is_dir() {
        resolved_path.to_path_buf()
    } else {
        resolved_path.parent()?.to_path_buf()
    };
    let mut probe = Some(start.as_path());
    while let Some(dir) = probe {
        if dir.join("build-manifest.json").is_file()
            && resolved_path.starts_with(dir.join("server"))
        {
            return Some(dir.to_path_buf());
        }
        probe = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::built_artifact_root;
    use std::fs;

    #[test]
    fn finds_the_artifact_owning_a_server_entry() {
        let artifact = tempfile::tempdir().expect("create artifact root");
        let entry = artifact.path().join("server").join("serve-entry.js");
        fs::create_dir_all(entry.parent().expect("entry parent")).expect("create server tree");
        fs::write(artifact.path().join("build-manifest.json"), "{}").expect("write manifest");
        fs::write(&entry, "export {};").expect("write entry");

        assert_eq!(
            built_artifact_root(&entry).as_deref(),
            Some(artifact.path())
        );
    }

    #[test]
    fn ignores_source_posture_entries() {
        let project = tempfile::tempdir().expect("create project root");
        let entry = project
            .path()
            .join(".cache")
            .join("dekascript")
            .join("serve-entry.dsx");
        fs::create_dir_all(entry.parent().expect("entry parent")).expect("create source tree");
        fs::write(project.path().join("build-manifest.json"), "{}").expect("write manifest");
        fs::write(&entry, "source").expect("write source entry");

        assert_eq!(built_artifact_root(&entry), None);
    }
}
