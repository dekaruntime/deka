use crate::validation::imports::ImportKind;
use crate::validation::modules::collect_import_specs;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PackageIntegrity {
    pub fs_graph: String,
    pub module_graph: String,
}

pub fn compute_package_integrity(root: &Path) -> Result<PackageIntegrity, String> {
    let fs_graph = compute_fs_graph_hash(root)?;
    let module_graph = compute_module_graph_hash(root)?;
    Ok(PackageIntegrity {
        fs_graph,
        module_graph,
    })
}

fn compute_fs_graph_hash(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for path in files {
        let rel = path
            .strip_prefix(root)
            .map_err(|_| "failed to normalize integrity path")?;
        let rel_str = normalize_rel(rel);
        hasher.update(rel_str.as_bytes());
        hasher.update(b"\0");

        let mut file = File::open(&path)
            .map_err(|err| format!("failed to open {}: {}", path.display(), err))?;
        let mut buf = [0u8; 8192];
        loop {
            let read = file
                .read(&mut buf)
                .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
        }
        hasher.update(b"\n");
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn compute_module_graph_hash(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_phpx_files(root, root, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for path in files {
        let rel = path
            .strip_prefix(root)
            .map_err(|_| "failed to normalize module graph path")?;
        let rel_str = normalize_rel(rel);
        let source = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        let specs = collect_import_specs(&source, path.to_string_lossy().as_ref());
        let mut imports = specs
            .into_iter()
            .map(|spec| {
                format!(
                    "{}:{}:{}:{}",
                    import_kind_label(spec.kind),
                    spec.from,
                    spec.imported,
                    spec.local
                )
            })
            .collect::<Vec<_>>();
        imports.sort();

        hasher.update(rel_str.as_bytes());
        hasher.update(b"\0");
        for import in imports {
            hasher.update(import.as_bytes());
            hasher.update(b"\0");
        }
        hasher.update(b"\n");
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn import_kind_label(kind: ImportKind) -> &'static str {
    match kind {
        ImportKind::Phpx => "phpx",
        ImportKind::Wasm => "wasm",
    }
}

fn collect_files(root: &Path, current: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|err| format!("failed to read {}: {}", current.display(), err))?
    {
        let entry = entry.map_err(|err| format!("failed to read entry: {}", err))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to read entry type: {}", err))?;
        if file_type.is_dir() {
            if should_ignore_dir(&path, root) {
                continue;
            }
            collect_files(root, &path, out)?;
        } else if file_type.is_file() {
            if should_ignore_file(&path, root) {
                continue;
            }
            out.push(path);
        }
    }
    Ok(())
}

fn collect_phpx_files(root: &Path, current: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|err| format!("failed to read {}: {}", current.display(), err))?
    {
        let entry = entry.map_err(|err| format!("failed to read entry: {}", err))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to read entry type: {}", err))?;
        if file_type.is_dir() {
            if should_ignore_dir(&path, root) {
                continue;
            }
            collect_phpx_files(root, &path, out)?;
        } else if file_type.is_file() {
            if should_ignore_file(&path, root) {
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) == Some("phpx") {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn should_ignore_dir(path: &Path, root: &Path) -> bool {
    let rel = match path.strip_prefix(root) {
        Ok(rel) => rel,
        Err(_) => return true,
    };
    let rel_str = normalize_rel(rel);
    rel_str == ".git"
        || rel_str == ".cache"
        || rel_str == "php_modules/.cache"
        || rel_str == "node_modules"
        || rel_str == "target"
        || rel_str.starts_with(".git/")
        || rel_str.starts_with(".cache/")
        || rel_str.starts_with("php_modules/.cache/")
        || rel_str.starts_with("node_modules/")
        || rel_str.starts_with("target/")
}

fn should_ignore_file(path: &Path, root: &Path) -> bool {
    let rel = match path.strip_prefix(root) {
        Ok(rel) => rel,
        Err(_) => return true,
    };
    let rel_str = normalize_rel(rel);
    rel_str == ".DS_Store"
}

fn normalize_rel(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{compute_fs_graph_hash, compute_module_graph_hash};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn fs_graph_hash_changes_on_file_edit() {
        let dir = tempdir().expect("tmp");
        let root = dir.path();
        fs::write(root.join("mod.phpx"), "import { a } from 'core/result'").unwrap();
        let first = compute_fs_graph_hash(root).expect("hash");
        fs::write(root.join("mod.phpx"), "import { b } from 'core/result'").unwrap();
        let second = compute_fs_graph_hash(root).expect("hash");
        assert_ne!(first, second);
    }

    #[test]
    fn module_graph_hash_changes_on_import_edit() {
        let dir = tempdir().expect("tmp");
        let root = dir.path();
        fs::write(root.join("mod.phpx"), "import { a } from 'core/result'").unwrap();
        let first = compute_module_graph_hash(root).expect("hash");
        fs::write(root.join("mod.phpx"), "import { a } from 'core/bytes'").unwrap();
        let second = compute_module_graph_hash(root).expect("hash");
        assert_ne!(first, second);
    }
}
