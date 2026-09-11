#![allow(clippy::all)]

use core::Context;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::OnceLock;

mod artifact_loader;
mod build_values;
mod css;
mod dev;
mod dsc_transpile;
mod extensions;
mod islands;
mod js_pipeline;
mod platform;
mod prerender;
mod run;
pub mod security;
mod serve;

pub mod build_watch;

pub fn run(context: &Context) {
    run::run(context);
}

pub fn run_with_dsc(context: &Context, dsc: Option<std::path::PathBuf>) {
    run::run_with_dsc(context, dsc);
}

pub fn serve(context: &Context) {
    serve::serve(context);
}

pub fn serve_with_dsc(context: &Context, dsc: Option<std::path::PathBuf>) {
    serve::serve_with_dsc(context, dsc);
}

pub fn prerender_static_pages(
    project_root: &std::path::Path,
    dist_client: &std::path::Path,
    tasks: &[prerender::StaticRenderTask],
    policy_json: &str,
    dsc: Option<std::path::PathBuf>,
) -> Result<(), String> {
    prerender::prerender_static_pages(project_root, dist_client, tasks, policy_json, dsc)
}

pub use prerender::StaticRenderTask;

pub fn materialize_build_values(
    project_root: &std::path::Path,
    entries: Vec<build_values::BuildEntry>,
    policy_json: &str,
    only: Option<&std::collections::BTreeSet<String>>,
    dsc: Option<std::path::PathBuf>,
    dev_mode: bool,
) -> Result<build_values::MaterializedBuild, String> {
    build_values::materialize_build_values(project_root, entries, policy_json, only, dsc, dev_mode)
}

pub use build_values::{BuildEntry, MaterializedBuild};

pub fn write_island_client_assets(
    assets_dir: &std::path::Path,
    islands: &[runtime_core::framework::ClientIsland],
    flavor: islands::ClientAssetFlavor,
) -> Result<(), String> {
    islands::write_island_client_assets(assets_dir, islands, flavor)
}

pub fn write_island_client_assets_with_dsc(
    assets_dir: &std::path::Path,
    islands: &[runtime_core::framework::ClientIsland],
    flavor: islands::ClientAssetFlavor,
    dsc: &std::path::Path,
) -> Result<(), String> {
    islands::write_island_client_assets_with_dsc(assets_dir, islands, flavor, dsc)
}

pub fn write_island_client_assets_for_project(
    project_root: &std::path::Path,
    flavor: islands::ClientAssetFlavor,
) -> Result<(), String> {
    islands::write_island_client_assets_for_project(project_root, flavor)
}

pub fn write_defer_client_assets(
    assets_dir: &std::path::Path,
    flavor: islands::ClientAssetFlavor,
) -> Result<(), String> {
    islands::write_defer_client_assets(assets_dir, flavor)
}

pub use islands::ClientAssetFlavor;

pub use islands::collect_hashed_asset_renames;
pub use islands::inline_importmap_tag;

pub fn rewrite_serve_entry_asset_urls(project_root: &std::path::Path) -> Result<(), String> {
    islands::rewrite_serve_entry_asset_urls(project_root)
}

pub fn write_route_css_assets(
    assets_dir: &std::path::Path,
    styles: &[runtime_core::framework::RouteStyle],
) -> Result<(), String> {
    css::write_route_css_assets(assets_dir, styles)
}

pub fn write_route_css_assets_for_project(
    project_root: &std::path::Path,
) -> Result<(), String> {
    css::write_route_css_assets_for_project(project_root)
}

pub fn platform(context: &Context) {
    platform::platform(context);
}

pub fn serve_desktop(context: &Context) {
    let _ = context;
    eprintln!("[desktop] desktop runtime mode is deferred in reboot MVP");
    std::process::exit(1);
}

const VFS_MAGIC: &[u8; 8] = b"DEKAVFS1";
const VFS_TAIL_SCAN_BYTES: u64 = 64 * 1024;
const VFS_MIN_METADATA_BYTES: usize = 28;

static HAS_EMBEDDED_VFS: OnceLock<bool> = OnceLock::new();
static EMBEDDED_VFS_BYTES: OnceLock<Option<Vec<u8>>> = OnceLock::new();

pub fn has_embedded_vfs() -> bool {
    *HAS_EMBEDDED_VFS.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|path| find_embedded_vfs_metadata(&path).ok())
            .is_some()
    })
}

pub fn read_embedded_vfs() -> Option<compile::vfs::VFS> {
    let bytes = EMBEDDED_VFS_BYTES
        .get_or_init(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| read_embedded_vfs_bytes(&path).ok())
        })
        .as_ref()?;

    compile::vfs::VFS::from_bytes(bytes).ok()
}

fn find_embedded_vfs_metadata(path: &Path) -> Result<compile::binary::BinaryMetadata, String> {
    let mut file =
        File::open(path).map_err(|err| format!("failed to open executable for VFS scan: {err}"))?;
    let file_len = file
        .metadata()
        .map_err(|err| format!("failed to stat executable for VFS scan: {err}"))?
        .len();

    if file_len < VFS_MIN_METADATA_BYTES as u64 {
        return Err("executable is too small to contain embedded VFS metadata".to_string());
    }

    let scan_len = file_len.min(VFS_TAIL_SCAN_BYTES) as usize;
    let scan_start = file_len - scan_len as u64;
    file.seek(SeekFrom::Start(scan_start))
        .map_err(|err| format!("failed to seek executable tail for VFS scan: {err}"))?;

    let mut tail = vec![0; scan_len];
    file.read_exact(&mut tail)
        .map_err(|err| format!("failed to read executable tail for VFS scan: {err}"))?;

    if scan_len < VFS_MIN_METADATA_BYTES {
        return Err("executable tail is too small to contain embedded VFS metadata".to_string());
    }

    let last_start = scan_len.saturating_sub(VFS_MIN_METADATA_BYTES);
    for idx in (0..=last_start).rev() {
        if tail.get(idx..idx + VFS_MAGIC.len()) != Some(VFS_MAGIC.as_slice()) {
            continue;
        }

        let metadata = match compile::binary::BinaryMetadata::from_bytes(&tail[idx..]) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let metadata_start = scan_start + idx as u64;
        let vfs_end = metadata
            .vfs_offset
            .checked_add(metadata.vfs_size)
            .ok_or_else(|| "embedded VFS offset overflow".to_string())?;

        if vfs_end <= metadata_start {
            return Ok(metadata);
        }
    }

    Err("no embedded VFS metadata found".to_string())
}

fn read_embedded_vfs_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = find_embedded_vfs_metadata(path)?;
    let mut file =
        File::open(path).map_err(|err| format!("failed to open executable for VFS read: {err}"))?;
    file.seek(SeekFrom::Start(metadata.vfs_offset))
        .map_err(|err| format!("failed to seek embedded VFS: {err}"))?;

    let mut vfs_bytes = vec![0; metadata.vfs_size as usize];
    file.read_exact(&mut vfs_bytes)
        .map_err(|err| format!("failed to read embedded VFS: {err}"))?;
    Ok(vfs_bytes)
}

pub fn run_embedded_vfs(args: Vec<String>) -> Result<(), String> {
    let vfs = read_embedded_vfs().ok_or_else(|| "No embedded VFS found".to_string())?;
    let temp_dir = tempfile::Builder::new()
        .prefix("deka-embedded-vfs-")
        .tempdir()
        .map_err(|err| format!("failed to create embedded VFS tempdir: {err}"))?;
    let entry_point = materialize_vfs(&vfs, temp_dir.path())?;
    let entry_arg = entry_point.to_string_lossy().to_string();

    let mut positionals = Vec::with_capacity(args.len() + 1);
    positionals.push(entry_arg.clone());
    positionals.extend(args);

    let cli_args = core::Args {
        flags: std::collections::HashMap::new(),
        params: std::collections::HashMap::new(),
        commands: vec!["run".to_string()],
        positionals,
    };
    let env = core::EnvContext::load();
    let resolved = core::resolve_handler_path(&entry_arg)
        .map_err(|err| format!("failed to resolve embedded VFS entry point: {err}"))?;
    let static_config = core::StaticServeConfig::load(&resolved.directory);
    let handler = core::HandlerContext {
        input: entry_arg,
        resolved,
        static_config,
        serve_config_path: None,
    };
    let context = Context {
        args: cli_args,
        env,
        handler,
    };

    run(&context);
    Ok(())
}

fn materialize_vfs(vfs: &compile::vfs::VFS, root: &Path) -> Result<std::path::PathBuf, String> {
    let entry_relative = Path::new(&vfs.entry_point);
    if !is_safe_vfs_path(entry_relative) {
        return Err(format!(
            "embedded VFS contains unsafe entry point: {}",
            vfs.entry_point
        ));
    }

    for (path, entry) in &vfs.files {
        let relative = Path::new(path);
        if !is_safe_vfs_path(relative) {
            return Err(format!("embedded VFS contains unsafe path: {path}"));
        }

        let output = root.join(relative);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create embedded VFS directory: {err}"))?;
        }

        let content = if entry.metadata.compressed {
            let mut decoder = flate2::read::GzDecoder::new(&entry.content[..]);
            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|err| format!("failed to decompress embedded VFS file {path}: {err}"))?;
            decompressed
        } else {
            entry.content.clone()
        };

        std::fs::write(&output, content)
            .map_err(|err| format!("failed to write embedded VFS file {path}: {err}"))?;
    }

    Ok(root.join(entry_relative))
}

fn is_safe_vfs_path(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, std::path::Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use compile::binary::BinaryEmbedder;
    use compile::vfs::{RuntimeMode, VFS};

    #[test]
    fn detects_and_reads_vfs_from_binary_tail() {
        let dir = tempfile::tempdir().unwrap();
        let runtime_path = dir.path().join("runtime");
        let output_path = dir.path().join("app");
        std::fs::write(&runtime_path, b"fake-runtime").unwrap();

        let mut vfs = VFS::new("index.phpx".to_string(), RuntimeMode::Server);
        vfs.add_file(
            "index.phpx".to_string(),
            b"<?php echo 'hello';".to_vec(),
            "phpx".to_string(),
            false,
        );
        let bytes = vfs.to_bytes().unwrap();
        BinaryEmbedder::new(runtime_path)
            .embed(&bytes, "index.phpx", &output_path)
            .unwrap();

        let metadata = find_embedded_vfs_metadata(&output_path).unwrap();
        assert_eq!(metadata.entry_point, "index.phpx");

        let decoded = VFS::from_bytes(&read_embedded_vfs_bytes(&output_path).unwrap()).unwrap();
        assert_eq!(decoded.entry_point, "index.phpx");
        assert_eq!(
            decoded.get_file("index.phpx").unwrap().content,
            b"<?php echo 'hello';"
        );
    }

    #[test]
    fn ignores_binaries_without_vfs_magic() {
        let dir = tempfile::tempdir().unwrap();
        let output_path = dir.path().join("plain");
        std::fs::write(&output_path, b"plain executable").unwrap();

        assert!(find_embedded_vfs_metadata(&output_path).is_err());
    }

    #[test]
    fn rejects_magic_that_is_not_valid_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let output_path = dir.path().join("plain");
        std::fs::write(&output_path, b"plain DEKAVFS1 executable").unwrap();

        assert!(find_embedded_vfs_metadata(&output_path).is_err());
    }
}
