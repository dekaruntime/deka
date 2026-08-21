use compile::binary::{BinaryEmbedder, extract_vfs};
use compile::vfs::{RuntimeMode, VFS};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_full_compile_cycle() {
    // Create a temporary directory for our test
    let temp_dir = tempdir().unwrap();

    // Create a simple VFS
    let mut vfs = VFS::new("test.js".to_string(), RuntimeMode::Server);
    vfs.add_file(
        "test.js".to_string(),
        b"console.log('hello world')".to_vec(),
        "js".to_string(),
        false,
    );

    // Serialize VFS
    let vfs_bytes = vfs.to_bytes().unwrap();

    // Find runtime binary
    let runtime_binary =
        BinaryEmbedder::find_runtime_binary().expect("Could not find runtime binary");

    // Create output path
    let output_path = temp_dir.path().join("test-app");

    // Embed VFS
    let embedder = BinaryEmbedder::new(runtime_binary);
    embedder
        .embed(&vfs_bytes, "test.js", &output_path)
        .expect("Failed to embed VFS");

    // Verify output exists
    assert!(output_path.exists());

    // Extract VFS from compiled binary
    let (extracted_vfs_bytes, metadata) = extract_vfs(&output_path).expect("Failed to extract VFS");

    // Verify metadata
    assert_eq!(metadata.entry_point, "test.js");
    assert_eq!(metadata.vfs_size as usize, vfs_bytes.len());

    // Verify VFS data matches
    assert_eq!(extracted_vfs_bytes, vfs_bytes);

    // Deserialize and verify content
    let extracted_vfs = VFS::from_bytes(&extracted_vfs_bytes).unwrap();
    assert_eq!(extracted_vfs.entry_point, "test.js");
    assert_eq!(extracted_vfs.file_count(), 1);

    let test_file = extracted_vfs.get_file("test.js").unwrap();
    assert_eq!(test_file.content, b"console.log('hello world')");
}

#[test]
fn test_binary_permissions() {
    // This test verifies that the compiled binary has execute permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempdir().unwrap();
        let mut vfs = VFS::new("app.js".to_string(), RuntimeMode::Server);
        vfs.add_file(
            "app.js".to_string(),
            b"test".to_vec(),
            "js".to_string(),
            false,
        );
        let vfs_bytes = vfs.to_bytes().unwrap();

        let runtime_binary =
            BinaryEmbedder::find_runtime_binary().expect("Could not find runtime binary");

        let output_path = temp_dir.path().join("test-app");
        let embedder = BinaryEmbedder::new(runtime_binary);
        embedder.embed(&vfs_bytes, "app.js", &output_path).unwrap();

        let metadata = fs::metadata(&output_path).unwrap();
        let permissions = metadata.permissions();
        let mode = permissions.mode();

        // Check that the executable bit is set (0o100 for owner execute)
        assert_ne!(mode & 0o111, 0, "Binary should have execute permissions");
    }
}
