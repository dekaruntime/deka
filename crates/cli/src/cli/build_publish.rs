//! Transactional static publication for `deka build` (deka#719).
//!
//! Every dist-writing step targets a fresh staging tree under
//! `<project_root>/.deka-dist-stage/dist`; once all of them succeed, the
//! staged tree atomically replaces `dist/`. A *returned* publish error
//! restores the previous output from `dist.prev-<pid>` immediately, so
//! `dist/` is never left absent by a build the CLI reported as failed; a
//! *crash* mid-publish leaves it at `dist.prev-<pid>` with no `dist/` at
//! all, and [`recover_interrupted_publish`] rolls that forward at the start
//! of the next build. `DEKA_TEST_PUBLISH_PAUSE_MS` widens the window between
//! the two renames so tests can kill a real build inside the transaction.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use runtime_core::framework::{BuildManifest, DeferredIsland, RouteMode};

/// A fully written staging tree, ready to replace `dist/`.
pub struct StagedDist {
    pub root: PathBuf,
}

/// Create a fresh staging directory at `<project_root>/.deka-dist-stage`.
/// A leftover from a crashed prior build is removed first. The staging tree
/// lives on the same filesystem as `dist/` (both under the project root),
/// which the final `rename` requires.
pub fn stage_dist(project_root: &Path) -> Result<StagedDist, String> {
    let root = project_root.join(".deka-dist-stage");
    if root.exists() {
        fs::remove_dir_all(&root)
            .map_err(|err| format!("failed to remove {}: {err}", root.display()))?;
    }
    fs::create_dir_all(&root)
        .map_err(|err| format!("failed to create {}: {err}", root.display()))?;
    Ok(StagedDist { root })
}

/// Atomically replace `dist/` with the staged tree (the `dist/` child of
/// `staged.root`, where every dist-writing step wrote):
/// 1. move the existing `dist` aside to `dist.prev-<pid>` (removing a stale
///    same-pid leftover and retrying once),
/// 2. rename the staged `dist` into place — on failure, restore the
///    moved-aside backup immediately so a returned error never leaves
///    `dist/` absent,
/// 3. best-effort removal of the moved-aside tree (dead weight, not
///    correctness — a failure here is logged and ignored).
pub fn publish(project_root: &Path, staged: &StagedDist) -> Result<(), String> {
    let dist = project_root.join("dist");
    let staged_dist = staged.root.join("dist");
    let backup = project_root.join(format!("dist.prev-{}", std::process::id()));
    if dist.exists() {
        if let Err(err) = fs::rename(&dist, &backup) {
            if !backup.exists() {
                return Err(format!(
                    "failed to move {} aside: {err}",
                    dist.display()
                ));
            }
            fs::remove_dir_all(&backup)
                .map_err(|err| format!("failed to remove stale {}: {err}", backup.display()))?;
            fs::rename(&dist, &backup).map_err(|err| {
                format!(
                    "failed to move {} aside after removing stale backup: {err}",
                    dist.display()
                )
            })?;
        }
    }
    // Test-only fault injection (deka#719): widen the crash window between
    // the two renames so an integration test can SIGKILL a real build inside
    // the publication transaction and observe the recoverable state. Inert
    // unless DEKA_TEST_PUBLISH_PAUSE_MS is set to a positive number.
    if let Ok(ms) = std::env::var("DEKA_TEST_PUBLISH_PAUSE_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            if ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
        }
    }
    if let Err(err) = fs::rename(&staged_dist, &dist) {
        // Promotion failed after dist/ was moved aside: restore the previous
        // output immediately instead of leaving dist/ absent until the next
        // build's recovery pass.
        if backup.exists() {
            if let Err(restore_err) = fs::rename(&backup, &dist) {
                return Err(format!(
                    "failed to publish staged dist {} -> {}: {err}; \
                     and failed to restore the previous dist from {}: {restore_err}",
                    staged_dist.display(),
                    dist.display(),
                    backup.display()
                ));
            }
            stdio::warn(
                "build",
                &format!(
                    "publish failed; restored the previous dist from {}",
                    backup.display()
                ),
            );
        }
        return Err(format!(
            "failed to publish staged dist {} -> {}: {err}",
            staged_dist.display(),
            dist.display()
        ));
    }
    // The staged root is now an empty husk; remove it best-effort.
    let _ = fs::remove_dir_all(&staged.root);
    if backup.exists() {
        if let Err(err) = fs::remove_dir_all(&backup) {
            stdio::warn(
                "build",
                &format!(
                    "failed to remove superseded {} (output is still correct): {err}",
                    backup.display()
                ),
            );
        }
    }
    Ok(())
}

/// The staged tree is complete: hash its artifacts into the manifests, persist
/// them (v1 beside the compiler cache for `deka dev`, v2 inside the staged
/// dist as the deployment descriptor), atomically replace `dist/`, then print
/// the route table. The table prints only after a successful publish; a failed
/// build prints nothing.
pub fn finalize(
    project_root: &Path,
    staged: &StagedDist,
    manifest: Option<&mut BuildManifest>,
    artifact: Option<runtime_core::framework::ArtifactManifestV2>,
) -> Result<(), String> {
    let mut manifest = manifest;
    let staged_dist = staged.root.join("dist");
    if let Some(manifest) = manifest.as_deref_mut() {
        manifest.record_artifacts(&staged_dist)?;
        manifest.write(
            &runtime_core::framework::compiler_cache_dir(project_root).join("build-manifest.json"),
        )?;
    }
    if let Some(mut artifact) = artifact {
        artifact.record_payloads(&staged_dist)?;
        artifact.write_into(&staged_dist)?;
    }
    publish(project_root, staged)?;
    if let Some(manifest) = manifest {
        stdio::hint("routes:");
        for line in manifest.render_route_table().lines() {
            stdio::raw(line);
        }
    }
    Ok(())
}

/// Build the v2 deployment descriptor from the planned v1 manifest and the
/// emitted server entries (deka#762). `app`/`api` are the SAME scans the
/// manifest was planned from — publication must not re-scan sources
/// (deka#719: rendering, logging and publication read the manifest boundary,
/// not fresh source walks). Payloads are recorded later, in [`finalize`],
/// once the staged tree is final.
#[allow(clippy::too_many_arguments)]
pub fn build_artifact_manifest(
    manifest: &BuildManifest,
    project_root: &Path,
    dist_root: &Path,
    worker_emitted: bool,
    trailing_slash: bool,
    app: &runtime_core::framework::FrameworkManifest,
    api: &[runtime_core::framework::FrameworkEntry],
) -> Result<runtime_core::framework::ArtifactManifestV2, String> {
    use runtime_core::framework::{
        ARTIFACT_FORMAT, ArtifactClient, ArtifactCompat, ArtifactManifestV2, ArtifactProducer,
        ArtifactRoute, ArtifactServer, ArtifactSlot, ArtifactWorker, MODULE_FORMAT, RUNTIME_ABI,
        RouteMode, client_output_path, server_entries,
    };


    let routes: Vec<ArtifactRoute> = manifest
        .routes
        .iter()
        .map(|route| {
            let entry = match route.mode {
                RouteMode::RequestTime => Some(format!("page:{}", route.template)),
                RouteMode::Api => Some(format!("api:{}", route.template)),
                RouteMode::Static | RouteMode::StaticParams => None,
            };
            let outputs = match route.mode {
                RouteMode::Static => vec![client_output_path(&route.template)],
                RouteMode::StaticParams => route
                    .instances
                    .iter()
                    .map(|instance| client_output_path(instance))
                    .collect(),
                RouteMode::RequestTime | RouteMode::Api => Vec::new(),
            };
            ArtifactRoute {
                template: route.template.clone(),
                mode: route.mode,
                instances: route.instances.clone(),
                outputs,
                entry,
                slot: route.slot.clone(),
                deferred: route.deferred.clone(),
                source_file: route.source_file.clone(),
            }
        })
        .collect();

    let entries = server_entries(project_root, app, api)?;

    let slots: Vec<ArtifactSlot> = manifest
        .slots
        .iter()
        .map(|slot| ArtifactSlot {
            id: slot.id.clone(),
            binding: slot.binding.clone(),
            file: slot.file.clone(),
            descriptor_digest: format!("sha256:{}", slot.descriptor_digest),
            module: format!("server/.values/{}.js", slot.id),
            observations: Vec::new(),
        })
        .collect();

    let client_index = dist_root.join("client/index.html");

    Ok(ArtifactManifestV2 {
        format: ARTIFACT_FORMAT.to_string(),
        origin: "authored".to_string(),
        producer: ArtifactProducer {
            deka: env!("CARGO_PKG_VERSION").to_string(),
            dsc: manifest
                .compiler
                .dsc
                .clone()
                .filter(|dsc| !dsc.is_empty())
                .or_else(crate::cli::build_dsc::dsc_identity)
                .unwrap_or_else(|| "unknown".to_string()),
            plan_version: manifest.compiler.plan_version,
        },
        compat: ArtifactCompat {
            runtime_abi: RUNTIME_ABI,
            module_format: MODULE_FORMAT.to_string(),
            targets: vec!["native".to_string(), "worker".to_string()],
            host_imports: Vec::new(),
        },
        client: ArtifactClient {
            root: "client".to_string(),
            index: client_index
                .is_file()
                .then(|| "client/index.html".to_string()),
            trailing_slash,
        },
        server: ArtifactServer {
            root: "server".to_string(),
            entries,
        },
        worker: worker_emitted.then(|| ArtifactWorker {
            entry: "_worker.js".to_string(),
        }),
        routes,
        slots,
        payloads: Vec::new(),
        payload_root: String::new(),
    })
}

/// Roll forward a publish that crashed between "move aside" and "rename into
/// place": when `dist/` is absent and `dist.prev-*` trees exist, restore the
/// newest one and drop the rest. When `dist/` exists, stray `dist.prev-*`
/// trees are left alone (they belong to an in-flight session; `publish`
/// manages its own backup).
pub fn recover_interrupted_publish(project_root: &Path) {
    let dist = project_root.join("dist");
    if dist.exists() {
        return;
    }
    let Ok(reader) = fs::read_dir(project_root) else {
        return;
    };
    let mut prevs: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in reader.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("dist.prev-") || !entry.path().is_dir() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        prevs.push((modified, entry.path()));
    }
    if prevs.is_empty() {
        return;
    }
    prevs.sort_by(|a, b| a.0.cmp(&b.0));
    let (newest_modified, newest) = prevs.pop().expect("non-empty");
    match fs::rename(&newest, &dist) {
        Ok(()) => {
            let age = newest_modified
                .elapsed()
                .map(|d| format!("{}s ago", d.as_secs()))
                .unwrap_or_else(|_| "unknown age".to_string());
            stdio::warn(
                "build",
                &format!(
                    "rolled forward output from an interrupted build (restored {} from {})",
                    dist.display(),
                    age
                ),
            );
        }
        Err(err) => {
            stdio::error(
                "build",
                &format!(
                    "failed to roll forward interrupted build output {} -> {}: {err}",
                    newest.display(),
                    dist.display()
                ),
            );
            return;
        }
    }
    for (_, stale) in prevs {
        let _ = fs::remove_dir_all(&stale);
    }
}

/// One static render per planned static output: plain `○` rows render their
/// template, `●` rows render each concrete instance with its literal params.
/// `ƒ`/`λ` rows publish no static HTML.
pub fn render_tasks(manifest: &BuildManifest) -> Vec<runtime::StaticRenderTask> {
    let mut tasks = Vec::new();
    for route in &manifest.routes {
        match route.mode {
            RouteMode::Static => tasks.push(runtime::StaticRenderTask {
                template: route.template.clone(),
                route: route.template.clone(),
                params: BTreeMap::new(),
            }),
            RouteMode::StaticParams => {
                // The manifest stores one entry per template (deka#738 F6);
                // each recorded instance is its own render task.
                for instance in &route.instances {
                    tasks.push(runtime::StaticRenderTask {
                        template: route.template.clone(),
                        route: instance.clone(),
                        params: params_for_instance(&route.template, instance),
                    });
                }
            }
            RouteMode::RequestTime | RouteMode::Api => {}
        }
    }
    tasks
}

/// Re-derive an instance's parameter values from its template: each `[name]`
/// template segment pairs with the same-position instance segment.
pub fn params_for_instance(
    template: &str,
    instance: &str,
) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    let template_segments: Vec<&str> = template.trim_matches('/').split('/').collect();
    let instance_segments: Vec<&str> = instance.trim_matches('/').split('/').collect();
    for (index, segment) in template_segments.iter().enumerate() {
        if let Some(name) = segment
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
        {
            if let Some(value) = instance_segments.get(index) {
                params.insert(name.to_string(), (*value).to_string());
            }
        }
    }
    params
}

/// Record `server:defer` component names on each manifest route whose source
/// file declares them (deka#718's `◐` classification input; not consumed yet).
pub fn apply_deferred(
    manifest: &mut BuildManifest,
    deferred: &[DeferredIsland],
    project_root: &Path,
) {
    if deferred.is_empty() {
        return;
    }
    for route in &mut manifest.routes {
        let names: Vec<String> = deferred
            .iter()
            .filter(|item| same_source_file(project_root, &item.file, &route.source_file))
            .map(|item| item.component.clone())
            .collect();
        if !names.is_empty() {
            route.deferred = names;
        }
    }
}

/// Deferred scans and manifest scans both walk `project_root/app`, but one
/// side may spell the root relatively; compare project-relative.
fn same_source_file(project_root: &Path, a: &str, b: &str) -> bool {
    let normalize = |path: &str| {
        let path = Path::new(path);
        let rel = path.strip_prefix(project_root).unwrap_or(path);
        rel.to_string_lossy().replace('\\', "/")
    };
    normalize(a) == normalize(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, body).expect("write");
    }

    fn tree_digest(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            for entry in fs::read_dir(&current).expect("read_dir").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(dir).expect("rel").to_string_lossy().into_owned();
                    out.insert(rel, fs::read(&path).expect("read"));
                }
            }
        }
        out
    }

    #[test]
    fn publish_swaps_staged_tree_over_dist() {
        let project = tempfile::tempdir().unwrap();
        write(project.path(), "dist/old.txt", "old");
        write(project.path(), "dist/client/index.html", "old index");
        let staged = stage_dist(project.path()).unwrap();
        write(&staged.root, "dist/new.txt", "new");
        publish(project.path(), &staged).unwrap();

        let dist = project.path().join("dist");
        assert_eq!(fs::read(dist.join("new.txt")).unwrap(), b"new");
        assert!(!dist.join("old.txt").exists(), "staged tree must fully replace dist");
        assert!(!project.path().join(".deka-dist-stage").exists());
        assert!(
            fs::read_dir(project.path())
                .unwrap()
                .flatten()
                .all(|entry| !entry.file_name().to_string_lossy().starts_with("dist.prev-")),
            "backup must be removed after a successful publish"
        );
    }

    #[test]
    fn failed_staged_build_leaves_prior_dist_intact() {
        let project = tempfile::tempdir().unwrap();
        write(project.path(), "dist/marker.txt", "v1");
        let before = tree_digest(&project.path().join("dist"));

        // A build that fails BEFORE publish never touches dist; simulate by
        // staging and abandoning (no publish call).
        let staged = stage_dist(project.path()).unwrap();
        write(&staged.root, "dist/marker.txt", "v2-corrupt");
        drop(staged);

        let after = tree_digest(&project.path().join("dist"));
        assert_eq!(before, after, "prior dist must remain byte-identical");
    }

    #[test]
    fn failed_promotion_restores_prior_dist() {
        let project = tempfile::tempdir().unwrap();
        write(project.path(), "dist/marker.txt", "v1");

        // Promotion fails when the staged tree has no `dist` child to rename
        // into place (simulates a failure between staging writes and the
        // final rename).
        let staged = stage_dist(project.path()).unwrap();
        let err = publish(project.path(), &staged).expect_err("publish must fail");
        assert!(
            err.contains("failed to publish staged dist"),
            "error should name the promotion step: {err}"
        );

        let dist = project.path().join("dist");
        assert_eq!(
            fs::read(dist.join("marker.txt")).unwrap(),
            b"v1",
            "the previous dist must be restored immediately, not left at dist.prev-*"
        );
        assert!(
            fs::read_dir(project.path())
                .unwrap()
                .flatten()
                .all(|entry| !entry.file_name().to_string_lossy().starts_with("dist.prev-")),
            "a restored backup must be consumed"
        );
    }

    #[test]
    fn recovery_restores_prev_when_dist_missing() {
        let project = tempfile::tempdir().unwrap();
        write(project.path(), "dist.prev-1234/good.txt", "good");
        recover_interrupted_publish(project.path());
        let dist = project.path().join("dist");
        assert_eq!(fs::read(dist.join("good.txt")).unwrap(), b"good");
        assert!(
            fs::read_dir(project.path())
                .unwrap()
                .flatten()
                .all(|entry| !entry.file_name().to_string_lossy().starts_with("dist.prev-")),
            "recovered backup must be consumed"
        );
    }

    #[test]
    fn recovery_restores_newest_prev_and_drops_older_ones() {
        let project = tempfile::tempdir().unwrap();
        write(project.path(), "dist.prev-1/old.txt", "older");
        write(project.path(), "dist.prev-2/newer.txt", "newest");
        // Deterministic mtimes: 2020 for the older tree, 2021 for the newer.
        for (dir, stamp) in [("dist.prev-1", "202001010000"), ("dist.prev-2", "202101010000")] {
            let status = std::process::Command::new("touch")
                .args(["-t", stamp])
                .arg(project.path().join(dir))
                .status()
                .expect("run touch");
            assert!(status.success(), "touch -t must succeed");
        }
        recover_interrupted_publish(project.path());
        let dist = project.path().join("dist");
        assert!(dist.join("newer.txt").exists(), "newest backup must win");
        assert!(!dist.join("old.txt").exists());
        assert!(!project.path().join("dist.prev-1").exists());
        assert!(!project.path().join("dist.prev-2").exists());
    }

    #[test]
    fn recovery_noops_when_dist_present() {
        let project = tempfile::tempdir().unwrap();
        write(project.path(), "dist/current.txt", "current");
        write(project.path(), "dist.prev-1/stale.txt", "stale");
        recover_interrupted_publish(project.path());
        assert!(project.path().join("dist").join("current.txt").exists());
        assert!(
            project.path().join("dist.prev-1").join("stale.txt").exists(),
            "stray backups must be left alone while dist exists"
        );
    }

    #[test]
    fn params_for_instance_pairs_bracket_segments() {
        let params = params_for_instance("/posts/[slug]", "/posts/hello");
        assert_eq!(params.get("slug").map(String::as_str), Some("hello"));
        assert!(params_for_instance("/about", "/about").is_empty());
    }
}
