use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Virtual File System structure embedded in compiled binaries
#[derive(Debug, Serialize, Deserialize)]
pub struct VFS {
    /// VFS format version
    pub version: u32,
    /// Entry point file path (e.g., "handler.js" or "app.php")
    pub entry_point: String,
    /// Runtime mode (server or desktop)
    pub mode: RuntimeMode,
    /// All files in the VFS
    pub files: HashMap<String, FileEntry>,
}

/// Runtime mode for compiled binaries
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuntimeMode {
    Server,
    Desktop,
}

/// Individual file entry in the VFS
#[derive(Debug, Serialize, Deserialize)]
pub struct FileEntry {
    /// File contents (compressed with gzip)
    pub content: Vec<u8>,
    /// File metadata
    pub metadata: FileMetadata,
}

/// Metadata for a file in the VFS
#[derive(Debug, Serialize, Deserialize)]
pub struct FileMetadata {
    /// Original file size (before compression)
    pub size: u64,
    /// File type (js, ts, php, etc.)
    pub file_type: String,
    /// Whether the file is compressed
    pub compressed: bool,
}

impl VFS {
    /// Create a new empty VFS
    pub fn new(entry_point: String, mode: RuntimeMode) -> Self {
        Self {
            version: 1,
            entry_point,
            mode,
            files: HashMap::new(),
        }
    }

    /// Add a file to the VFS
    pub fn add_file(
        &mut self,
        path: String,
        content: Vec<u8>,
        file_type: String,
        compressed: bool,
    ) {
        let metadata = FileMetadata {
            size: content.len() as u64,
            file_type,
            compressed,
        };

        self.files.insert(path, FileEntry { content, metadata });
    }

    /// Get a file from the VFS
    pub fn get_file(&self, path: &str) -> Option<&FileEntry> {
        self.files.get(path)
    }

    /// Serialize VFS to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|e| format!("Failed to serialize VFS: {}", e))
    }

    /// Deserialize VFS from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(data).map_err(|e| format!("Failed to deserialize VFS: {}", e))
    }

    /// Get total number of files
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Get total size of all files (compressed)
    pub fn total_size(&self) -> u64 {
        self.files.values().map(|f| f.content.len() as u64).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vfs_creation() {
        let mut vfs = VFS::new("handler.js".to_string(), RuntimeMode::Server);
        vfs.add_file(
            "handler.js".to_string(),
            b"export default { fetch() {} }".to_vec(),
            "js".to_string(),
            false,
        );

        assert_eq!(vfs.file_count(), 1);
        assert!(vfs.get_file("handler.js").is_some());
    }

    #[test]
    fn test_vfs_serialization() {
        let mut vfs = VFS::new("handler.js".to_string(), RuntimeMode::Server);
        vfs.add_file(
            "handler.js".to_string(),
            b"console.log('hello')".to_vec(),
            "js".to_string(),
            false,
        );

        let bytes = vfs.to_bytes().unwrap();
        let deserialized = VFS::from_bytes(&bytes).unwrap();

        assert_eq!(deserialized.entry_point, "handler.js");
        assert_eq!(deserialized.file_count(), 1);
    }
}
