use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Magic bytes to identify VFS data in the binary
const VFS_MAGIC: &[u8; 8] = b"DEKAVFS1";

/// Metadata footer appended to the binary
#[derive(Debug)]
pub struct BinaryMetadata {
    /// Magic identifier
    pub magic: [u8; 8],
    /// Offset where VFS data starts
    pub vfs_offset: u64,
    /// Size of VFS data in bytes
    pub vfs_size: u64,
    /// Entry point file path
    pub entry_point: String,
}

impl BinaryMetadata {
    pub fn new(vfs_offset: u64, vfs_size: u64, entry_point: String) -> Self {
        Self {
            magic: *VFS_MAGIC,
            vfs_offset,
            vfs_size,
            entry_point,
        }
    }

    /// Serialize metadata to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Write magic
        bytes.extend_from_slice(&self.magic);

        // Write VFS offset (8 bytes, little-endian)
        bytes.extend_from_slice(&self.vfs_offset.to_le_bytes());

        // Write VFS size (8 bytes, little-endian)
        bytes.extend_from_slice(&self.vfs_size.to_le_bytes());

        // Write entry point length (4 bytes, little-endian)
        let entry_len = self.entry_point.len() as u32;
        bytes.extend_from_slice(&entry_len.to_le_bytes());

        // Write entry point string
        bytes.extend_from_slice(self.entry_point.as_bytes());

        bytes
    }

    /// Deserialize metadata from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 28 {
            return Err("Metadata too short".to_string());
        }

        // Read magic
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&bytes[0..8]);

        if &magic != VFS_MAGIC {
            return Err("Invalid magic bytes".to_string());
        }

        // Read VFS offset
        let vfs_offset = u64::from_le_bytes(bytes[8..16].try_into().unwrap());

        // Read VFS size
        let vfs_size = u64::from_le_bytes(bytes[16..24].try_into().unwrap());

        // Read entry point length
        let entry_len = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;

        if bytes.len() < 28 + entry_len {
            return Err("Metadata truncated".to_string());
        }

        // Read entry point
        let entry_point = String::from_utf8(bytes[28..28 + entry_len].to_vec())
            .map_err(|_| "Invalid UTF-8 in entry point".to_string())?;

        Ok(Self {
            magic,
            vfs_offset,
            vfs_size,
            entry_point,
        })
    }
}

/// Embeds VFS data into a runtime binary
pub struct BinaryEmbedder {
    /// Path to the source runtime binary
    runtime_binary_path: PathBuf,
}

impl BinaryEmbedder {
    pub fn new(runtime_binary_path: PathBuf) -> Self {
        Self {
            runtime_binary_path,
        }
    }

    /// Embed VFS data into the runtime binary and create output executable
    pub fn embed(
        &self,
        vfs_data: &[u8],
        entry_point: &str,
        output_path: &Path,
    ) -> Result<(), String> {
        // Read the runtime binary
        let runtime_binary = fs::read(&self.runtime_binary_path)
            .map_err(|e| format!("Failed to read runtime binary: {}", e))?;

        // Calculate VFS offset (where VFS data will start)
        let vfs_offset = runtime_binary.len() as u64;
        let vfs_size = vfs_data.len() as u64;

        // Create metadata
        let metadata = BinaryMetadata::new(vfs_offset, vfs_size, entry_point.to_string());
        let metadata_bytes = metadata.to_bytes();

        // Create output file
        let mut output_file = File::create(output_path)
            .map_err(|e| format!("Failed to create output file: {}", e))?;

        // Write runtime binary
        output_file
            .write_all(&runtime_binary)
            .map_err(|e| format!("Failed to write runtime binary: {}", e))?;

        // Write VFS data
        output_file
            .write_all(vfs_data)
            .map_err(|e| format!("Failed to write VFS data: {}", e))?;

        // Write metadata footer
        output_file
            .write_all(&metadata_bytes)
            .map_err(|e| format!("Failed to write metadata: {}", e))?;

        // Set executable permissions (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(output_path)
                .map_err(|e| format!("Failed to read output file metadata: {}", e))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(output_path, perms)
                .map_err(|e| format!("Failed to set executable permissions: {}", e))?;
        }

        Ok(())
    }

    /// Find the runtime binary in the project
    pub fn find_runtime_binary() -> Result<PathBuf, String> {
        // First, try to use the currently running binary
        // This is the CLI binary which contains the runtime
        if let Ok(current_exe) = std::env::current_exe() {
            if current_exe.exists() {
                return Ok(current_exe);
            }
        }

        // Fallback: Try to find the CLI binary relative to current directory
        let possible_paths = vec![
            PathBuf::from("target/release/cli"),
            PathBuf::from("target/debug/cli"),
            PathBuf::from("../target/release/cli"),
            PathBuf::from("../target/debug/cli"),
            PathBuf::from("../../target/release/cli"),
            PathBuf::from("../../target/debug/cli"),
        ];

        for path in possible_paths {
            if path.exists() {
                return Ok(path);
            }
        }

        Err(
            "Could not find runtime binary. Please build the project first with 'cargo build'"
                .to_string(),
        )
    }
}

/// Extract VFS data from a compiled binary
pub fn extract_vfs(binary_path: &Path) -> Result<(Vec<u8>, BinaryMetadata), String> {
    let mut file = File::open(binary_path).map_err(|e| format!("Failed to open binary: {}", e))?;

    // Read the entire file
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|e| format!("Failed to read binary: {}", e))?;

    // Search for magic bytes from the end
    // Metadata is at the very end, so we read backwards
    let min_metadata_size = 28; // magic (8) + offset (8) + size (8) + entry_len (4)

    if contents.len() < min_metadata_size {
        return Err("Binary too small to contain VFS data".to_string());
    }

    // Try to find metadata by searching backwards for magic bytes
    for i in (0..contents.len().saturating_sub(min_metadata_size)).rev() {
        if &contents[i..i + 8] == VFS_MAGIC {
            // Found magic bytes, try to parse metadata
            let metadata_start = i;
            let metadata_bytes = &contents[metadata_start..];

            if let Ok(metadata) = BinaryMetadata::from_bytes(metadata_bytes) {
                // Extract VFS data
                let vfs_start = metadata.vfs_offset as usize;
                let vfs_end = vfs_start + metadata.vfs_size as usize;

                if vfs_end <= contents.len() {
                    let vfs_data = contents[vfs_start..vfs_end].to_vec();
                    return Ok((vfs_data, metadata));
                }
            }
        }
    }

    Err("No VFS data found in binary".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_serialization() {
        let metadata = BinaryMetadata::new(1024, 2048, "handler.js".to_string());
        let bytes = metadata.to_bytes();
        let deserialized = BinaryMetadata::from_bytes(&bytes).unwrap();

        assert_eq!(deserialized.vfs_offset, 1024);
        assert_eq!(deserialized.vfs_size, 2048);
        assert_eq!(deserialized.entry_point, "handler.js");
    }
}
