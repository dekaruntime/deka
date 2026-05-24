#![allow(clippy::all)]

pub mod binary;
pub mod config;
pub mod embed;
pub mod vfs;

pub use config::{DekaConfig, WindowConfig};

use binary::BinaryEmbedder;
use core::Context;
use embed::Embedder;
use std::env;
use std::path::PathBuf;
use stdio as stdio_log;
use vfs::RuntimeMode;

pub fn run(context: &Context) {
    // Check if --desktop and --bundle flags are set
    let is_desktop = context.args.flags.contains_key("--desktop");
    let is_bundle = context.args.flags.contains_key("--bundle");
    let app_name = context.args.params.get("--name").map(|s| s.as_str());

    if is_desktop && is_bundle {
        stdio_log::log("compile", "Compiling as bundled desktop app...");
    } else if is_desktop {
        stdio_log::log("compile", "Compiling as desktop app...");
    } else {
        stdio_log::log("compile", "Single-file executable compilation started...");
    }

    // Get current directory
    let current_dir = env::current_dir().expect("Failed to get current directory");

    // Find entry point (PHP/PHPX only in reboot MVP)
    let entry_point = find_entry_point(&current_dir);

    match entry_point {
        Some(entry) => {
            stdio_log::log("compile", &format!("Found entry point: {}", entry));

            // Build VFS
            stdio_log::log("compile", "Scanning and embedding files...");
            let embedder = Embedder::new(current_dir.clone());

            let mode = if is_desktop {
                RuntimeMode::Desktop
            } else {
                RuntimeMode::Server
            };

            match embedder.build(&entry, mode) {
                Ok(vfs) => {
                    stdio_log::log(
                        "compile",
                        &format!(
                            "Embedded {} files ({} bytes)",
                            vfs.file_count(),
                            vfs.total_size()
                        ),
                    );

                    // Serialize VFS to bytes
                    match vfs.to_bytes() {
                        Ok(vfs_bytes) => {
                            stdio_log::log(
                                "compile",
                                &format!("VFS size: {} bytes", vfs_bytes.len()),
                            );

                            // Find the runtime binary
                            stdio_log::log("compile", "Locating runtime binary...");
                            match BinaryEmbedder::find_runtime_binary() {
                                Ok(runtime_path) => {
                                    stdio_log::log(
                                        "compile",
                                        &format!(
                                            "Found runtime binary: {}",
                                            runtime_path.display()
                                        ),
                                    );

                                    // Determine output path
                                    let output_name = if is_desktop {
                                        "deka-desktop"
                                    } else {
                                        "deka-app"
                                    };
                                    let output_path = current_dir.join(output_name);

                                    // Embed VFS into binary
                                    stdio_log::log("compile", "Embedding VFS into binary...");
                                    let embedder = BinaryEmbedder::new(runtime_path);

                                    match embedder.embed(&vfs_bytes, &entry, &output_path) {
                                        Ok(_) => {
                                            // If bundling, create platform-specific bundle
                                            if is_bundle && is_desktop {
                                                match create_macos_bundle(
                                                    &current_dir,
                                                    &output_path,
                                                    app_name,
                                                ) {
                                                    Ok(bundle_path) => {
                                                        stdio_log::log(
                                                            "compile",
                                                            "✓ Compilation successful!",
                                                        );
                                                        stdio_log::log(
                                                            "compile",
                                                            &format!(
                                                                "  Bundle: {}",
                                                                bundle_path.display()
                                                            ),
                                                        );
                                                        stdio_log::log("compile", "");
                                                        stdio_log::log(
                                                            "compile",
                                                            &format!(
                                                                "Run with: open {}",
                                                                bundle_path.display()
                                                            ),
                                                        );
                                                    }
                                                    Err(e) => {
                                                        stdio_log::error(
                                                            "compile",
                                                            &format!(
                                                                "Failed to create bundle: {}",
                                                                e
                                                            ),
                                                        );
                                                        stdio_log::log(
                                                            "compile",
                                                            "✓ Binary created successfully (without bundle)",
                                                        );
                                                        stdio_log::log(
                                                            "compile",
                                                            &format!(
                                                                "  Output: {}",
                                                                output_path.display()
                                                            ),
                                                        );
                                                    }
                                                }
                                            } else {
                                                stdio_log::log(
                                                    "compile",
                                                    "✓ Compilation successful!",
                                                );
                                                stdio_log::log(
                                                    "compile",
                                                    &format!("  Output: {}", output_path.display()),
                                                );
                                                stdio_log::log(
                                                    "compile",
                                                    &format!(
                                                        "  Size: {} bytes",
                                                        std::fs::metadata(&output_path)
                                                            .map(|m| m.len())
                                                            .unwrap_or(0)
                                                    ),
                                                );
                                                stdio_log::log("compile", "");
                                                stdio_log::log(
                                                    "compile",
                                                    &format!("Run with: ./{}", output_name),
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            stdio_log::error(
                                                "compile",
                                                &format!("Failed to embed binary: {}", e),
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    stdio_log::error(
                                        "compile",
                                        &format!("Failed to find runtime binary: {}", e),
                                    );
                                    stdio_log::error(
                                        "compile",
                                        "Please build the project first with: cargo build",
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            stdio_log::error("compile", &format!("Failed to serialize VFS: {}", e));
                        }
                    }
                }
                Err(e) => {
                    stdio_log::error("compile", &format!("Failed to build VFS: {}", e));
                }
            }
        }
        None => {
            stdio_log::error(
                "compile",
                "No entry point found. Looking for app.php, index.php, or index.phpx",
            );
        }
    }
}

/// Find the entry point file in the current directory
fn find_entry_point(dir: &PathBuf) -> Option<String> {
    let candidates = vec!["app.php", "index.php", "index.phpx"];

    for candidate in candidates {
        let path = dir.join(candidate);
        if path.exists() {
            return Some(candidate.to_string());
        }
    }

    None
}

/// Create a macOS .app bundle
fn create_macos_bundle(
    base_dir: &PathBuf,
    binary_path: &PathBuf,
    app_name: Option<&str>,
) -> Result<PathBuf, String> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    // Determine app name
    let name = app_name.unwrap_or("DekaApp");
    let bundle_name = format!("{}.app", name);
    let bundle_path = base_dir.join(&bundle_name);

    // Create bundle structure
    let contents_dir = bundle_path.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");

    fs::create_dir_all(&macos_dir)
        .map_err(|e| format!("Failed to create MacOS directory: {}", e))?;
    fs::create_dir_all(&resources_dir)
        .map_err(|e| format!("Failed to create Resources directory: {}", e))?;

    // Move binary to MacOS directory
    let binary_name = name.to_lowercase().replace(" ", "-");
    let dest_binary = macos_dir.join(&binary_name);
    fs::copy(binary_path, &dest_binary).map_err(|e| format!("Failed to copy binary: {}", e))?;

    // Make binary executable
    let metadata =
        fs::metadata(&dest_binary).map_err(|e| format!("Failed to get binary metadata: {}", e))?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&dest_binary, permissions)
        .map_err(|e| format!("Failed to set executable permissions: {}", e))?;

    // Remove temporary binary
    let _ = fs::remove_file(binary_path);

    // Create Info.plist
    let plist_content = generate_info_plist(name, &binary_name);
    let plist_path = contents_dir.join("Info.plist");
    fs::write(&plist_path, plist_content)
        .map_err(|e| format!("Failed to write Info.plist: {}", e))?;

    Ok(bundle_path)
}

/// Generate Info.plist content for macOS bundle
fn generate_info_plist(app_name: &str, executable_name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>{}</string>
    <key>CFBundleDisplayName</key>
    <string>{}</string>
    <key>CFBundleIdentifier</key>
    <string>com.deka.{}</string>
    <key>CFBundleVersion</key>
    <string>1.0.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>{}</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.13</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.utilities</string>
</dict>
</plist>
"#,
        app_name, app_name, executable_name, executable_name
    )
}
