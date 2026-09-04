use compile::binary::extract_vfs;
use compile::vfs::{VFS, RuntimeMode};
use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

/// VFS Provider - in-memory file system from embedded VFS data
pub struct VfsProvider {
    vfs: Arc<VFS>,
    /// Decompressed file cache
    cache: HashMap<String, Vec<u8>>,
}

impl VfsProvider {
    /// Create a new VFS provider from VFS data
    pub fn new(vfs: VFS) -> Self {
        Self {
            vfs: Arc::new(vfs),
            cache: HashMap::new(),
        }
    }

    /// Read a file from VFS (with decompression if needed)
    pub fn read_file(&mut self, path: &str) -> Result<String, String> {
        // Check cache first
        if let Some(cached) = self.cache.get(path) {
            return String::from_utf8(cached.clone())
                .map_err(|_| format!("File is not valid UTF-8: {}", path));
        }

        // Get file from VFS
        let file_entry = self.vfs
            .get_file(path)
            .ok_or_else(|| format!("File not found in VFS: {}", path))?;

        // Decompress if needed
        let content = if file_entry.metadata.compressed {
            let mut decoder = GzDecoder::new(&file_entry.content[..]);
            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|e| format!("Failed to decompress file {}: {}", path, e))?;
            decompressed
        } else {
            file_entry.content.clone()
        };

        // Cache the decompressed content
        self.cache.insert(path.to_string(), content.clone());

        String::from_utf8(content)
            .map_err(|_| format!("File is not valid UTF-8: {}", path))
    }

    /// Get entry point from VFS
    pub fn entry_point(&self) -> &str {
        &self.vfs.entry_point
    }

    /// Get runtime mode from VFS
    pub fn mode(&self) -> &RuntimeMode {
        &self.vfs.mode
    }

    /// Check if file exists in VFS
    #[allow(dead_code)]
    pub fn exists(&self, path: &str) -> bool {
        self.vfs.get_file(path).is_some()
    }

    /// Get all files from VFS (for mounting)
    pub fn get_all_files(&mut self) -> HashMap<String, String> {
        let mut files = HashMap::new();

        for (path, file_entry) in self.vfs.files.iter() {
            // Decompress if needed
            let content = if file_entry.metadata.compressed {
                let mut decoder = GzDecoder::new(&file_entry.content[..]);
                let mut decompressed = Vec::new();
                if decoder.read_to_end(&mut decompressed).is_ok() {
                    decompressed
                } else {
                    continue; // Skip files that fail to decompress
                }
            } else {
                file_entry.content.clone()
            };

            // Convert to string
            if let Ok(content_str) = String::from_utf8(content.clone()) {
                files.insert(path.clone(), content_str);
                // Also cache it
                self.cache.insert(path.clone(), content);
            }
        }

        files
    }
}

/// Detect if the current binary has embedded VFS
pub fn detect_embedded_vfs() -> Option<VfsProvider> {
    // Get current executable path
    let current_exe = std::env::current_exe().ok()?;

    // Try to extract VFS
    if let Ok((vfs_bytes, _metadata)) = extract_vfs(&current_exe) {
        // Deserialize VFS
        if let Ok(vfs) = VFS::from_bytes(&vfs_bytes) {
            return Some(VfsProvider::new(vfs));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use compile::vfs::VFS;

    #[test]
    fn test_vfs_provider() {
        let mut vfs = VFS::new("test.js".to_string(), RuntimeMode::Server);
        vfs.add_file(
            "test.js".to_string(),
            b"console.log('hello')".to_vec(),
            "js".to_string(),
            false,
        );

        let mut provider = VfsProvider::new(vfs);
        assert_eq!(provider.entry_point(), "test.js");
        assert_eq!(provider.mode(), &RuntimeMode::Server);
        assert!(provider.exists("test.js"));
        assert!(!provider.exists("missing.js"));

        let content = provider.read_file("test.js").unwrap();
        assert_eq!(content, "console.log('hello')");
    }
}
