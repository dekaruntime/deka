use crate::vfs::{RuntimeMode, VFS};
use flate2::Compression;
use flate2::write::GzEncoder;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Discover and embed files into VFS
pub struct Embedder {
    /// Root directory to scan
    root: PathBuf,
    /// Files to include (patterns or specific paths)
    include: Vec<String>,
    /// Files to exclude (patterns)
    exclude: Vec<String>,
}

impl Embedder {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            include: vec![
                "**/*.js".to_string(),
                "**/*.ts".to_string(),
                "**/*.php".to_string(),
            ],
            exclude: vec![
                "node_modules/**".to_string(),
                "target/**".to_string(),
                ".git/**".to_string(),
                "*.wasm".to_string(),
            ],
        }
    }

    /// Add include pattern
    pub fn include(mut self, pattern: String) -> Self {
        self.include.push(pattern);
        self
    }

    /// Add exclude pattern
    pub fn exclude(mut self, pattern: String) -> Self {
        self.exclude.push(pattern);
        self
    }

    /// Scan directory and build VFS
    pub fn build(&self, entry_point: &str, mode: RuntimeMode) -> Result<VFS, String> {
        let mut vfs = VFS::new(entry_point.to_string(), mode);

        // For now, just scan the root directory for common files
        // TODO: Implement proper glob pattern matching
        self.scan_directory(&self.root, &mut vfs)?;

        Ok(vfs)
    }

    /// Recursively scan directory
    fn scan_directory(&self, dir: &Path, vfs: &mut VFS) -> Result<(), String> {
        if !dir.is_dir() {
            return Ok(());
        }

        let entries =
            fs::read_dir(dir).map_err(|e| format!("Failed to read directory {:?}: {}", dir, e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();

            // Skip excluded patterns
            if self.should_exclude(&path) {
                continue;
            }

            if path.is_dir() {
                self.scan_directory(&path, vfs)?;
            } else if path.is_file() {
                self.embed_file(&path, vfs)?;
            }
        }

        Ok(())
    }

    /// Check if path should be excluded
    fn should_exclude(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        // Check if path matches any exclude pattern
        for pattern in &self.exclude {
            if pattern.ends_with("/**") {
                let prefix = pattern.trim_end_matches("/**");
                if path_str.contains(prefix) {
                    return true;
                }
            } else if pattern.starts_with("*.") {
                let ext = pattern.trim_start_matches("*.");
                if path_str.ends_with(ext) {
                    return true;
                }
            }
        }

        false
    }

    /// Embed a single file into VFS
    fn embed_file(&self, path: &Path, vfs: &mut VFS) -> Result<(), String> {
        // Read file contents
        let content =
            fs::read(path).map_err(|e| format!("Failed to read file {:?}: {}", path, e))?;

        // Get relative path from root
        let relative_path = path
            .strip_prefix(&self.root)
            .map_err(|_| format!("Path {:?} is not under root {:?}", path, self.root))?
            .to_string_lossy()
            .to_string();

        // Determine file type from extension
        let file_type = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Compress content for text files
        let (final_content, compressed) = if should_compress(&file_type) {
            match compress_content(&content) {
                Ok(compressed_data) => (compressed_data, true),
                Err(_) => (content, false), // Fall back to uncompressed if compression fails
            }
        } else {
            (content, false)
        };

        // Add to VFS
        vfs.add_file(relative_path, final_content, file_type, compressed);

        Ok(())
    }
}

/// Determine if file type should be compressed
fn should_compress(file_type: &str) -> bool {
    matches!(
        file_type,
        "js" | "ts" | "jsx" | "tsx" | "php" | "json" | "css" | "html" | "txt" | "md"
    )
}

/// Compress content using gzip
fn compress_content(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(data)
        .map_err(|e| format!("Compression failed: {}", e))?;
    encoder
        .finish()
        .map_err(|e| format!("Compression finalization failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::RuntimeMode;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_embedder() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("handler.js");
        fs::write(&file_path, b"export default {}").unwrap();

        let embedder = Embedder::new(dir.path().to_path_buf());
        let vfs = embedder.build("handler.js", RuntimeMode::Server).unwrap();

        assert_eq!(vfs.file_count(), 1);
        assert!(vfs.get_file("handler.js").is_some());
    }
}
